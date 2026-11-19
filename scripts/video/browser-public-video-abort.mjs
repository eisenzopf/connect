/** One installed-Chromium integration invoked only by its normal Rust test.
 * Private signed configuration enters stdin. No matrix entrypoint is called. */
import assert from 'node:assert/strict';
import {createPrivateKey,webcrypto} from 'node:crypto';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {closeOwnedChromium} from '../lib/browser_owned_chromium.mjs';

let input='';for await(const part of process.stdin){input+=part;assert.ok(input.length<=65536);}
const spec=JSON.parse(input);
assert.equal(spec.format,'connect/chromium-original-public-b-abort-integration/1');
assert.equal(spec.operationId,'browser-wire-public-b-abort');
assert.equal(spec.actors.length,3);assert.equal(spec.sessions.length,2);
const root=await fs.realpath(spec.root),privateRoot=await fs.realpath(spec.privateDirectory);
assert.ok(privateRoot.startsWith(root+path.sep)||privateRoot.startsWith('/private/')||privateRoot.startsWith('/tmp/'));
for(const actor of spec.actors){
 const url=new URL(actor.url);assert.equal(url.protocol,'wss:');assert.ok(['localhost','127.0.0.1'].includes(url.hostname));
 assert.ok(!url.username&&!url.password&&!url.search&&!url.hash);
 const alg=actor.server.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
 actor.server.jwk=await webcrypto.subtle.exportKey('jwk',await webcrypto.subtle.importKey('raw',Buffer.from(actor.server.publicRaw,'base64url'),alg,true,['verify']));
 delete actor.server.publicRaw;
 if(actor.signing.privatePkcs8){actor.signing.jwk=createPrivateKey({key:Buffer.from(actor.signing.privatePkcs8,'base64url'),format:'der',type:'pkcs8'}).export({format:'jwk'});delete actor.signing.privatePkcs8;}
}
const chromium=(await import(process.env.CONNECT_PLAYWRIGHT_MODULE?pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href:'playwright')).chromium;
const sdkRoot=await fs.realpath(path.join(root,'sdk/typescript/dist'));
const tapFile=path.join(root,'scripts/lib/browser_matrix_rtc_observation.mjs');
const pcmFile=path.join(root,'scripts/lib/browser_matrix_pcm.mjs');
const boundaryFile=path.join(root,'scripts/lib/browser_matrix_stages.mjs');
const assets=http.createServer(async(request,response)=>{
 try{
  assert.equal(request.method,'GET');const url=new URL(request.url,'http://127.0.0.1');
  if(url.pathname==='/'){response.writeHead(200,{'Content-Type':'text/html'});response.end('<!doctype html><meta charset=utf-8><title>Owned terminal integration</title>');return;}
  let file;
  if(url.pathname==='/rtc-observer.mjs')file=tapFile;
  else if(url.pathname==='/browser_matrix_rtc_audio_diagnostics.mjs')file=path.join(root,'scripts/lib/browser_matrix_rtc_audio_diagnostics.mjs');
  else if(url.pathname==='/pcm-identity.mjs')file=pcmFile;
  else if(url.pathname==='/pcm-boundary.mjs')file=boundaryFile;
  else {assert.ok(url.pathname.startsWith('/sdk/'));file=await fs.realpath(path.resolve(sdkRoot,'.'+url.pathname.slice(4)));assert.ok(file.startsWith(sdkRoot+path.sep)&&file.endsWith('.js'));}
  response.writeHead(200,{'Content-Type':'text/javascript'});response.end(await fs.readFile(file));
 }catch{response.writeHead(404);response.end('unavailable');}
});
await new Promise((resolve,reject)=>{assets.once('error',reject);assets.listen(spec.assetPort,'127.0.0.1',resolve);});
let browser,context,result,primaryFailure;
try{
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${spec.debugPort}`);
 context=await browser.newContext();const page=await context.newPage();
 await page.exposeFunction('terminalCheckpoint',async(stage,proof,remainingMs)=>{
  assert.ok(['abort-candidate-closed','abort-complete'].includes(stage)||stage==='before-public-ready-0-0');
  assert.ok(Number.isFinite(remainingMs)&&remainingMs>0&&remainingMs<=8000);
  assert.ok(proof.operation_id===spec.operationId||new RegExp('^'+spec.operationId+'-[01]-[01]$').test(proof.operation_id));
  const event=path.join(privateRoot,stage+'.json'),ack=path.join(privateRoot,stage+'.ack.json');
  const pending=event+'.pending',file=await fs.open(pending,'wx',0o600);
  try{await file.writeFile(JSON.stringify(proof));await file.sync();}finally{await file.close();}
  await fs.rename(pending,event);
  const until=performance.now()+remainingMs;
  for(;;){
   try{const value=JSON.parse(await fs.readFile(ack,'utf8'));assert.equal(value.operation_id,proof.operation_id);assert.equal(value.stage,stage);return;}
   catch(error){if(error.code!=='ENOENT')throw error;}
   assert.ok(performance.now()<until,'same original fixture checkpoint deadline');
   await new Promise(resolve=>setTimeout(resolve,5));
  }
 });
 await page.exposeFunction('retainPublicPcmDiagnostics',async(stage,value)=>{
  assert.ok(['failure','closed','restore-wait'].includes(stage));const text=JSON.stringify(value);assert.ok(text.length<=2*1024*1024,'bounded original public PCM diagnostics');
  await fs.writeFile(path.join(privateRoot,'rtc-audio-'+stage+'.json'),text,{flag:'wx',mode:0o600});
 });
 await page.goto(`http://127.0.0.1:${spec.assetPort}/`,{waitUntil:'domcontentloaded'});
 result=await page.evaluate(async spec=>{
  const {BrowserSdkClient}=await import('/sdk/browser_sdk.js');
  const {BrowserRtcConnection}=await import('/sdk/browser_sdk_webrtc.js');
  const {BrowserClient}=await import('/sdk/browser_client.js');
  const {browserMatrixRtcObservation}=await import('/rtc-observer.mjs');
  const {pcmIdentity}=await import('/pcm-identity.mjs');
  const {pcmBoundary}=await import('/pcm-boundary.mjs');
  const check=(value,message)=>{if(!value)throw Error(message);};
  const opts=()=>({timeoutMs:8000}),mutation=operationId=>({...opts(),operationId});
  const done=async promise=>(await (await promise).wait(opts())).value;
  const wait=async(predicate,until,message)=>{while(!predicate()){check(performance.now()<until,message);await new Promise(resolve=>setTimeout(resolve,5));}};
  const clients=[],captures=[],renders=[],timers=[],oscillators=[],surfaces=[];
  const current=[],byConnection=new Map(),active=new Map(),progress=new Map(),ready=new Map(),moves=[];
  const sourceHz=[440,660,880,1100],expected=[1,0,3,2],observationFailures=[];
  const renderTimes=new Map(),audioFailures=[];
  globalThis.connectRtcAudioFacts=true;
  const audio=new AudioContext({sampleRate:48000});await audio.resume();
  let observer,functionalFailure,cleanupFailures=[],measuring=false,closeProof,publicSends=0;
  const cancelTargetWait=new AbortController();
  const continuation=BrowserClient.prototype.requestReplacementContinuation;
  const NativePeer=globalThis.RTCPeerConnection,begin=BrowserRtcConnection.begin,beginReplacement=BrowserRtcConnection.beginReplacement;
  const nativeFrame=HTMLVideoElement.prototype.requestVideoFrameCallback,nativeDraw=CanvasRenderingContext2D.prototype.drawImage;
  const peers=new Map(),decoded=new Map(),warm=new Map(),phases=new Map(),publicMarkers=new Map(),publicDraws=new Map(),renderTargets=new Map();
  let arming=null;
  globalThis.RTCPeerConnection=class extends NativePeer {
   constructor(...args){super(...args);check(arming&&peers.size<8&&!peers.has(arming),'eight exact original SDK native peers');peers.set(arming,this);}
  };
  for(const [name,original]of [['begin',begin],['beginReplacement',beginReplacement]])BrowserRtcConnection[name]=function(...args){
   check(arming===null,'original synchronous SDK constructor');arming=args[1].connectionId;
   try{return original.apply(this,args);}finally{arming=null;}
  };
  HTMLVideoElement.prototype.requestVideoFrameCallback=function(callback){
   const element=this;return nativeFrame.call(element,function(now,metadata){
    try{const tracks=element.srcObject instanceof MediaStream?element.srcObject.getVideoTracks():[];
    if(tracks.length===1){const track=tracks[0];const matches=[...peers].flatMap(([id,peer])=>peer.getTransceivers().filter(t=>t.receiver.track===track).map(t=>({id,peer,transceiver:t})));
     check(matches.length===1&&typeof matches[0].transceiver.mid==='string','actual original receiver/track/MID per decoded callback');
     let value=decoded.get(track);if(!value){check(decoded.size<16,'bounded original video elements');value={element,track,...matches[0],callbacks:0};decoded.set(track,value);}
     check(value.element===element&&value.callbacks<50000,'one original decoder element per track and bounded callbacks');value.callbacks++;value.last=performance.now();
    }
    }catch(error){invalid(`native callback ownership: ${String(error.message).slice(0,120)}`);}
    return callback.call(element,now,metadata);
   });
  };
  CanvasRenderingContext2D.prototype.drawImage=function(source,...args){
   try{const id=renderTargets.get(this.canvas);if(id){const value=warm.get(id);check(value&&source===value.element,'public output draws SAME already decoded B element');check(value.transceiver.receiver.track===value.track,'public draw SAME actual native receiver');publicDraws.set(id,(publicDraws.get(id)??0)+1);}}catch(error){invalid(`public B draw ownership: ${String(error.message).slice(0,120)}`);}
   return nativeDraw.call(this,source,...args);
  };
  const observeReply=frames=>{try{for(const frame of frames)if(frame.type==='operation.updated'&&frame.payload?.webrtc_public_video_phase){const value=structuredClone(frame.payload);phases.set(value.operation_id,value);}}catch(error){invalid(`signed phase observation: ${String(error.message).slice(0,120)}`);}};
  BrowserClient.prototype.requestReplacementContinuation=function(type,payload,scope,options){
   const marker=payload.replacement_preparation,profile=marker?.profile;
   const original=()=>{if(type==='connection.offer'&&profile==='connect-replacement-public-video-ready-v1')publicSends++;const work=continuation.call(this,type,payload,scope,options);void work.then(observeReply,()=>{});return work;};
   if(type!=='connection.offer')return original();
   if(profile==='connect-replacement-ready-v1'){
    const evidence=marker.readiness,old=ready.get(evidence.audio.stream_id);if(old)check(JSON.stringify(old)===JSON.stringify(evidence),'same original private marker replay');else ready.set(evidence.audio.stream_id,structuredClone(evidence));
   }
   if(profile!=='connect-replacement-public-video-ready-v1')return original();
   const snapshot=JSON.stringify(marker),previous=publicMarkers.get(scope.operation_id);
   if(previous){check(previous.bytes===snapshot,'public marker replay bytes unchanged');return original();}
   const current=phases.get(scope.operation_id),phase=current?.webrtc_public_video_phase;
   check(current?.state==='preparing'&&phase?.phase==='cut-bound-post-f'&&phase.operation_record_id===marker.operation_record_id,'actual verified original Cut phase before B marker');
   const peer=peers.get(scope.connid);check(peer&&peer.getTransceivers().length===3,'actual audio+A+B native sections');
   const actual=peer.getTransceivers().filter(t=>t.mid===phase.binding.mid);check(actual.length===1,'exact original B native MID');const transceiver=actual[0],track=transceiver.receiver.track,value=decoded.get(track);
   const others=peer.getTransceivers().filter(t=>t!==transceiver);check(others.length===2&&others.some(t=>t.receiver.track.kind==='audio')&&others.some(t=>t.receiver.track.kind==='video')&&others.every(t=>t.direction==='sendrecv'&&t.currentDirection==='sendrecv'&&t.sender.track?.readyState==='live'),'actual audio and camera/preview A remain original duplex roles');
   check(transceiver.direction==='recvonly'&&transceiver.currentDirection==='recvonly'&&transceiver.sender.track===null&&value?.callbacks>0&&value.transceiver===transceiver,'real never-replaced B receiver decoded before Ready');
   check(marker.public_video_readiness.binding.mid===transceiver.mid&&marker.public_video_readiness.binding.stream_id===phase.binding.stream_id,'same native B binding in actual marker');
   warm.set(scope.connid,value);publicMarkers.set(scope.operation_id,{bytes:snapshot,marker:structuredClone(marker),phase:structuredClone(phase)});
   const match=new RegExp('^'+spec.operationId+'-([01])-([01])$').exec(scope.operation_id);check(match,'one original test Root');
   const cutoff=performance.now()+options.timeoutMs;
   return globalThis.terminalCheckpoint(`before-public-ready-${match[1]}-${match[2]}`,{operation_id:scope.operation_id,record_id:marker.operation_record_id,connection_id:scope.connid,marker:structuredClone(marker),phase,native:{mid:transceiver.mid,track_id:track.id,decoded_callbacks:value.callbacks}},options.timeoutMs).then(()=>{
    const remaining=Math.floor(cutoff-performance.now());check(remaining>0&&!options.signal?.aborted,'SAME original send cutoff after test checkpoint');
    check(peers.get(scope.connid)===peer&&peer.getTransceivers().filter(t=>t.mid===phase.binding.mid).length===1&&peer.getTransceivers().find(t=>t.mid===phase.binding.mid)===transceiver&&transceiver.receiver.track===track&&track.readyState==='live'&&decoded.get(track)===value,'same actual B pointers remain current across fixture checkpoint');
    const states=[...new Set(peer.getTransceivers().flatMap(t=>[t.sender.transport?.state,t.receiver.transport?.state]).filter(Boolean))];
    check(peer.connectionState==='connected'&&states.length>0&&states.every(state=>state==='connected'),'actual candidate connected before original native close');
    closeProof={operation_id:scope.operation_id,record_id:marker.operation_record_id,connection_id:scope.connid,at_ms:performance.now(),connection_state:peer.connectionState,dtls:states,only_native_peer_closed:true};
    NativePeer.prototype.close.call(peer);check(peer.connectionState==='closed','actual synchronous original native PC close');
    // This fixture holds the original public send BEFORE transmission. It
    // releases only on actual caller/owner cancellation after Root confirms
    // durable production Discarded, never by forging a response/error/Closed.
    return new Promise((_,reject)=>{
     const signal=options.signal;check(signal,'original actual owned preparation signal');
     const aborted=()=>{signal.removeEventListener('abort',aborted);reject(signal.reason);};
     signal.addEventListener('abort',aborted,{once:true});if(signal.aborted)aborted();
    });
   });
  };
  const invalid=message=>{if(measuring&&observationFailures.length<16)observationFailures.push(String(message).slice(0,160));};
  const retainPcm=async stage=>{try{const at=performance.now();await globalThis.retainPublicPcmDiagnostics(stage,{format:'connect/owned-public-pcm-diagnostics/1',fixture:'browser-public-video-abort',operation_id:spec.operationId,at,render_origins:[...renderTimes].map(([connection_id,value])=>({connection_id,...value})),audio_failures:audioFailures,observation_failures:observationFailures,receivers:[...progress].map(([connection_id,p])=>({connection_id,i:byConnection.get(connection_id),selected:active.get(byConnection.get(connection_id))===connection_id,audio:p.audio,video:p.video,changing:p.changing,lastAudio:p.lastAudio,lastVideo:p.lastVideo,audio_age_ms:p.lastAudio===null?null:at-p.lastAudio,video_age_ms:p.lastVideo===null?null:at-p.lastVideo})),current:current.map((value,i)=>({i,connection_id:value.connection.id})),phases:[...phases.values()].map(value=>({operation_id:value.operation_id,record_id:value.record_id,state:value.state,phase:value.webrtc_public_video_phase?.phase,target_connection_id:value.webrtc_public_video_phase?.target_connection_id,target_generation:value.webrtc_public_video_phase?.target_generation})),audio_diagnostics:observer?.report()??null});}catch{}};
  try{
   for(const actor of spec.actors){
    const algorithm=actor.signing.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
    const key=await crypto.subtle.importKey('jwk',actor.signing.jwk,algorithm,false,['sign']);
    const trust=await crypto.subtle.importKey('jwk',actor.server.jwk,algorithm,false,['verify']);
    const enrollment={credential:actor.credential,identityId:actor.identityId,device:actor.device,
     signing:{algorithm:actor.signing.algorithm,keyId:actor.signing.keyId,key},
     server:{algorithm:actor.server.algorithm,keyId:actor.server.keyId,key:trust,
      notBefore:actor.server.notBefore,expiresAt:actor.server.expiresAt}};
    clients.push(await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:actor.identityId,deviceId:actor.device.id,
     endpoints:{'wss-webrtc':actor.url},preferredTransport:'wss-webrtc',allowFallback:false,
     resolveEnrollment:async()=>enrollment,timeoutMs:8000,attemptTimeoutMs:3000}));
   }
   const offerings=[];
   for(let i=0;i<4;i++){
    const session=clients[i%2].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[Math.floor(i/2)]);
    const connectionId=`browser-wire-connection-${i}`,cutoff=performance.now()+8000;
    const options=()=>{
     const timeoutMs=Math.floor(cutoff-performance.now());
     check(timeoutMs>0,'original8s initial offer deadline');
     return {timeoutMs};
    };
    const operation=await session.connections.offer({connectionId,
     streams:{first:{id:`browser-wire-audio-${i}`,kind:'audio',direction:'sendrecv',codecs:{first:'opus',rest:[]}},
      rest:[{id:`browser-wire-video-${i}`,kind:'video',direction:'sendrecv',codecs:{first:'vp8',rest:[]}}]}},{...options(),operationId:`browser-wire-offer-${i}`});
    const connection=session.connections.byId(connectionId);
    offerings.push({operation,options});
    current.push({connection,actor:i%2,audio:`browser-wire-audio-${i}`,send:`browser-wire-video-${i}`,receive:`browser-wire-video-${i}`});
   }
   const oldViews=current.map(value=>value.connection);
   observer=browserMatrixRtcObservation(BrowserRtcConnection,{
    invalid(...args){invalid(args.at(-1));},
    measurementOrigin(id){const i=byConnection.get(id);return i!==undefined&&active.get(i)===id?renderTimes.get(id)?.started:null;},
    audio(id,timestamp,at,samples,clock){
     const i=byConnection.get(id);if(i===undefined||active.get(i)!==id)return;
     if(samples.length!==960){invalid('actual20ms decoded PCM');return;}
     const identity=pcmIdentity(samples,expected[i]);
     if(!(identity.rms>.03))return identity.source_identified;
     if(!identity.identified){
      if(audioFailures.length<8)audioFailures.push({id,timestamp,at,clock,expected_hz:sourceHz[expected[i]],render:renderTimes.get(id),
       receivedAudio:progress.get(id)?.audio,identity,boundary:pcmBoundary(samples)});
      invalid(`actual selected source tone id=${id.slice(0,32)} hz=${sourceHz[expected[i]]} rms=${identity.rms.toFixed(3)} powers=${identity.powers.map(power=>power.toExponential(3)).join('/')}`);return identity.source_identified;
     }
     const p=progress.get(id);p.audio++;p.lastAudio=at;return identity.source_identified;
    },
    video(id,timestamp,at,frame,width,height){
     const i=byConnection.get(id);if(i===undefined||active.get(i)!==id)return;
     if(width!==640||height!==360){invalid('actual640x360 decoded VP8');return;}
     const ctx=surfaces[i].measurement.getContext('2d');ctx.drawImage(frame,0,0,16,16);
     const pixel=ctx.getImageData(0,0,1,1).data,mark=60+expected[i]*40;
     if(!(Math.abs(pixel[0]-mark)<12&&Math.abs(pixel[1]-mark)<12)){invalid('actual selected source pixels');return;}
     const moving=ctx.getImageData(0,8,16,1).data,p=progress.get(id);
     if(p.lastPixels&&moving.some((v,n)=>v!==p.lastPixels[n]))p.changing++;
     p.lastPixels=[...moving];p.video++;p.lastVideo=at;
    }
   });
   for(let i=0;i<4;i++){
    const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
    const measurement=document.createElement('canvas');measurement.width=16;measurement.height=16;
    const oscillator=audio.createOscillator(),gain=audio.createGain();oscillator.frequency.value=sourceHz[i];gain.gain.value=.15;
    oscillator.connect(gain);oscillator.start();oscillators.push(oscillator);
    let ordinal=0;const draw=()=>{const ctx=canvas.getContext('2d'),mark=60+i*40;ctx.fillStyle=`rgb(${mark},${mark},${mark})`;
     ctx.fillRect(0,0,640,360);ctx.fillStyle='white';ctx.fillRect((ordinal++*13)%600,180,20,100);};
    draw();timers.push(setInterval(draw,1000/30));surfaces.push({canvas,measurement,gain,outputs:[]});
   }
   const startCapture=async(i,connection,options)=>{
    check(captures.length<8,'bounded four originals plus four actual candidates');
    const owner=await connection.capture({audio:true,audioNode:surfaces[i].gain,video:'camera',videoCanvas:surfaces[i].canvas},options);
    captures.push({id:connection.id,owner});return owner;
   };
   const startRender=async(i,binding,options)=>{
    const timeline={started:performance.now(),returned:null};renderTimes.set(binding.connection.id,timeline);
    const output=document.createElement('canvas');output.width=640;output.height=360;surfaces[i].outputs.push(output);
    check(renders.length<8,'bounded public render owners');byConnection.set(binding.connection.id,i);
    progress.set(binding.connection.id,{audio:0,video:0,changing:0,lastAudio:null,lastVideo:null,lastPixels:null});
    observer.attach(binding.connection.id,output);
    if(warm.has(binding.connection.id))renderTargets.set(output,binding.connection.id);
    // This is the local public renderer selection after BOTH original
    // completion receipts; private decoder outputs have never been rendered.
    active.set(i,binding.connection.id);
    const owner=await binding.connection.render({audio:true,video:[{stream:binding.connection.streams.byId(binding.receive),canvas:output}]},options);
    timeline.returned=performance.now();
    renders.push({owner,connectionId:binding.connection.id,retirement:owner.retirement,finished:owner.finished});
   };
   for(let i=0;i<4;i++)await startCapture(i,current[i].connection,offerings[i].options());
   // Retain the accepted original operations; only actual source flow can
   // complete their waits. Do not resubmit a mutation to escape this ordering.
   for(let i=0;i<4;i++){
    const completed=await offerings[i].operation.wait(offerings[i].options());
    check(completed.value.id===current[i].connection.id,'same original offered connection completes');
   }
   for(let i=0;i<4;i++){
    const connection=current[i].connection,view=await connection.refresh(opts()),peer=expected[i];
    await done(connection.streams.subscribe({selectors:{first:{stream:`browser-wire-audio-${peer}`},rest:[{stream:`browser-wire-video-${peer}`}]},
     expectedStreamRevision:view.streamRevision},mutation(`browser-wire-select-${i}`)));
   }
   for(let i=0;i<4;i++)await startRender(i,current[i],opts());
   const fresh=(id,after)=>{const p=progress.get(id),now=performance.now();return p&&p.audio>=3&&p.video>=3&&p.changing>0&&p.lastAudio>=after&&p.lastVideo>=after&&now-p.lastAudio<200&&now-p.lastVideo<200;};
   const startup=performance.now()+8000;
   await wait(()=>current.every(value=>fresh(value.connection.id,0)),startup,'four original decoded AV receivers');measuring=true;
   const epochs=clients.map(client=>client.transport.physicalEpoch);check(epochs.every(value=>Number.isSafeInteger(value)&&value>0),'actual original control epochs');
   const before=current[0],retain=current[1],operationId=`${spec.operationId}-0-0`,cutoff=performance.now()+8000;
   const options=()=>{const timeoutMs=Math.floor(cutoff-performance.now());check(timeoutMs>0,'SAME original8s abort command cutoff');return {timeoutMs};};
   const sourceSession=clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[0]);
   const targetSession=clients[2].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[0]);
   const view=await sourceSession.refresh(options());
   const streams={audio:'browser-public-b-abort-audio',send:'browser-public-b-abort-a',receive:'browser-public-b-abort-b'};
   const operation=await before.connection.replaceDevice({retain:retain.connection.id,target:{endpoint:spec.targetEndpoint},expectedTopologyRevision:view.topologyRevision,
    video:{publications:[{previous:before.send,replacement:streams.send}],receivers:[{previous:before.receive,replacement:streams.receive}]}},{...options(),operationId});
   const prepared=await targetSession.connections.prepareReplacement({operation:operation.reference,audio:streams.audio,video:{send:streams.send,receive:streams.receive}},{...options(),signal:cancelTargetWait.signal});
   const completion=prepared.completion.then(value=>({fulfilled:true,value}),error=>({fulfilled:false,error:{name:error?.name,kind:error?.kind,certainty:error?.certainty}}));
   await startCapture(0,prepared.connection,options());
   await wait(()=>!!closeProof,cutoff,'actual native close after real Cut and B decoded callback');
   const status=await operation.status(options());check(status.state!=='completed'&&status.recordId===closeProof.record_id,'same uncompleted original Root');
   // Root acknowledges ONLY after the real production watcher/whole-candidate
   // joins append Discarded and no Commit/Activated exists. No opaque token is
   // supplied to this browser or constructed by the fixture.
   await globalThis.terminalCheckpoint('abort-candidate-closed',{operation_id:operationId,record_id:status.recordId,connection_id:prepared.connection.id,close:closeProof},options().timeoutMs);
   const afterDiscarded=performance.now(),post=current.map(value=>({...progress.get(value.connection.id)}));
   cancelTargetWait.abort(); // local pending SDK wait only, AFTER actual durable Discarded
   const target=await completion;check(!target.fulfilled,'cancelled held wait cannot complete or grant B');
   try{
   await wait(()=>current.every((value,i)=>{const p=progress.get(value.connection.id);return fresh(value.connection.id,afterDiscarded)&&p.audio>post[i].audio&&p.video>post[i].video&&p.changing>post[i].changing;}),cutoff,'genuinely new fresh OLD and unrelated AV after actual Discarded/restore');
   }catch(error){try{await globalThis.retainPublicPcmDiagnostics('restore-wait',{operation_id:operationId,at:performance.now(),after_discarded:afterDiscarded,cutoff,post:post.map(p=>({...p,lastPixels:undefined})),current:current.map((value,i)=>{const p=progress.get(value.connection.id),at=performance.now();return {i,connection_id:value.connection.id,...p,lastPixels:undefined,audio_age_ms:p.lastAudio===null?null:at-p.lastAudio,video_age_ms:p.lastVideo===null?null:at-p.lastVideo};}),audio_failures:audioFailures,observation_failures:observationFailures,audio_diagnostics:observer.report()});}catch{}throw error;}
   for(let i=0;i<4;i++)check(captures.find(value=>value.id===current[i].connection.id)?.owner.closed===false,'all original camera/audio owners remain current after failed target');
   for(let i=0;i<clients.length;i++)check(clients[i].transport.physicalEpoch===epochs[i],'original signed control epochs unchanged');
   const source=await before.connection.refresh(options()),retained=await retain.connection.refresh(options());
   check(source.state==='ready'&&source.generation===1&&retained.state==='ready'&&retained.generation===1,'failed candidate never replaces OLD/current retained owner');
   check(publicSends===0,'no held public Ready was actually transmitted');
   check(observationFailures.length===0,'no malformed/wrong-source actual AV after restore: '+JSON.stringify(observationFailures));
   const report={operation_id:operationId,record_id:status.recordId,candidate:prepared.connection.id,close:closeProof,public_ready_sent:publicSends>0,local_wait_cancel_after_durable_discarded:true,target_failure:target.error,
    same_original_control_epochs:true,old_decoded_outputs:current.map(value=>({connection_id:value.connection.id,...progress.get(value.connection.id),lastPixels:undefined})),elapsed_ms:8000-(cutoff-performance.now())};
   await globalThis.terminalCheckpoint('abort-complete',report,options().timeoutMs);
   return {format:spec.format,passed:true,...report};
  }catch(error){functionalFailure=error;await retainPcm('failure');throw error;}
  finally{
   cancelTargetWait.abort();
   for(const timer of timers)clearInterval(timer);
   for(const value of captures){value.owner.close();if(value.owner.closed!==true)cleanupFailures.push('original-capture-close');}
   for(const entry of renders)entry.owner.close();
   for(const entry of renders){try{const receipt=await entry.retirement;
    check(receipt&&Object.isFrozen(receipt)&&receipt.connectionId===entry.connectionId&&receipt.scope==='renderer-local'&&receipt.outputsDetached===true&&['closed','not-created'].includes(receipt.audioGraph),'exact renderer retirement');await entry.finished;}
    catch(error){cleanupFailures.push(error.name);}}
   for(const oscillator of oscillators){try{oscillator.stop();oscillator.disconnect();}catch{cleanupFailures.push('source-close');}}
   try{await audio.close();}catch{cleanupFailures.push('source-audio-close');}
   if(observer){try{await observer.close();}catch{cleanupFailures.push('observer-close');}}
   await retainPcm('closed');
   if(!functionalFailure&&clients[0])for(let sid=0;sid<2;sid++){
    try{const session=clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[sid]),view=await session.refresh(opts());
     await done(session.end({reasonCode:0,reason:'original roundtrip cleanup',expectedTopologyRevision:view.topologyRevision},mutation(`browser-wire-session-end-${sid}`)));}
    catch{cleanupFailures.push('session-end');}
   }
   for(const client of clients){try{await client.close(opts());}catch{cleanupFailures.push('sdk-close');}}
   if(![...peers.values()].every(peer=>peer.connectionState==='closed'))cleanupFailures.push('original-native-peer-not-closed');
   BrowserClient.prototype.requestReplacementContinuation=continuation;
   BrowserRtcConnection.begin=begin;BrowserRtcConnection.beginReplacement=beginReplacement;globalThis.RTCPeerConnection=NativePeer;
   HTMLVideoElement.prototype.requestVideoFrameCallback=nativeFrame;CanvasRenderingContext2D.prototype.drawImage=nativeDraw;
   peers.clear();decoded.clear();warm.clear();phases.clear();publicMarkers.clear();publicDraws.clear();renderTargets.clear();
   ready.clear();byConnection.clear();active.clear();progress.clear();
   for(const group of surfaces){for(const canvas of [group.canvas,group.measurement,...group.outputs]){canvas.width=0;canvas.height=0;}}
   if(!functionalFailure)check(cleanupFailures.length===0,'original owned browser work did not join');
  }
 },spec);
}catch(error){primaryFailure=error;throw error;}
finally{
 const cleanup=await closeOwnedChromium({browser,context,assets});
 if(result)result.browser_close=cleanup.browserClose;
 if(!primaryFailure&&cleanup.failures.length)throw new AggregateError(cleanup.failures.map(value=>Error(value.phase+': '+value.name)),'Original browser cleanup failed');
}
process.stdout.write(JSON.stringify(result)+'\n');
