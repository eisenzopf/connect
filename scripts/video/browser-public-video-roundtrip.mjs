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
assert.equal(spec.format,'connect/chromium-original-public-b-roundtrip-integration/1');
assert.equal(spec.operationId,'browser-wire-public-b-roundtrip');
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
  assert.ok(stage==='roundtrip-complete'||/^before-public-ready-[01]-[01]$/.test(stage));
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
 await page.exposeFunction('originalReceiptFailure',value=>{
  const text=JSON.stringify(value);assert.ok(text.length<=4096,'bounded public receipt diagnostic');
  console.error('ORIGINAL_RECEIPT_FAILURE '+text);
 });
 await page.exposeFunction('retainPublicPcmDiagnostics',async(stage,value)=>{
  assert.ok(['failure','closed'].includes(stage));const text=JSON.stringify(value);assert.ok(text.length<=2*1024*1024,'bounded original public PCM diagnostics');
  await fs.writeFile(path.join(privateRoot,'rtc-audio-'+stage+'.json'),text,{flag:'wx',mode:0o600});
 });
 await page.goto(`http://127.0.0.1:${spec.assetPort}/`,{waitUntil:'domcontentloaded'});
 result=await page.evaluate(async spec=>{
  const {BrowserSdkClient}=await import('/sdk/browser_sdk.js');
  const {BrowserRtcConnection}=await import('/sdk/browser_sdk_webrtc.js');
  const {BrowserClient}=await import('/sdk/browser_client.js');
  const {browserMatrixRtcObservation}=await import('/rtc-observer.mjs');
  const {strictPcmIdentity:pcmIdentity,PcmSampleLedger}=await import('/pcm-identity.mjs');
  const pcmLedger=new PcmSampleLedger();
  const {pcmBoundary}=await import('/pcm-boundary.mjs');
  // Original receipt failure diagnostics: selected public metadata only.
  function originalReceiptFailureDiagnostic(owner,reason,reference,phase){
   const read=(object,key)=>{try{return object?.[key];}catch{return undefined;}};
   const token=value=>typeof value==='string'&&/^[a-zA-Z0-9_.:-]{1,256}$/.test(value)?value:null;
   const kind=value=>typeof value==='string'&&/^[a-z][a-z0-9_-]{0,95}$/.test(value)?value:null;
   const problem=read(reason,'problem'),scope=read(reference,'scope'),code=read(problem,'code');
   return {profile:'connect/original-receipt-failure/1',receipt_owner:owner,
    kind:kind(read(reason,'kind')),certainty:kind(read(reason,'certainty')),
    problem:{code:Number.isInteger(code)&&code>=100&&code<=599?code:null,
     category:kind(read(problem,'category')),reason:kind(read(problem,'reason'))},
    operation:{operation_id:token(read(reference,'operation_id')),command:token(read(reference,'command')),
     scope:{conversation_id:token(read(scope,'conversation_id')),session_id:token(read(scope,'session_id')),connection_id:token(read(scope,'connection_id'))}},
    last_observed_record_id:token(read(phase,'record_id'))};
  }
  async function reportOriginalReceiptFailures(receipts,reference,phase){
   for(const [index,receipt]of receipts.entries())if(receipt.status==='rejected'){
    try{await globalThis.originalReceiptFailure(originalReceiptFailureDiagnostic(
     index===0?'source-operation-wait':'target-prepared-completion',receipt.reason,reference,phase));}catch{}
   }
  }
  function originalContinuationFailureDiagnostic(type,operationId,scope,reason){
   const read=(object,key)=>{try{return object?.[key];}catch{return undefined;}};
   const token=value=>typeof value==='string'&&/^[a-zA-Z0-9_.:-]{1,256}$/.test(value)?value:null;
   const code=read(reason,'code');
   const receipt=originalReceiptFailureDiagnostic('original-request-continuation',reason,
    {operation_id:operationId,command:type,scope:{conversation_id:read(scope,'cid'),session_id:read(scope,'sid'),connection_id:read(scope,'connid')}},null);
   return {profile:'connect/original-continuation-failure/1',request_type:type==='operation.get'||type==='connection.offer'?type:null,
    code:typeof code==='string'&&/^[a-z][a-z0-9_-]{0,95}$/.test(code)?code:null,
    problem:receipt.problem,scope:{operation_id:token(operationId),cid:receipt.operation.scope.conversation_id,
     sid:receipt.operation.scope.session_id,connid:receipt.operation.scope.connection_id}};
  }
  function observeOriginalContinuation(work,type,operationId,scope){
   void work.then(observeReply,reason=>{
    try{void Promise.resolve(globalThis.originalReceiptFailure(
     originalContinuationFailureDiagnostic(type,operationId,scope,reason))).catch(()=>{});}catch{}
   });
   return work;
  }
  // End original receipt failure diagnostics.
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
  let observer,functionalFailure,cleanupFailures=[],measuring=false;
  const continuation=BrowserClient.prototype.requestReplacementContinuation;
  const NativePeer=globalThis.RTCPeerConnection,begin=BrowserRtcConnection.begin,beginReplacement=BrowserRtcConnection.beginReplacement;
  const nativeFrame=HTMLVideoElement.prototype.requestVideoFrameCallback,nativeDraw=CanvasRenderingContext2D.prototype.drawImage;
  const peers=new Map(),decoded=new Map(),warm=new Map(),phases=new Map(),publicMarkers=new Map(),publicDraws=new Map(),renderTargets=new Map(),publicReadyCalls=new Map(),privateReadyCalls=new Map();
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
   const original=()=>{const work=continuation.call(this,type,payload,scope,options);return observeOriginalContinuation(work,type,type==='operation.get'?payload.operation_id:scope.operation_id,scope);};
   if(type!=='connection.offer')return original();
   if(profile==='connect-replacement-ready-v1'){
    privateReadyCalls.set(scope.operation_id,(privateReadyCalls.get(scope.operation_id)??0)+1);
    const evidence=marker.readiness,old=ready.get(evidence.audio.stream_id);if(old)check(JSON.stringify(old)===JSON.stringify(evidence),'same original private marker replay');else ready.set(evidence.audio.stream_id,structuredClone(evidence));
   }
   if(profile!=='connect-replacement-public-video-ready-v1')return original();
   publicReadyCalls.set(scope.operation_id,(publicReadyCalls.get(scope.operation_id)??0)+1);
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
    const work=continuation.call(this,type,payload,scope,{...options,timeoutMs:remaining});return observeOriginalContinuation(work,type,scope.operation_id,scope);
   });
  };
  const invalid=message=>{if(measuring&&observationFailures.length<16)observationFailures.push(String(message).slice(0,160));};
  const retainPcm=async stage=>{try{const at=performance.now();await globalThis.retainPublicPcmDiagnostics(stage,{format:'connect/owned-public-pcm-diagnostics/1',pcm_measurement:pcmLedger.report(),fixture:'browser-public-video-roundtrip',operation_id:spec.operationId,at,render_origins:[...renderTimes].map(([connection_id,value])=>({connection_id,...value})),audio_failures:audioFailures,observation_failures:observationFailures,receivers:[...progress].map(([connection_id,p])=>({connection_id,i:byConnection.get(connection_id),selected:active.get(byConnection.get(connection_id))===connection_id,audio:p.audio,video:p.video,changing:p.changing,lastAudio:p.lastAudio,lastVideo:p.lastVideo,audio_age_ms:p.lastAudio===null?null:at-p.lastAudio,video_age_ms:p.lastVideo===null?null:at-p.lastVideo})),current:current.map((value,i)=>({i,connection_id:value.connection.id})),moves:moves.map(value=>({operation_id:value.operation_id,record_id:value.record_id,source:value.source,target:value.target,generation:value.generation})),phases:[...phases.values()].map(value=>({operation_id:value.operation_id,record_id:value.record_id,state:value.state,phase:value.webrtc_public_video_phase?.phase,target_connection_id:value.webrtc_public_video_phase?.target_connection_id,target_generation:value.webrtc_public_video_phase?.target_generation})),audio_diagnostics:observer?.report()??null});}catch{}};
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
     pcmLedger.push({owner:id,clock,timestamp,at,identity,usable:identity.identified&&identity.rms>.03});
     const p=progress.get(id);p.audio=pcmLedger.availableSamples(id)/960;
     if(!(identity.rms>.03))return identity.source_identified;
     if(!identity.identified){
      if(audioFailures.length<8)audioFailures.push({id,timestamp,at,clock,expected_hz:sourceHz[expected[i]],render:renderTimes.get(id),
       receivedAudio:progress.get(id)?.audio,identity,boundary:pcmBoundary(samples)});
      invalid(`actual selected source tone id=${id.slice(0,32)} hz=${sourceHz[expected[i]]} rms=${identity.rms.toFixed(3)} powers=${identity.powers.map(power=>power.toExponential(3)).join('/')}`);return identity.source_identified;
     }
     p.lastAudio=at;return identity.source_identified;
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
   for(let phase=0;phase<2;phase++){
    const retainedBefore=[progress.get(current[1].connection.id).video,progress.get(current[3].connection.id).video];
    const outcomes=await Promise.allSettled([0,2].map(async i=>{
     const sid=Math.floor(i/2),before=current[i],retain=current[i+1],targetActor=phase===0?2:0;
     const targetEndpoint=phase===0?spec.targetEndpoint:spec.originalEndpoint;
     const operationId=`${spec.operationId}-${phase}-${sid}`,cutoff=performance.now()+8000,started=performance.now();
     const options=()=>{const timeoutMs=Math.floor(cutoff-performance.now());check(timeoutMs>0,'original8s movement deadline');return {timeoutMs};};
     const sourceSession=clients[before.actor].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[sid]);
     const targetSession=clients[targetActor].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[sid]);
     const view=await sourceSession.refresh(options()),source=await before.connection.refresh(options());
     check(source.generation===phase+1,'exact original source generation');
     const streams={audio:`browser-wire-${phase}-${sid}-audio`,send:`browser-wire-${phase}-${sid}-video`,receive:`browser-wire-${phase}-${sid}-incoming`};
     const operation=await before.connection.replaceDevice({retain:retain.connection.id,target:{endpoint:targetEndpoint},expectedTopologyRevision:view.topologyRevision,
      video:{publications:[{previous:before.send,replacement:streams.send}],receivers:[{previous:before.receive,replacement:streams.receive}]}},
      {...options(),operationId});
     const reference=operation.referenceJson();
     const prepared=await targetSession.connections.prepareReplacement({operation:operation.reference,audio:streams.audio,
      video:{send:streams.send,receive:streams.receive}},options());
     const capture=await startCapture(i,prepared.connection,options());
     const receipts=await Promise.allSettled([operation.wait(options()),prepared.completion]);
     if(receipts.some(receipt=>receipt.status==='rejected'))await reportOriginalReceiptFailures(receipts,reference,phases.get(operationId));
     for(const receipt of receipts)if(receipt.status==='rejected')throw receipt.reason;
     const a=receipts[0].value.value,b=receipts[1].value.value;
     for(const result of [a,b]){
      check(result.connection.id===prepared.connection.id&&result.activationState==='ready','both original receipts name one ready target');
      check(result.binding.generation===phase+2&&result.binding.deviceId===spec.actors[targetActor].device.id&&result.binding.endpointId===targetEndpoint,'exact new generation Device and Endpoint');
      check(result.retired.length===1&&result.retired[0]===before.connection.id&&result.retained.length===1&&result.retained[0].id===retain.connection.id,'exact old source retirement and unchanged retained peer');
     }
     const evidence=ready.get(streams.audio);check(evidence?.audio.decoded_samples>=960&&evidence.video?.stream_id===streams.send&&evidence.video.decoded_frames>0&&(evidence.video.decoded_keyframes===null||evidence.video.decoded_keyframes>0),'actual private audio/A decoder evidence, never B');
     const publicReady=publicMarkers.get(operationId),activated=phases.get(operationId);check(publicReady&&activated?.state==='completed'&&activated.webrtc_public_video_phase?.phase==='activated','actual original Completed+Activated, no legacy fallback');
     check(JSON.stringify(activated.webrtc_public_video_phase.binding)===JSON.stringify(publicReady.marker.public_video_readiness.binding),'same activated B native tuple');
     check(captures.find(value=>value.id===before.connection.id)?.owner.closed===true,'exact original capture retired by actual SDK receipt');
     check(capture.closed===false,'actual candidate capture remains live');
     const next={connection:prepared.connection,actor:targetActor,audio:streams.audio,send:streams.send,receive:streams.receive};
     let postTakeNativeFault=false;
     if(phase===0&&sid===0){
      const owned=warm.get(next.connection.id),element=owned.element,descriptor=Object.getOwnPropertyDescriptor(element,'requestVideoFrameCallback');
      const beforePublic=publicReadyCalls.get(operationId),beforePrivate=privateReadyCalls.get(operationId),beforeMarker=publicReady.marker;
      const failedCanvas=document.createElement('canvas');surfaces[i].outputs.push(failedCanvas);
      // Register the ACTUAL failed output before entering render, exactly as
      // startRender does, so even an erroneous synchronous draw is observed.
      renderTargets.set(failedCanvas,next.connection.id);const beforeDraws=publicDraws.get(next.connection.id)??0;
      let faultCalls=0,nativeFailure,refused=false;
      Object.defineProperty(element,'requestVideoFrameCallback',{configurable:true,writable:true,value:function(callback){
       if(faultCalls++===0){
        // Exact real B element, after the renderer has borrowed its completed
        // loan. The native WebIDL API itself refuses a noncallback argument;
        // no fake transport/Host error or readiness/phase is constructed.
        try{return nativeFrame.call(this,null);}catch(error){nativeFailure=error;throw error;}
       }
       return nativeFrame.call(this,callback);
      }});
      try{await next.connection.render({audio:true,video:[{stream:next.connection.streams.byId(next.receive),canvas:failedCanvas}]},options());}
      catch{refused=true;}
      finally{if(descriptor)Object.defineProperty(element,'requestVideoFrameCallback',descriptor);else delete element.requestVideoFrameCallback;}
      check(refused&&faultCalls===1&&nativeFailure instanceof TypeError,'actual native rVFC argument failure after completed B borrow');
      // This narrow source-order assertion applies only to the injected FIRST
      // native callback-registration failure: production paints only inside
      // paint after successful registration. It is not general render atomicity
      // or a claim that an already-authorized B picture would be a media leak.
      check((publicDraws.get(next.connection.id)??0)===beforeDraws,'first native callback registration refusal precedes any paint on this tracked failed target');
      check(owned.element===element&&owned.transceiver.receiver.track===owned.track&&owned.track.readyState==='live'&&element.srcObject?.getVideoTracks()[0]===owned.track&&owned.peer.connectionState==='connected','failed renderer returns same completed B element/track without detaching or closing physical owner');
      check(publicReadyCalls.get(operationId)===beforePublic&&privateReadyCalls.get(operationId)===beforePrivate&&publicMarkers.get(operationId).marker===beforeMarker,'post-take failure never resubmits private/public Ready');
      renderTargets.delete(failedCanvas);postTakeNativeFault=true;
     }
     await startRender(i,next,options());
     await wait(()=>fresh(next.connection.id,started),cutoff,'actual ready target renders fresh changing audio/video within original move budget');
     check((publicDraws.get(next.connection.id)??0)>0,'same warmed B element actually drawn');
     const retired=await before.connection.refresh(options());check(retired.state==='retired','original source retired in normal SDK snapshot');
     const retained=await retain.connection.refresh(options());check(retained.state==='ready'&&retained.generation===1,'original retained peer stays generation1');
     const status=await operation.status(options());check(status.state==='completed'&&status.recordId,'same original durable root completed');
     current[i]=next;
     return {operation_id:operationId,record_id:status.recordId,source:before.connection.id,target:next.connection.id,
      retained:retain.connection.id,source_actor:before.actor,target_actor:targetActor,generation:phase+2,
      source_generation:phase+1,endpoint_id:targetEndpoint,device_id:spec.actors[targetActor].device.id,
      post_take_native_fault_same_lease_retry:postTakeNativeFault,readiness:evidence,public_ready:publicReady.marker,activated:activated.webrtc_public_video_phase,same_warmed_element:publicDraws.get(next.connection.id)>0,same_native_receiver:warm.get(next.connection.id).transceiver.receiver.track===warm.get(next.connection.id).track,reference,elapsed_ms:performance.now()-started,public_output:{audio:progress.get(next.connection.id).audio,video:progress.get(next.connection.id).video}};
    }));
    for(const outcome of outcomes)if(outcome.status==='rejected')throw outcome.reason;
    moves.push(...outcomes.map(value=>value.value));
    for(let sid=0;sid<2;sid++)check(progress.get(current[sid*2+1].connection.id).video>retainedBefore[sid],'retained decoder advances during actual handoff');
    const after=performance.now();await wait(()=>current.every(value=>fresh(value.connection.id,after)),after+2000,'all four current receivers fresh after phase');
    for(let actor=0;actor<3;actor++){
     check(clients[actor].transport.physicalEpoch===epochs[actor],'same original shared WSS client throughout');
     const room=await clients[actor].conversations.byId(spec.conversationId).refresh(opts());check(room.id===spec.conversationId,'actual unchanged control remains usable');
    }
   }
   check(moves.length===4&&moves.filter(value=>value.generation===3).length===2,'two genuine original-Device returns');
   check(observationFailures.length===0,`no wrong source or malformed actual decoded output: ${observationFailures.join(', ')} details=${JSON.stringify(audioFailures)}`);
   const report={operation_id:spec.operationId,moves,original_connections:oldViews.map(value=>value.id),current:current.map(value=>value.connection.id),
    wss_epochs:epochs,original_device_id:spec.actors[0].device.id,target_device_id:spec.actors[2].device.id,
    active_receivers:current.map(value=>({id:value.connection.id,...progress.get(value.connection.id),lastPixels:undefined}))};
   await globalThis.terminalCheckpoint('roundtrip-complete',report,8000);
   return {format:spec.format,passed:true,...report};
  }catch(error){functionalFailure=error;await retainPcm('failure');throw error;}
  finally{
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
   peers.clear();decoded.clear();warm.clear();phases.clear();publicMarkers.clear();publicDraws.clear();renderTargets.clear();publicReadyCalls.clear();privateReadyCalls.clear();
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
