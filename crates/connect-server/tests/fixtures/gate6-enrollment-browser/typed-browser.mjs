import {BrowserRtcConnection} from '/sdk/browser_sdk_webrtc.js';
import {BrowserClient,BrowserEnrollmentKey,BrowserEnrollmentStore,BrowserSdkClient,browserOperationKinds,sdkId,browserCursor,BrowserSourcePicker,BrowserCaptureControls,BrowserOperationProgress,browserInputDevices} from '/sdk/browser.js';
const trace=[],rtcOwners=[];
const nonempty=items=>({first:items[0],rest:items.slice(1)}),offered=items=>nonempty(items.map(v=>({...v,codecs:nonempty(v.codecs)})));
const originalRtcBegin=BrowserRtcConnection.begin;
BrowserRtcConnection.begin=function(...args){const begun=originalRtcBegin.apply(this,args);rtcOwners.push(begun.owner);return begun;};
const rtcObservations=async()=>Promise.all(rtcOwners.map(async owner=>{
 const peer=owner.peer;const stats=await peer.getStats().catch(()=>new Map());
 return {stopped:owner.stopped,connection:peer.connectionState,ice:peer.iceConnectionState,tracks:peer.getTransceivers().map(t=>({sent:t.sender.track?.kind,sender:t.sender.track?.readyState,enabled:t.sender.track?.enabled,received:t.receiver.track.readyState})),rtp:[...stats.values()].filter(v=>['inbound-rtp','outbound-rtp'].includes(v.type)).map(v=>({type:v.type,kind:v.kind,packetsSent:v.packetsSent,packetsReceived:v.packetsReceived,framesEncoded:v.framesEncoded,framesDecoded:v.framesDecoded,bytesSent:v.bytesSent}))};
}));
const originalBegin=BrowserClient.prototype.begin,originalEvents=BrowserClient.prototype.onEvent,originalRequest=BrowserClient.prototype.request;
let deferNextSnapshot=false,heldSnapshot;
BrowserClient.prototype.request=function(...args){const work=originalRequest.apply(this,args);if(args[0]!=='conversation.get'||!deferNextSnapshot)return work;deferNextSnapshot=false;return work.then(frames=>new Promise(resolve=>{heldSnapshot=()=>resolve(frames);}));};
BrowserClient.prototype.begin=function(...args){const begun=originalBegin.apply(this,args);trace.push({request:args[0],operation:args[2]?.operation_id});return begun;};
BrowserClient.prototype.onEvent=function(callback){return originalEvents.call(this,frame=>{trace.push({event:frame.type});return callback(frame);});};
const within=async(work,label)=>{let timer;try{return await Promise.race([work,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(label+' deadline')),5000);})]);}finally{clearTimeout(timer);}};
const assert=(v,m)=>{if(!v)throw Error(m);};
const decode=s=>Uint8Array.from(atob(s.replaceAll('-','+').replaceAll('_','/')),v=>v.charCodeAt(0));
async function post(path,value){const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(value)});assert(r.ok,'fixture backend response');return r.json();}
globalThis.runBrowserFixture=async cfg=>{try{globalThis.typedUiComplete=false;
 const publicKey=await crypto.subtle.importKey('raw',decode(cfg.server_public_raw),{name:'Ed25519'},true,['verify']),server={key:publicKey,keyId:cfg.server_keyid,algorithm:'EdDSA',notBefore:cfg.server_not_before,expiresAt:cfg.server_expires_at};
 const vault=new BrowserEnrollmentStore('connect-typed-browser'),identities=[cfg.identity_id,cfg.recipient_id],profiles=[],now=Math.floor(Date.now()/1000),pins=[{algorithm:'sha-256',value:decode(cfg.certificate_sha256)}];
 const operations=['connection.end','connection.offer','connection.update','conversation.get','conversation.participant.add','conversation.participant.remove','conversation.participant.update','conversation.subscribe','create','history','operation.get','send','session.accept','session.cancel','session.end','session.invite','session.reject','session.update','session_get','stream.subscribe','stream.unsubscribe'];
 for(let actor=0;actor<2;actor++){
  const byPath={},device='device-'+cfg.suffix+'-'+actor;
  for(const transport of ['webtransport','wss-webrtc']){
   const name=cfg.suffix+'-'+actor+'-'+transport,key=await BrowserEnrollmentKey.generate('EdDSA');
   await vault.prepare(name,cfg.api_origin,{operation_id:'approve-'+name,completion_operation_id:'claim-'+name,identity_id:identities[actor],device:{id:device,kind:'browser',platform:'owned-Chrome'},key_id:'key-'+name,algorithm:key.algorithm,public_key:key.publicKey,transport,endpoint_address:location.origin+'/device/'+name,capabilities:['control','data','audio','video','screen'],browser_origin:location.origin,operations,recipients:identities,credential_expires_at:now+300,challenge_expires_at:now+120},key);
   const offer=(await post('/typed-approve',await vault.request(name))).result.Ok,session=await vault.acceptOffer(name,offer);await session.claim();
   // Explicit trusted fixture setup: key enrollment does not assert Endpoint assurance.
   await post('/typed-verify',{endpoint_id:offer.endpoint_id,operation_id:'verify-'+name});
   byPath[transport]={name,contact:{url:transport==='webtransport'?cfg.wt_url:cfg.wss_url,server,...(transport==='webtransport'?{serverCertificateHashes:pins}:{})}};
  }
  profiles.push({device,byPath});
 }
 const waitUntil=async(test,label)=>{const until=Date.now()+10000;while(!test()){if(Date.now()>until)throw Error(label+' deadline');await new Promise(r=>setTimeout(r,15));}};
 const ui=async phase=>{globalThis.typedUiPhase=phase;await waitUntil(()=>globalThis.typedUiAcknowledged===phase,'owned UI '+phase);};
 const captured=[];const originalCapture=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
 navigator.mediaDevices.getUserMedia=async input=>{const stream=await originalCapture(input);captured.push(stream);return stream;};
 const displays=[];const originalDisplay=navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
 navigator.mediaDevices.getDisplayMedia=async input=>{const stream=await originalDisplay(input);if(stream.getVideoTracks().some(t=>t.getSettings().displaySurface!=='browser')){for(const track of stream.getTracks())track.stop();throw Error('owned tab required');}displays.push(stream);return stream;};
 const savedCancellations=new Map();let savedNative;
 const result={passed:true,direct_wt:0,bounded_wss_fallback:0,incoming_rejections:0,incoming_accepts:0,explicit_retry:0,cancellations:0,recovered_original:0,current_authority_refusal:0,wt_native_connection:0,wt_owned_capture_retired:0,wt_owned_renderer_retired:0,restored_media_observation_only:0,wss_media_connection:0,wss_owned_capture_retired:0,wss_owned_renderer_retired:0,owned_device_permission:0,permission_denial_ui:0,keyboard_capture_retry:0,device_source_choice:0,operation_progress_ui:0,wt_screen_connection:0,wss_screen_connection:0,owned_tab_capture:0,owned_tab_capture_retired:0,screen_kind_observed:0,screen_recovery_without_custody:0,screen_duplex_decoded:0,screen_controls:0,screen_resubscription:0,restored_control_observations:0,screen_camera_selector_refused:0,screen_stale_gate_refused:0,screen_conflict_original_recovered:0,session_updates:0,session_reinvitations:0,restored_session_updates:0,invitation_read_recovered:0,queued_cancellation_refused:0,message_acceptances:0,journal_received:0,journal_resumed:0,message_recovered_original:0,membership_adds:0,membership_updates:0,membership_removals:0,membership_rejoins:0,membership_refusals:0,membership_old_cursor_refusals:0,membership_old_receipt_refusals:0,membership_recovered:0};
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
   const create=await caller.conversations.create({policy:'persistent',participants:[sdkId(identities[1])]},createId);
   sessionStorage.setItem('create-reference-'+suffix,JSON.stringify(create.referenceJson()));
   const room=(await caller.operations.restore(JSON.parse(sessionStorage.getItem('create-reference-'+suffix)),browserOperationKinds.conversationCreate).wait(options)).value;
   const target=await room.participants.byIdentity(sdkId(identities[1]),options);let incoming=callee.incomingInvitations(options);
   {const other=callee.conversations.byId(room.id),source=await room.participants.byIdentity(sdkId(identities[0]),options),outgoing=room.events({},options),received=other.events({},options);try{
    await outgoing.ready();await received.ready();
    const observe=async(reader,messageId,body)=>within((async()=>{for(let i=0;i<16;i++){const event=(await reader.next()).value;if(event.data?.kind==='message'&&event.data.data.id===messageId){assert(event.conversationId===room.id&&event.data.data.body===body,'typed current journal identifies exact private message');result.journal_received++;return event;}}throw Error('expected typed message fact');})(),'typed private message journal');
    const input={messageId:sdkId('typed-message-caller-'+suffix),to:nonempty([target.id]),text:'owned caller '+path},key=mutation('typed-message-caller');sessionStorage.setItem('typed-message-caller-intent-'+suffix,JSON.stringify({input,mutation:key}));const sent=await room.send(input,key);sessionStorage.setItem('typed-message-caller-reference-'+suffix,JSON.stringify(sent.referenceJson()));const accepted=(await sent.wait(options)).value;assert(accepted.messageId===input.messageId&&accepted.boundary==='local-mutation'&&accepted.operation.operationId===key.operationId,'typed acceptance remains local mutation, not delivery');result.message_acceptances++;const first=await observe(received,input.messageId,input.text);sessionStorage.setItem('typed-journal-cursor-'+suffix,first.cursor);await received.close();
    const replyInput={messageId:sdkId('typed-message-callee-'+suffix),to:nonempty([source.id]),text:'owned callee '+path},replyKey=mutation('typed-message-callee');sessionStorage.setItem('typed-message-callee-intent-'+suffix,JSON.stringify({input:replyInput,mutation:replyKey}));const reply=await other.send(replyInput,replyKey);assert((await reply.wait(options)).value.messageId===replyInput.messageId,'counterpart typed acceptance');result.message_acceptances++;await observe(outgoing,replyInput.messageId,replyInput.text);
    const resumed=other.events({after:browserCursor(sessionStorage.getItem('typed-journal-cursor-'+suffix))},options);try{const value=await observe(resumed,replyInput.messageId,replyInput.text);assert(value.cursor!==first.cursor,'explicit saved journal cursor resumes later fact');result.journal_resumed++;}finally{await resumed.close();}
   }finally{await outgoing.close();await received.close();}}

   // A separate Conversation exercises membership epochs without changing the
   // Session/media fixture's original authority or historical receipts.
   {const createInput={policy:'persistent',participants:[]},createKey=mutation('member-room');sessionStorage.setItem('member-room-intent-'+suffix,JSON.stringify({input:createInput,mutation:createKey}));const opened=await caller.conversations.create(createInput,createKey),members=(await opened.wait(options)).value,kind=(await target.refresh(options)).kind;
    const change=async(name,input,submit)=>{const key=mutation(name);sessionStorage.setItem(name+'-intent-'+suffix,JSON.stringify({input,mutation:key}));const operation=await submit(input,key);sessionStorage.setItem(name+'-reference-'+suffix,JSON.stringify(operation.referenceJson()));return {operation,value:(await operation.wait(options)).value};};
    const first=await change('member-add',{identityId:sdkId(identities[1]),kind,role:'reviewer',expectedRevision:(await members.refresh(options)).revision},members.participants.add);assert(first.value.receiptSnapshot.active&&first.value.receiptSnapshot.role==='reviewer','typed explicit Participant admission');result.membership_adds++;
    const currentPeer=callee.conversations.byId(members.id),originalEpoch=first.value.receiptSnapshot.membershipEpoch,journal=currentPeer.events({},options);let cursor;try{cursor=await journal.ready();}finally{await journal.close();}
    const updated=await change('member-update',{role:'observer',expectedConversationRevision:(await members.refresh(options)).revision},first.value.update.bind(first.value));assert(updated.value.id===first.value.id&&updated.value.receiptSnapshot.role==='observer'&&(await first.value.refresh(options)).role==='observer'&&first.value.receiptSnapshot.role==='reviewer','current role and immutable prior receipt view remain distinct');result.membership_updates++;
    const restored=(await caller.operations.restore(JSON.parse(sessionStorage.getItem('member-update-reference-'+suffix)),browserOperationKinds.participantUpdate).wait(options)).value;assert(restored.id===first.value.id,'original Participant update recovery does not resubmit');result.membership_recovered++;
    const removed=await change('member-remove',{expectedConversationRevision:(await members.refresh(options)).revision},first.value.remove.bind(first.value));assert(removed.value.id===first.value.id&&!removed.value.receiptSnapshot.active,'typed Participant removal');result.membership_removals++;
    let refusal;try{await currentPeer.refresh(options);}catch(error){refusal=error;}assert(refusal?.kind==='authentication','removed membership cannot read the Conversation');result.membership_refusals++;
    const rejoined=await change('member-rejoin',{identityId:sdkId(identities[1]),kind,role:'participant',expectedRevision:(await members.refresh(options)).revision},members.participants.add);assert(rejoined.value.id===first.value.id&&rejoined.value.receiptSnapshot.active&&rejoined.value.receiptSnapshot.membershipEpoch===originalEpoch+1,'rejoin preserves stable Participant ID with successor membership epoch');assert((await currentPeer.participants.byId(first.value.id).refresh(options)).membershipEpoch===originalEpoch+1,'successor membership is freshly authorized');result.membership_rejoins++;
    const stale=currentPeer.events({after:cursor},options);try{let denied;try{await stale.ready();}catch(error){denied=error;}assert(denied?.kind==='authentication','original journal cursor cannot cross membership epochs');result.membership_old_cursor_refusals++;}finally{await stale.close();}
    let oldReceipt;try{await caller.operations.restore(JSON.parse(sessionStorage.getItem('member-update-reference-'+suffix)),browserOperationKinds.participantUpdate).wait(options);}catch(error){oldReceipt=error;}assert(oldReceipt?.kind==='authentication','original membership receipt cannot borrow the successor target epoch');result.membership_old_receipt_refusals++;
   }

   try{
    for(const action of ['reject','accept']){
     const sessionId=sdkId('session-'+action+'-'+suffix),invitationId=mutation('invite-'+action);
     sessionStorage.setItem('invite-id-'+suffix,JSON.stringify({sessionId,operationId:invitationId.operationId}));
     const invite=await room.sessions.invite({sessionId,to:[target.id],medium:action==='accept'?'voice+video':'mixed',intent:'synthetic',capabilities:{audio:['opus'],video:['vp8'],screen:['vp8'],data:['reliable-ordered']}},invitationId);
     const pending=(await invite.wait(options)).value;
     assert(pending.id===sessionId,'invitation original operation completed');
     let recoveredHint;if(action==='accept'){
      await incoming.close();const abort=new AbortController(),abandoned=callee.incomingInvitations({...options,signal:abort.signal}),epoch=callee.transport.physicalEpoch;heldSnapshot=undefined;deferNextSnapshot=true;const before=trace.filter(v=>v.request==='conversation.get').length,observation=abandoned.next();await waitUntil(()=>typeof heldSnapshot==='function','original actual authorized snapshot held');abort.abort();let cancelled;try{await observation;}catch(error){cancelled=error;}assert(cancelled?.kind==='cancelled'&&callee.transport.state==='connected'&&callee.transport.physicalEpoch===epoch,'local wait cancellation preserves original physical owner');await abandoned.close();const release=heldSnapshot;heldSnapshot=undefined;release();incoming=callee.incomingInvitations(options);const recovered=(await incoming.next()).value;assert(recovered.session.id===pending.id&&trace.filter(v=>v.request==='conversation.get').length===before+2,'retained invitation requires a fresh actual authorized snapshot');recoveredHint=recovered;result.invitation_read_recovered++;
     }
     const hint=recoveredHint??(await incoming.next()).value;
     assert(hint.session.id===pending.id&&hint.session.conversationId===room.id,'incoming scoped handle without copied IDs');
     assert(hint.operationId!==invitationId.operationId,'server source distinct from mutation intent');
     const current=await hint.session.refresh(options);
     const decision=action==='reject'?await hint.session.reject({reasonCode:486,reason:'Unavailable',expectedTopologyRevision:current.topologyRevision},mutation('reject')):await hint.session.accept({capabilities:hint.capabilities,expectedTopologyRevision:current.topologyRevision},mutation('accept'));
     const completed=await decision.wait(options);
     if(action==='reject'){
      assert(completed.value.state==='rejected'&&(await hint.session.refresh(options)).state==='rejected','current rejected state');result.incoming_rejections++;
     }else{
      assert(completed.value.id===pending.id,'accept typed result');result.incoming_accepts++;result.explicit_retry++;
      {
       const cameraBefore=captured.length,cycle=path==='webtransport'?0:1,picker=new BrowserSourcePicker({audio:true,video:['camera']});document.body.append(picker.element);
       await ui('grant-'+cycle);await picker.refresh(options);
       const devices=await browserInputDevices(options);assert(devices.microphones.length>0&&devices.cameras.length>0&&devices.microphones.every(v=>/fake/i.test(v.label))&&devices.cameras.every(v=>/fake/i.test(v.label)),'only owned Chrome synthetic devices');
       const selectors=picker.element.querySelectorAll('select');for(const select of selectors)select.addEventListener('keydown',event=>trace.push({source_key:event.key,trusted:event.isTrusted}));selectors[0].focus();await ui('microphone-'+cycle);selectors[2].focus();await ui('camera-'+cycle);
       const selection=picker.selection();trace.push({source_selection:[...selectors].map(v=>({index:v.selectedIndex,empty:!v.value,disabled:v.disabled,focus:v===document.activeElement}))});assert(selection.audio&&selection.video==='camera'&&selection.audioDeviceId&&selection.videoDeviceId,'keyboard selects explicit local sources');result.device_source_choice++;
       await ui('deny-'+cycle);
       const savedOffer={connectionId:sdkId('native-'+suffix),streams:offered([{id:sdkId('audio-'+suffix),kind:'audio',direction:'sendrecv',codecs:['opus']},{id:sdkId('video-'+suffix),kind:'video',direction:'sendrecv',codecs:['vp8']}])},savedMutation={...mutation('native-offer'),timeoutMs:25000};
       sessionStorage.setItem('media-intent-'+suffix,JSON.stringify({input:savedOffer,mutation:savedMutation,selection}));
       const native=await pending.connections.offer(savedOffer,savedMutation);
       savedNative=native.referenceJson();sessionStorage.setItem('media-reference-'+suffix,JSON.stringify(savedNative));
       const connection=pending.connections.byId(sdkId('native-'+suffix)),controls=new BrowserCaptureControls(connection,selection,{timeoutMs:5000}),progress=new BrowserOperationProgress(native,{timeoutMs:10000});document.body.append(controls.element,progress.element);
       let capture,renderer;try{
        let trustedClicks=0;const start=controls.element.querySelector('button');start.addEventListener('click',event=>{if(event.isTrusted)trustedClicks++;trace.push({capture_click:cycle,trusted:event.isTrusted});});start.focus();await ui('capture-'+cycle);await waitUntil(()=>controls.state==='failed','permission denied state');
        const message=controls.element.querySelector('[role="status"]');assert(message.getAttribute('aria-live')==='polite'&&/permission was denied/.test(message.textContent)&&!start.disabled,'accessible denied permission state');result.permission_denial_ui++;
        await ui('grant-'+cycle);start.focus();
        await ui('capture-'+cycle);await waitUntil(()=>controls.state==='ready','actual owned permission capture');capture=await controls.start();assert(trustedClicks===2,'two trusted keyboard capture decisions');assert(!capture.closed&&captured.length===cameraBefore+1,'one granted synthetic capture per path');result.owned_device_permission++;result.keyboard_capture_retry++;
        progress.element.querySelector('button').focus();await ui('progress-'+cycle);const completedConnection=(await progress.observe()).value;assert(progress.state==='ready'&&/completed/.test(progress.element.querySelector('[role="status"]').textContent),'accessible original operation completion');result.operation_progress_ui++;
        const observed=await completedConnection.refresh(options);assert(observed.id===connection.id&&observed.generation===1&&observed.streamRevision===completedConnection.receiptStreamRevision,'current Connection resource observation');
        const sources=await completedConnection.streams.list(options);assert(sources.length===2,'typed Stream handles');
        for(const source of sources){const view=await source.refresh(options);assert(view.ownership==='observation'&&view.connectionId===connection.id,'observations grant no capture');}
        result[path==='webtransport'?'wt_native_connection':'wss_media_connection']++;
        let duplicateCapture;try{await connection.capture({audio:true},{timeoutMs:1000});}catch(error){duplicateCapture=error;}
        assert(duplicateCapture?.kind==='conflict'&&!capture.closed,'duplicate capture cannot replace an active source');
        const videoSource=sources.find(v=>v.id===sdkId('video-'+suffix)),output=document.createElement('canvas');renderer=await connection.render({audio:false,video:[{stream:videoSource,canvas:output}]},options);
        assert(output.width===640&&output.height===360,'owned renderer configures its scoped canvas');
        let duplicateRender;try{await connection.render({audio:false},options);}catch(error){duplicateRender=error;}
        assert(duplicateRender?.kind==='conflict','duplicate renderer cannot replace its local owner');
        await new Promise(resolve=>setTimeout(resolve,100));
        const ended=await connection.end({reasonCode:200,reason:'Finished',expectedStreamRevision:completedConnection.receiptStreamRevision},mutation('native-end'));await ended.wait(options);
        assert(capture.closed&&captured[cameraBefore].getTracks().every(t=>t.readyState==='ended')&&controls.state==='idle','recorded end retires owned tracks and UI');result[path==='webtransport'?'wt_owned_capture_retired':'wss_owned_capture_retired']++;
        await within(renderer.finished,'Owned renderer retirement');result[path==='webtransport'?'wt_owned_renderer_retired':'wss_owned_renderer_retired']++;
       }finally{renderer?.close();controls.close();progress.close();picker.close();controls.element.remove();progress.element.remove();picker.element.remove();}

      }
      {const current=await pending.refresh(options),update={action:'medium-upgrade',media:nonempty(['screen','data'])},key=mutation('session-medium-upgrade');assert(!current.media.includes('screen')&&!current.media.includes('data'),'initial accepted Session has only camera/audio media');sessionStorage.setItem('session-medium-upgrade-intent-'+suffix,JSON.stringify({update,expectedTopologyRevision:current.topologyRevision,mutation:key}));const operation=await pending.update(update,current.topologyRevision,key);sessionStorage.setItem('session-medium-upgrade-reference-'+suffix,JSON.stringify(operation.referenceJson()));const upgraded=(await operation.wait(options)).value;assert(upgraded.receiptSnapshot.topologyRevision===current.topologyRevision+1&&JSON.stringify([...upgraded.receiptSnapshot.media].sort())===JSON.stringify(['audio','data','screen','video']),'additive upgrade supplies current Screen and data media floors');result.session_updates++;}
      {
       const cycle=path==='webtransport'?0:1,picker=new BrowserSourcePicker({audio:true,video:['screen']});document.body.append(picker.element);await picker.refresh(options);const selection=picker.selection();assert(selection.video==='screen'&&selection.videoDeviceId===undefined,'screen choice cannot use a camera ID');
       const input={connectionId:sdkId('screen-'+suffix),streams:offered([{id:sdkId('screen-audio-'+suffix),kind:'audio',direction:'sendrecv',codecs:['opus']},{id:sdkId('screen-video-'+suffix),kind:'screen',direction:'sendrecv',codecs:['vp8']}])},mutate={...mutation('screen-offer'),timeoutMs:25000};sessionStorage.setItem('screen-intent-'+suffix,JSON.stringify({input,mutation:mutate,selection}));
       const offer=await pending.connections.offer(input,mutate);sessionStorage.setItem('screen-reference-'+suffix,JSON.stringify(offer.referenceJson()));
       const connection=pending.connections.byId(input.connectionId),controls=new BrowserCaptureControls(connection,selection,{timeoutMs:10000}),progress=new BrowserOperationProgress(offer,{timeoutMs:10000});document.body.append(controls.element,progress.element);let renderer,capture,preview,peerCapture,peerRenderer,peerConnection,syntheticContext,syntheticTimer,syntheticStream;
       const displayBefore=displays.length,audioBefore=captured.length;
       try{
        let trustedClicks=0;const start=controls.element.querySelector('button');start.addEventListener('click',event=>{if(event.isTrusted)trustedClicks++;});start.focus();await ui('screen-capture-'+cycle);await waitUntil(()=>['ready','paused','failed'].includes(controls.state),'owned tab permission');assert(['ready','paused'].includes(controls.state)&&trustedClicks===1,'screen capture starts only from trusted keyboard decision');capture=await controls.start();assert(displays.length===displayBefore+1&&captured.length===audioBefore+1,'one owned display and synthetic microphone');
        const display=displays[displayBefore];assert(display.getVideoTracks()[0].getSettings().displaySurface==='browser','actual capture is a browser tab');
        trace.push({screen_visibility:document.visibilityState,capture_state:controls.state,enabled:display.getVideoTracks()[0].enabled});
        if(controls.state==='paused'){controls.element.querySelectorAll('button')[2].focus();await ui('screen-resume-'+cycle);await waitUntil(()=>controls.state==='ready','visible explicit screen resume');}
        // Witness the owned source tab itself. This is local capture evidence,
        // not a decoded receive-quality observation.
        preview=document.createElement('video');preview.muted=true;preview.srcObject=display;await within(preview.play(),'owned tab preview');assert(preview.readyState>=2,'owned tab has a current video frame');const witness=document.createElement('canvas');witness.width=16;witness.height=16;witness.getContext('2d').drawImage(preview,0,0,16,16);const color=witness.getContext('2d').getImageData(8,8,1,1).data;trace.push({owned_tab_pixel:[...color],width:preview.videoWidth,height:preview.videoHeight,ready:preview.readyState});assert(Math.abs(color[0]-12)<20&&Math.abs(color[1]-145)<20&&Math.abs(color[2]-210)<20,'captured pixels identify only owned source tab');result.owned_tab_capture++;
        progress.element.querySelector('button').focus();await ui('screen-progress-'+cycle);if(cycle===1)globalThis.typedUiComplete=true;const completed=(await progress.observe()).value,sources=await completed.streams.list(options);const logical=sources.find(v=>v.id===input.streams.rest[0].id),view=await logical.refresh(options);assert(view.medium==='screen'&&view.ownership==='observation','logical Screen survives physical video SDP');result.screen_kind_observed++;
        result[path==='webtransport'?'wt_screen_connection':'wss_screen_connection']++;
        // A second original Connection uses an explicitly supplied synthetic
        // moving red canvas. Only the first sender captures the owned blue tab.
        const peerInput={connectionId:sdkId('peer-screen-'+suffix),streams:offered([{id:sdkId('peer-screen-audio-'+suffix),kind:'audio',direction:'sendrecv',codecs:['opus']},{id:sdkId('peer-screen-video-'+suffix),kind:'screen',direction:'sendrecv',codecs:['vp8']}])};
        const peerMutation={...mutation('peer-screen-offer'),timeoutMs:25000};sessionStorage.setItem('peer-screen-intent-'+suffix,JSON.stringify({input:peerInput,mutation:peerMutation}));
        const peerOffer=await hint.session.connections.offer(peerInput,peerMutation);sessionStorage.setItem('peer-screen-reference-'+suffix,JSON.stringify(peerOffer.referenceJson()));peerConnection=hint.session.connections.byId(peerInput.connectionId);
        const source=document.createElement('canvas');source.width=640;source.height=360;const ctx=source.getContext('2d');let tick=0;const draw=()=>{ctx.fillStyle='rgb(210,30,40)';ctx.fillRect(0,0,640,360);ctx.fillStyle='white';ctx.fillRect((tick++*7)%600,0,12,12);};draw();syntheticTimer=setInterval(draw,33);
        syntheticContext=new AudioContext();const oscillator=syntheticContext.createOscillator(),sink=syntheticContext.createMediaStreamDestination();oscillator.frequency.value=660;oscillator.connect(sink);oscillator.start();await syntheticContext.resume();syntheticStream=new MediaStream([...sink.stream.getAudioTracks(),...source.captureStream(30).getVideoTracks()]);
        peerCapture=await peerConnection.capture({audio:true,video:'screen',stream:syntheticStream},options);const peerCompleted=(await peerOffer.wait(options)).value;
        let revision=(await connection.refresh(options)).streamRevision,peerRevision=(await peerConnection.refresh(options)).streamRevision;assert(revision&&peerRevision,'current owners expose negotiated revision');
        const callerParticipant=await room.participants.byIdentity(sdkId(identities[0]),options);
        const subscribe=async(owner,participant,expected,name)=>{const sourceId=participant===target.id?peerInput.streams.rest[0].id:input.streams.rest[0].id;const selection=path==='webtransport'?{participant,kinds:{first:'screen',rest:[]}}:{stream:sourceId};const request={selectors:{first:selection,rest:[]},expectedStreamRevision:expected},key=mutation(name);sessionStorage.setItem(name+'-intent-'+suffix,JSON.stringify({input:request,mutation:key}));const operation=await owner.streams.subscribe(request,key);sessionStorage.setItem(name+'-reference-'+suffix,JSON.stringify(operation.referenceJson()));try{return (await operation.wait(options)).value;}catch(error){trace.push({subscription_failed:await operation.status(options)});throw error;}};
        const selected=await subscribe(connection,target.id,revision,'screen-subscribe');revision=selected.streamRevision;
        const peerSelected=await subscribe(peerConnection,callerParticipant.id,peerRevision,'peer-screen-subscribe');peerRevision=peerSelected.streamRevision;
        const receive=(receipt,own)=>{assert(receipt.bindings!==null,'committed binding capture available');const candidates=receipt.bindings.filter(v=>v.parameters.medium==='screen');const chosen=candidates.find(v=>v.parameters.id!==own)||candidates[0];assert(chosen,'typed Screen receiver parameters');return receipt.connection.streams.byId(chosen.parameters.id);};
        const output=document.createElement('canvas'),peerOutput=document.createElement('canvas');renderer=await connection.render({audio:false,video:[{stream:receive(selected,input.streams.rest[0].id),canvas:output}]},options);peerRenderer=await peerConnection.render({audio:false,video:[{stream:receive(peerSelected,peerInput.streams.rest[0].id),canvas:peerOutput}]},options);
        const pixel=canvas=>canvas.getContext('2d').getImageData(canvas.width/2,canvas.height/2,1,1).data;const matches=(canvas,color)=>{const p=pixel(canvas);return color.every((v,i)=>Math.abs(v-p[i])<28);};
        await waitUntil(()=>matches(output,[210,30,40])&&matches(peerOutput,[12,145,210]),'opposite Screen decoded pixels');trace.push({screen_decoded:path,caller:[...pixel(output)],callee:[...pixel(peerOutput)]});result.screen_duplex_decoded++;
        const update=async(action)=>{revision=(await connection.refresh(options)).streamRevision;const key=mutation('screen-'+action),input={action,streams:{first:logical.id,rest:[]},...(action==='hold'?{direction:'inactive'}:{})};sessionStorage.setItem('screen-'+action+'-intent-'+suffix,JSON.stringify({input,revision,mutation:key}));const operation=await connection.update(input,revision,key);sessionStorage.setItem('screen-'+action+'-reference-'+suffix,JSON.stringify(operation.referenceJson()));let receipt;try{receipt=(await operation.wait(options)).value;}catch(error){trace.push({control_failed:await operation.status(options)});throw error;}assert(receipt.outcome.kind==='gates'&&receipt.outcome.action===action&&receipt.outcome.streams.some(v=>v.parameters.id===logical.id),'typed original gate receipt');revision=receipt.streamRevision;return receipt;};
        const sourceFrames=peerRenderer.diagnostics.video;await waitUntil(()=>peerRenderer.diagnostics.video>=sourceFrames+2,'Screen delivery before mute');
        await update('mute');await new Promise(r=>setTimeout(r,500));const mutedFrames=peerRenderer.diagnostics.video,oppositeFrames=renderer.diagnostics.video;await new Promise(r=>setTimeout(r,400));assert(peerRenderer.diagnostics.video===mutedFrames&&renderer.diagnostics.video>oppositeFrames,'mute stops selected Screen source while opposite sender remains live');
        await update('unmute');await waitUntil(()=>peerRenderer.diagnostics.video>mutedFrames,'Screen delivery after unmute');
        const heldReceipt=await update('hold');await new Promise(r=>setTimeout(r,500));const heldFrames=peerRenderer.diagnostics.video;await new Promise(r=>setTimeout(r,400));assert(peerRenderer.diagnostics.video===heldFrames,'inactive hold stops selected Screen source');
        if(path==='webtransport'){
         const current=(await connection.refresh(options)).streamRevision,stale=heldReceipt.streamRevision-1,key=mutation('screen-stale-resume');assert(stale>=1&&current>=heldReceipt.streamRevision,'last completed native hold proves the stale revision');
         const wire={v:2,id:'screen-stale-validation',ts:'2026-01-01T00:00:00Z',type:'connection.update',cid:room.id,sid:connection.sessionId,connid:connection.id,operation_id:key.operationId,expected_revision:stale,payload:{action:'resume',streams:[logical.id]}},reference={schema:'connect-operation-reference/1',operation_id:key.operationId,command:'connection.update',scope:{conversation_id:room.id,session_id:connection.sessionId,connection_id:connection.id}};
         sessionStorage.setItem('screen-stale-resume-intent-'+suffix,JSON.stringify(wire));sessionStorage.setItem('screen-stale-resume-reference-'+suffix,JSON.stringify(reference));
         await caller.raw.request(caller.raw.validate(wire),options);
         const original=caller.operations.restore(JSON.parse(sessionStorage.getItem('screen-stale-resume-reference-'+suffix)),browserOperationKinds.connectionUpdate);let refused;try{await original.wait(options);}catch(error){refused=error;}
         const status=await original.status(options);
         trace.push({screen_stale_observation:{requested:stale,observed_before:current,status,refused:refused&&{kind:refused.kind,certainty:refused.certainty}}});
         assert(refused?.kind==='application'&&refused?.certainty==='recorded'&&status.state==='failed'&&status.problem?.code==='conflict'&&status.problem?.message==='Operation intent, revision or execution reservation conflicts','Store admission retains its exact failed stale-revision receipt');result.screen_stale_gate_refused++;
         const recovered=await caller.operations.restore(reference,browserOperationKinds.connectionUpdate).status(options);assert(recovered.recordId===status.recordId&&recovered.revision===status.revision&&JSON.stringify(recovered.problem)===JSON.stringify(status.problem),'original failed receipt remains unchanged on restore');result.screen_conflict_original_recovered++;
         assert(peerRenderer.diagnostics.video===heldFrames,'failed stale resume cannot unpause Screen');trace.push({screen_stale_refused:{requested:stale,held_revision:heldReceipt.streamRevision,record_id:status.recordId}});
        }
        await update('resume');await waitUntil(()=>peerRenderer.diagnostics.video>heldFrames,'Screen delivery after resume');trace.push({screen_gate_delivery:path,muted_frames:mutedFrames,held_frames:heldFrames,resumed_frames:peerRenderer.diagnostics.video});result.screen_controls++;
        // Exact remote logical ID, followed by the corresponding original selector,
        // exercises routing removal/reinstallation without a new Connection.
        const unsubscribeInput={streams:{first:peerInput.streams.rest[0].id,rest:[]},expectedStreamRevision:revision},unsubscribeKey=mutation('screen-unsubscribe');sessionStorage.setItem('screen-unsubscribe-intent-'+suffix,JSON.stringify({input:unsubscribeInput,mutation:unsubscribeKey}));const unsubscribe=await connection.streams.unsubscribe(unsubscribeInput,unsubscribeKey);sessionStorage.setItem('screen-unsubscribe-reference-'+suffix,JSON.stringify(unsubscribe.referenceJson()));revision=(await unsubscribe.wait(options)).value.streamRevision;
        renderer.close();await within(renderer.finished,'prior Screen renderer close');
        const reselected=await subscribe(connection,target.id,revision,'screen-resubscribe');revision=reselected.streamRevision;const resumedOutput=document.createElement('canvas');renderer=await connection.render({audio:false,video:[{stream:receive(reselected,input.streams.rest[0].id),canvas:resumedOutput}]},options);await waitUntil(()=>matches(resumedOutput,[210,30,40]),'Screen receive restored after resubscribe');result.screen_resubscription++;
        if(path==='wss-webrtc'){let refused;const op=await connection.streams.subscribe({selectors:{first:{participant:target.id,kinds:{first:'video',rest:[]}},rest:[]},expectedStreamRevision:revision},mutation('screen-camera-refused'));try{await op.wait(options);}catch(error){refused=error;}assert(refused&&refused.kind==='application'&&refused.certainty==='recorded','Screen slot refuses unnegotiated camera selector');result.screen_camera_selector_refused++;}
        peerRevision=(await peerConnection.refresh(options)).streamRevision;const peerEnd=await peerConnection.end({reasonCode:200,reason:'Synthetic counterpart finished',expectedStreamRevision:peerRevision},mutation('peer-screen-end'));await peerEnd.wait(options);await within(peerRenderer.finished,'counterpart Screen renderer retirement');assert(peerCapture.closed,'counterpart capture retired');
        revision=(await connection.refresh(options)).streamRevision;
        const end=await connection.end({reasonCode:200,reason:'Screen finished',expectedStreamRevision:revision},mutation('screen-end'));await end.wait(options);await within(renderer.finished,'screen renderer retirement');assert(capture.closed&&display.getTracks().every(t=>t.readyState==='ended')&&captured[audioBefore].getTracks().every(t=>t.readyState==='ended')&&controls.state==='idle','screen end retires only owned tracks and UI');result.owned_tab_capture_retired++;
       }finally{clearInterval(syntheticTimer);for(const track of syntheticStream?.getTracks()??[])track.stop();await syntheticContext?.close();peerCapture?.close();peerRenderer?.close();if(preview){preview.pause();preview.srcObject=null;}renderer?.close();controls.close();progress.close();picker.close();controls.element.remove();progress.element.remove();picker.element.remove();}
      }
      // Current Session policy is distinct from live Connection negotiation.
      // All media owners have ended before these idle policy controls.
      for(const update of [{action:'role-change',participantId:target.id,role:'observer'},{action:'codec-renegotiate',codecPreferences:nonempty(['opus','vp8'])}]){
       const current=await pending.refresh(options),key=mutation('session-'+update.action);sessionStorage.setItem('session-'+update.action+'-intent-'+suffix,JSON.stringify({update,expectedTopologyRevision:current.topologyRevision,mutation:key}));const op=await pending.update(update,current.topologyRevision,key);sessionStorage.setItem('session-'+update.action+'-reference-'+suffix,JSON.stringify(op.referenceJson()));const observed=(await op.wait(options)).value;assert(observed.id===pending.id&&observed.receiptSnapshot.topologyRevision===current.topologyRevision+1,'typed original Session update receipt');if(update.action==='codec-renegotiate')assert(JSON.stringify(observed.receiptSnapshot.codecPreferences)===JSON.stringify(['opus','vp8']),'original codec policy');result.session_updates++;
      }
      {const current=await hint.session.refresh(options),key=mutation('session-leave'),update={action:'participant-leave',participantId:target.id};sessionStorage.setItem('session-leave-intent-'+suffix,JSON.stringify({update,expectedTopologyRevision:current.topologyRevision,mutation:key}));const leave=await hint.session.update(update,current.topologyRevision,key);await leave.wait(options);result.session_updates++;
       const latest=await pending.refresh(options),input={to:nonempty([target.id]),medium:'mixed',intent:'synthetic',capabilities:{audio:['opus'],video:['vp8'],screen:['vp8'],data:['reliable-ordered']},expectedTopologyRevision:latest.topologyRevision},reinviteKey=mutation('session-reinvite');sessionStorage.setItem('session-reinvite-intent-'+suffix,JSON.stringify({input,mutation:reinviteKey}));const reinvite=await pending.inviteParticipants(input,reinviteKey);sessionStorage.setItem('session-reinvite-reference-'+suffix,JSON.stringify(reinvite.referenceJson()));assert((await reinvite.wait(options)).value.id===pending.id,'existing Session invitation preserves stable ID');const renewed=(await incoming.next()).value;assert(renewed.session.id===hint.session.id&&renewed.operationId!==hint.operationId,'fresh invitation source for explicit reinvitation');const currentRenewed=await renewed.session.refresh(options),accepted=await renewed.session.accept({capabilities:renewed.capabilities,expectedTopologyRevision:currentRenewed.topologyRevision},mutation('session-reaccept'));await accepted.wait(options);result.session_reinvitations++;}
      const latest=await pending.refresh(options),end=await pending.end({reasonCode:200,reason:'Finished',expectedTopologyRevision:latest.topologyRevision},mutation('end'));
      const saved=end.referenceJson();sessionStorage.setItem('end-reference-'+suffix,JSON.stringify(saved));await end.wait(options);
      await caller.reconnect({preferredTransport:path});
      {const original=caller.operations.restore(JSON.parse(sessionStorage.getItem('typed-message-caller-reference-'+suffix)),browserOperationKinds.messageSend),value=(await original.wait(options)).value;assert(value.messageId===sdkId('typed-message-caller-'+suffix)&&value.boundary==='local-mutation','original typed message acceptance survives physical replacement');result.message_recovered_original++;}

      {
       for(const media of ['media','screen']){
        const historical=(await caller.operations.restore(JSON.parse(sessionStorage.getItem(media+'-reference-'+suffix)),browserOperationKinds.connectionOffer).wait(options)).value;
        let refused;try{await historical.capture({audio:true},options);}catch(error){refused=error;}
        assert(refused?.kind==='receipt-unavailable','restored historical offer has no new physical media custody');result.restored_media_observation_only++;if(media==='screen')result.screen_recovery_without_custody++;
       }
      }
      {const saved=JSON.parse(sessionStorage.getItem('session-role-change-reference-'+suffix)),original=caller.operations.restore(saved,browserOperationKinds.sessionUpdate),observed=(await original.wait(options)).value,current=await observed.refresh(options);assert(observed.receiptSnapshot.topologyRevision<current.topologyRevision&&current.state==='ended','historical Session receipt does not replace current state');result.restored_session_updates++;}
      for(const [name,kind] of [['screen-subscribe',browserOperationKinds.streamSubscribe],['screen-resume',browserOperationKinds.connectionUpdate],['screen-unsubscribe',browserOperationKinds.streamUnsubscribe]]){const restored=(await caller.operations.restore(JSON.parse(sessionStorage.getItem(name+'-reference-'+suffix)),kind).wait(options)).value;assert(restored.connection.id===sdkId('screen-'+suffix),'original control retains its Connection scope');let denied;try{await restored.connection.render({audio:false},options);}catch(error){denied=error;}assert(denied?.kind==='receipt-unavailable','restored control has no new physical custody');result.restored_control_observations++;}
      const restored=caller.operations.restore(JSON.parse(sessionStorage.getItem('end-reference-'+suffix)),browserOperationKinds.sessionEnd),ended=await restored.wait(options);
      assert(ended.value.receiptSnapshot.state==='ended'&&(await ended.value.refresh(options)).state==='ended','original receipt survives physical replacement');result.recovered_original++;
     }
    }
    const cancelInvite=await room.sessions.invite({sessionId:sdkId('session-cancel-'+suffix),to:[target.id],medium:'mixed',intent:'synthetic',capabilities:{audio:['opus'],video:['vp8'],screen:['vp8']}},mutation('invite-cancel'));
    const invited=(await cancelInvite.wait(options)).value,received=(await incoming.next()).value;
    assert(received.session.id===invited.id,'cancellation recipient scope');
    const live=await invited.refresh(options),cancel=await invited.cancelInvitation({reasonCode:487,reason:'Cancelled',expectedTopologyRevision:live.topologyRevision},mutation('cancel'));
    assert((await cancel.wait(options)).value.state==='cancelled','typed cancellation');result.cancellations++;
    savedCancellations.set(path,cancel.referenceJson());
    {const input={sessionId:sdkId('session-queued-cancel-'+suffix),to:[target.id],medium:'text-chat',intent:'queued-synthetic',capabilities:{data:['reliable-ordered']}},key=mutation('queued-invite');sessionStorage.setItem('queued-invite-intent-'+suffix,JSON.stringify({input,mutation:key}));const queued=await room.sessions.invite(input,key),session=(await queued.wait(options)).value,current=await session.refresh(options),cancelInput={reasonCode:487,reason:'Queued cancellation',expectedTopologyRevision:current.topologyRevision},cancelKey=mutation('queued-cancel');sessionStorage.setItem('queued-cancel-intent-'+suffix,JSON.stringify({input:cancelInput,mutation:cancelKey}));const cancelled=await session.cancelInvitation(cancelInput,cancelKey);await cancelled.wait(options);await incoming.close();incoming=callee.incomingInvitations({timeoutMs:100});let empty;try{await incoming.next();}catch(error){empty=error;}assert(empty?.kind==='deadline'&&callee.transport.state==='connected','queued cancellation yields no stale invitation or automatic decision');result.queued_cancellation_refused++;}
   }finally{await incoming.close();}
  }finally{for(const client of clients)await client.close({timeoutMs:5000});}
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
  }finally{await client.close({timeoutMs:5000});}
 }
 const mutations=trace.filter(x=>x.operation&&['conversation.create','message.send','conversation.participant.add','conversation.participant.update','conversation.participant.remove','session.invite','session.accept','session.reject','session.cancel','session.end','session.update','connection.offer','connection.end','connection.update','stream.subscribe','stream.unsubscribe'].includes(x.request));
 assert(mutations.length===76&&new Set(mutations.map(x=>x.operation)).size===76,'seventy-six original mutations including one refused stale resume, no status/restore resubmission');
 return {...result,credentials:4,devices:2,endpoints:4,original_mutations:mutations.length};
}catch(error){throw Error(String(error.stack??error)+"\n"+JSON.stringify(error.problem??{})+"\n"+JSON.stringify(await rtcObservations())+"\n"+JSON.stringify(trace));}};
