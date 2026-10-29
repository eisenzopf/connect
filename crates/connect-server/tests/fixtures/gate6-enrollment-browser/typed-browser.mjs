import {BrowserClient,BrowserEnrollmentKey,BrowserEnrollmentStore,BrowserSdkClient,browserOperationKinds,sdkId} from '/sdk/browser.js';
const trace=[];
const originalRequest=BrowserClient.prototype.request,originalEvents=BrowserClient.prototype.onEvent;
BrowserClient.prototype.request=async function(...args){const frames=await originalRequest.apply(this,args);trace.push({request:args[0],operation:args[2]?.operation_id});return frames;};
BrowserClient.prototype.onEvent=function(callback){return originalEvents.call(this,frame=>{trace.push({event:frame.type});return callback(frame);});};
const assert=(v,m)=>{if(!v)throw Error(m);};
const decode=s=>Uint8Array.from(atob(s.replaceAll('-','+').replaceAll('_','/')),v=>v.charCodeAt(0));
async function post(path,value){const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(value)});assert(r.ok,'fixture backend response');return r.json();}
globalThis.runBrowserFixture=async cfg=>{try{
 const publicKey=await crypto.subtle.importKey('raw',decode(cfg.server_public_raw),{name:'Ed25519'},true,['verify']),server={key:publicKey,keyId:cfg.server_keyid,algorithm:'EdDSA',notBefore:cfg.server_not_before,expiresAt:cfg.server_expires_at};
 const vault=new BrowserEnrollmentStore('connect-typed-browser'),identities=[cfg.identity_id,cfg.recipient_id],profiles=[],now=Math.floor(Date.now()/1000),pins=[{algorithm:'sha-256',value:decode(cfg.certificate_sha256)}];
 const operations=['conversation.get','create','operation.get','session.accept','session.cancel','session.end','session.invite','session.reject','session_get'];
 for(let actor=0;actor<2;actor++){
  const byPath={},device='device-'+cfg.suffix+'-'+actor;
  for(const transport of ['webtransport','wss-webrtc']){
   const name=cfg.suffix+'-'+actor+'-'+transport,key=await BrowserEnrollmentKey.generate('EdDSA');
   await vault.prepare(name,cfg.api_origin,{operation_id:'approve-'+name,completion_operation_id:'claim-'+name,identity_id:identities[actor],device:{id:device,kind:'browser',platform:'owned-Chrome'},key_id:'key-'+name,algorithm:key.algorithm,public_key:key.publicKey,transport,endpoint_address:location.origin+'/device/'+name,capabilities:['control','data'],browser_origin:location.origin,operations,recipients:identities,credential_expires_at:now+300,challenge_expires_at:now+120},key);
   const offer=(await post('/typed-approve',await vault.request(name))).result.Ok,session=await vault.acceptOffer(name,offer);await session.claim();
   byPath[transport]={name,contact:{url:transport==='webtransport'?cfg.wt_url:cfg.wss_url,server,...(transport==='webtransport'?{serverCertificateHashes:pins}:{})}};
  }
  profiles.push({device,byPath});
 }
 const savedCancellations=new Map();
 const result={passed:true,direct_wt:0,bounded_wss_fallback:0,incoming_rejections:0,incoming_accepts:0,explicit_retry:0,cancellations:0,recovered_original:0,current_authority_refusal:0};
 for(const path of ['webtransport','wss-webrtc']){
  const resolutions=[[],[]],clients=[];
  try{
   for(let actor=0;actor<2;actor++){
    const resolve=vault.resolveProfiles(profiles[actor].byPath);
    clients.push(await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:identities[actor],deviceId:profiles[actor].device,endpoints:{webtransport:path==='webtransport'?cfg.wt_url:cfg.unreachable_wt_url,'wss-webrtc':cfg.wss_url},timeoutMs:5000,attemptTimeoutMs:350,resolveEnrollment:context=>{resolutions[actor].push(context.transport);return resolve(context);}}));
   }
   const [caller,callee]=clients,options={timeoutMs:5000},suffix=cfg.suffix+'-'+path,mutation=name=>({...options,operationId:sdkId(name+'-'+suffix)});
   assert(clients.every(c=>c.transport.selectedTransport===path),'same typed workflow selected path');
   assert(resolutions.every(r=>JSON.stringify(r)===JSON.stringify(path==='webtransport'?['webtransport']:['webtransport','wss-webrtc'])),'transport-bound profile resolved per contact');
   result[path==='webtransport'?'direct_wt':'bounded_wss_fallback']++;
   // Persist logical IDs/references before IO; restoring observes their original record.
   const createId=mutation('room');sessionStorage.setItem('create-'+suffix,JSON.stringify(createId.operationId));
   const create=await caller.conversations.create({policy:'persistent',participants:[{identityId:sdkId(identities[1]),role:'participant'}]},createId);
   sessionStorage.setItem('create-reference-'+suffix,JSON.stringify(create.referenceJson()));
   const room=(await caller.operations.restore(JSON.parse(sessionStorage.getItem('create-reference-'+suffix)),browserOperationKinds.conversationCreate).wait(options)).value;
   const target=await room.participants.byIdentity(sdkId(identities[1]),options),incoming=callee.incomingInvitations(options);
   try{
    for(const action of ['reject','accept']){
     const sessionId=sdkId('session-'+action+'-'+suffix),invitationId=mutation('invite-'+action);
     sessionStorage.setItem('invite-id-'+suffix,JSON.stringify({sessionId,operationId:invitationId.operationId}));
     const invite=await room.sessions.invite({sessionId,to:[target.id],medium:'text-chat',intent:'synthetic',capabilities:{data:['reliable-ordered']}},invitationId);
     const pending=(await invite.wait(options)).value;
     assert(pending.id===sessionId,'invitation original operation completed');
     const hint=(await incoming.next()).value;
     assert(hint.session.id===pending.id&&hint.session.conversationId===room.id,'incoming scoped handle without copied IDs');
     assert(hint.operationId!==invitationId.operationId,'server source distinct from mutation intent');
     const current=await hint.session.refresh(options);
     const decision=action==='reject'?await hint.session.reject({reasonCode:486,reason:'Unavailable',expectedTopologyRevision:current.topologyRevision},mutation('reject')):await hint.session.accept({capabilities:hint.capabilities,expectedTopologyRevision:current.topologyRevision},mutation('accept'));
     const completed=await decision.wait(options);
     if(action==='reject'){
      assert(completed.value.state==='rejected'&&(await hint.session.refresh(options)).state==='rejected','current rejected state');result.incoming_rejections++;
     }else{
      assert(completed.value.id===pending.id,'accept typed result');result.incoming_accepts++;result.explicit_retry++;
      const latest=await pending.refresh(options),end=await pending.end({reasonCode:200,reason:'Finished',expectedTopologyRevision:latest.topologyRevision},mutation('end'));
      const saved=end.referenceJson();sessionStorage.setItem('end-reference-'+suffix,JSON.stringify(saved));await end.wait(options);
      await caller.reconnect({preferredTransport:path});
      const restored=caller.operations.restore(JSON.parse(sessionStorage.getItem('end-reference-'+suffix)),browserOperationKinds.sessionEnd),ended=await restored.wait(options);
      assert(ended.value.receiptSnapshot.state==='ended'&&(await ended.value.refresh(options)).state==='ended','original receipt survives physical replacement');result.recovered_original++;
     }
    }
    const cancelInvite=await room.sessions.invite({sessionId:sdkId('session-cancel-'+suffix),to:[target.id],medium:'text-chat',intent:'synthetic',capabilities:{data:['reliable-ordered']}},mutation('invite-cancel'));
    const invited=(await cancelInvite.wait(options)).value,received=(await incoming.next()).value;
    assert(received.session.id===invited.id,'cancellation recipient scope');
    const live=await invited.refresh(options),cancel=await invited.cancelInvitation({reasonCode:487,reason:'Cancelled',expectedTopologyRevision:live.topologyRevision},mutation('cancel'));
    assert((await cancel.wait(options)).value.state==='cancelled','typed cancellation');result.cancellations++;
    savedCancellations.set(path,cancel.referenceJson());
   }finally{await incoming.close();}
  }finally{for(const client of clients)client.close();}
 }
 // Both path workflows finish before retiring WT: a refused preferred profile
 // deliberately cannot fall back to a different transport credential.
 for(const path of ['webtransport','wss-webrtc']){
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:identities[0],deviceId:profiles[0].device,endpoints:{webtransport:cfg.wt_url,'wss-webrtc':cfg.wss_url},preferredTransport:path,timeoutMs:5000,attemptTimeoutMs:350,resolveEnrollment:vault.resolveProfiles(profiles[0].byPath)});
  try{
   const reference=savedCancellations.get(path),original=client.operations.restore(reference,browserOperationKinds.sessionCancelInvitation),options={timeoutMs:5000};
   await original.status(options);
   const profile=await vault.load(profiles[0].byPath[path].name);
   await post('/revoke',{enrollment_id:profile.offer.enrollment_id,operation_id:'revoke-'+cfg.suffix+'-'+path});
   let refused;try{await original.status(options);}catch(error){refused=error;}
   assert(refused&&['authentication','transport-uncertain','cancelled'].includes(refused.kind),'current revoked authority refuses original receipt');
   let originalClaim;try{await client.reconnect({preferredTransport:path});}catch(error){originalClaim=error;}
   assert(originalClaim?.kind==='authentication'&&originalClaim?.certainty==='refused','revoked original claim cannot restore a new physical');result.current_authority_refusal++;
  }finally{client.close();}
 }
 const mutations=trace.filter(x=>x.operation&&x.request!=='operation.get');
 assert(mutations.length===16&&new Set(mutations.map(x=>x.operation)).size===16,'eight original mutations per path, no status/restore resubmission');
 return {...result,credentials:4,devices:2,endpoints:4,original_mutations:mutations.length};
}catch(error){throw Error(String(error)+"\n"+JSON.stringify(trace));}};
