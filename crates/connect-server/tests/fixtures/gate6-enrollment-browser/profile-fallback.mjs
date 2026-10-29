import {BrowserEnrollmentKey,BrowserEnrollmentStore,BrowserTransportClient,BrowserClientOpenError,ConnectError} from '/sdk/browser.js';
const assert=(v,m)=>{if(!v)throw Error(m);};
const decode=s=>Uint8Array.from(atob(s.replaceAll('-','+').replaceAll('_','/')),v=>v.charCodeAt(0));
async function post(path,value){const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(value)});assert(r.ok,'fixture backend response');return r.json();}
globalThis.runBrowserFixture=async cfg=>{
 const pairKey=await crypto.subtle.importKey('raw',decode(cfg.server_public_raw),{name:'Ed25519'},true,['verify']),server={key:pairKey,keyId:cfg.server_keyid,algorithm:'EdDSA',notBefore:cfg.server_not_before,expiresAt:cfg.server_expires_at};
 const vault=new BrowserEnrollmentStore('connect-profile-fallback'),byPath=new Map(),device='device-'+cfg.suffix,now=Math.floor(Date.now()/1000);
 for(const path of ['webtransport','wss-webrtc']){
  const name=cfg.suffix+'-'+path,key=await BrowserEnrollmentKey.generate('EdDSA');
  const request={operation_id:'approve-'+name,completion_operation_id:'claim-'+name,identity_id:cfg.identity_id,device:{id:device,kind:'browser',platform:'owned-Chrome'},key_id:'key-'+name,algorithm:key.algorithm,public_key:key.publicKey,transport:path,endpoint_address:location.origin+'/device/'+name,capabilities:['control','data'],browser_origin:location.origin,operations:['operation.get'],recipients:[cfg.recipient_id],credential_expires_at:now+300,challenge_expires_at:now+120};
  await vault.prepare(name,cfg.api_origin,request,key);const offer=(await post('/approve',await vault.request(name))).result.Ok,session=await vault.acceptOffer(name,offer),credential=await session.claim();byPath.set(path,{name,session,credential});
 }
 const pins=[{algorithm:'sha-256',value:decode(cfg.certificate_sha256)}],resolutions=[];
 const options=(path)=>{const p=byPath.get(path);return p.session.clientOptions(p.credential,{url:path==='webtransport'?cfg.wt_url:cfg.wss_url,server,...(path==='webtransport'?{serverCertificateHashes:pins}:{})});};
 const profiles={webtransport:{name:byPath.get('webtransport').name,contact:{url:cfg.wt_url,server,serverCertificateHashes:pins}},'wss-webrtc':{name:byPath.get('wss-webrtc').name,contact:{url:cfg.wss_url,server}}};
 const savedProfiles=vault.resolveProfiles(profiles);profiles.webtransport.name='changed-after-capture';profiles.webtransport.contact.url='https://other.fixture.invalid/changed';
 const configuration=(wt,resolve)=>({identityId:cfg.identity_id,deviceId:device,endpoints:{webtransport:wt,'wss-webrtc':cfg.wss_url},timeoutMs:5000,attemptTimeoutMs:350,resolveEnrollment:async context=>{resolutions.push([context.transport,context.reason,context.physicalEpoch]);return resolve(context);}});
 const direct=await BrowserTransportClient.connect(configuration(cfg.wt_url,savedProfiles));assert(direct.status.selectedTransport==='webtransport','approved WT direct');direct.close();assert(resolutions.length===1&&resolutions[0][0]==='webtransport','one direct resolver');resolutions.length=0;
 const fallback=await BrowserTransportClient.connect(configuration(cfg.unreachable_wt_url,savedProfiles));
 assert(fallback.status.selectedTransport==='wss-webrtc'&&fallback.status.fallbackReason==='preferred-transport-timeout','bounded WT outage selects WSS');assert(JSON.stringify(resolutions)===JSON.stringify([['webtransport','initial',1],['wss-webrtc','initial',1]]),'separate profile resolution in same round');
 await fallback.reconnect({preferredTransport:'wss-webrtc'});assert(fallback.status.physicalEpoch===2&&resolutions[2][0]==='wss-webrtc'&&resolutions[2][1]==='reconnect','explicit reconnect resolves current selected profile');fallback.close();resolutions.length=0;
 let reused;try{await BrowserTransportClient.connect(configuration(cfg.unreachable_wt_url,()=>options('webtransport')));}catch(error){reused=error;}
 assert(reused instanceof BrowserClientOpenError&&reused.phase==='authentication','reusing preferred WT credential cannot authenticate WSS fallback');assert(resolutions.length===2,'wrong reused credential cannot restart the round');resolutions.length=0;
 let refused;try{await BrowserTransportClient.connect(configuration(cfg.wt_url,()=>({...options('wss-webrtc'),serverCertificateHashes:pins})));}catch(error){refused=error;}
 assert(refused instanceof BrowserClientOpenError&&refused.phase==='authentication','wrong profile must fail actual WT authentication');assert(resolutions.length===1&&resolutions[0][0]==='webtransport','authentication refusal cannot fall back');resolutions.length=0;
 const retired=byPath.get('wss-webrtc');await post('/revoke',{enrollment_id:retired.session.offer.enrollment_id,operation_id:'revoke-wss-'+cfg.suffix});
 try{await BrowserTransportClient.connect({...configuration(cfg.wt_url,savedProfiles),preferredTransport:'wss-webrtc'});throw Error('revoked profile admitted');}catch(error){assert(error instanceof ConnectError&&!(error instanceof BrowserClientOpenError)&&error.code==='forbidden','revoked saved profile refuses at original claim before physical opening');}
 assert(resolutions.length===1&&resolutions[0][0]==='wss-webrtc','revocation cannot trigger another credential or transport');
 resolutions.length=0;
 try{await BrowserTransportClient.connect({...configuration(cfg.wt_url,()=>options('wss-webrtc')),preferredTransport:'wss-webrtc'});throw Error('revoked cached credential admitted');}catch(error){assert(error instanceof BrowserClientOpenError&&error.phase==='authentication','revoked cached credential refuses actual peer authentication');}
 assert(resolutions.length===1&&resolutions[0][0]==='wss-webrtc','revoked cached peer cannot trigger fallback');
 const wt=byPath.get('webtransport');await post('/revoke',{enrollment_id:wt.session.offer.enrollment_id,operation_id:'revoke-wt-'+cfg.suffix});
 return {passed:true,direct_wt:1,bounded_wss_fallback:1,explicit_wss_reconnect:1,reused_preferred_credential_refusal:1,wrong_profile_authentication_refusal:1,revoked_profile_refusal:1,revoked_cached_peer_refusal:1,credentials:2,device:1,endpoints:2,mutation_commands:0};
};
