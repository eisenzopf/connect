/** Owned mixed Native→RTC target. Native source SDK owns the original Root. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {webcrypto} from 'node:crypto';
import {pathToFileURL} from 'node:url';
import {closeOwnedChromium} from '../lib/browser_owned_chromium.mjs';
let text='';for await(const part of process.stdin){text+=part;assert.ok(text.length<65536);}
const spec=JSON.parse(text);assert.equal(spec.format,'connect/native-rtc-public-b-target/1');
const root=await fs.realpath(spec.root),directory=await fs.realpath(spec.directory),sdk=await fs.realpath(path.join(root,'sdk/typescript/dist'));
assert.ok(directory.startsWith(root+path.sep)||directory.startsWith('/private/')||directory.startsWith('/tmp/'));
const actor=spec.actor;assert.equal(new URL(actor.url).protocol,'wss:');
const algorithm=actor.server.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
actor.server.jwk=await webcrypto.subtle.exportKey('jwk',await webcrypto.subtle.importKey('raw',Buffer.from(actor.server.publicRaw,'base64url'),algorithm,true,['verify']));delete actor.server.publicRaw;
const emit=async(name,value)=>{const file=path.join(directory,name+'.pending');await fs.writeFile(file,JSON.stringify(value),{flag:'wx',mode:0o600});await fs.rename(file,path.join(directory,name+'.json'));};
const read=async(name,until)=>{for(;;){try{return JSON.parse(await fs.readFile(path.join(directory,name+'.json'),'utf8'));}catch(error){if(error.code!=='ENOENT')throw error;}assert.ok(performance.now()<until,'original owned IPC cutoff');await new Promise(r=>setTimeout(r,5));}};
const assets=http.createServer(async(req,res)=>{try{assert.equal(req.method,'GET');const url=new URL(req.url,'http://127.0.0.1');let file;
 if(url.pathname==='/'){res.writeHead(200,{'Content-Type':'text/html'});res.end('<!doctype html><meta charset="utf-8"><title>Original mixed B target</title>');return;}
 if(url.pathname==='/observer.mjs')file=path.join(root,'scripts/lib/browser_matrix_rtc_observation.mjs');
 else if(url.pathname==='/browser_matrix_rtc_audio_diagnostics.mjs')file=path.join(root,'scripts/lib/browser_matrix_rtc_audio_diagnostics.mjs');
 else if(url.pathname==='/pcm-identity.mjs')file=path.join(root,'scripts/lib/browser_matrix_pcm.mjs');
 else{assert.ok(url.pathname.startsWith('/sdk/'));file=await fs.realpath(path.resolve(sdk,'.'+url.pathname.slice(4)));assert.ok(file.startsWith(sdk+path.sep)&&file.endsWith('.js'));}
 res.writeHead(200,{'Content-Type':'text/javascript'});res.end(await fs.readFile(file));}catch{res.writeHead(404);res.end('unavailable');}});
await new Promise((resolve,reject)=>{assets.once('error',reject);assets.listen(spec.assetPort,'127.0.0.1',resolve);});
const {chromium}=await import(process.env.CONNECT_PLAYWRIGHT_MODULE?pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href:'playwright');
let browser,context,result,failed;
try{
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${spec.debugPort}`);context=await browser.newContext();const page=await context.newPage();
 await page.exposeFunction('mixedReady',()=>emit('browser-ready',{operation_id:spec.operationId}));
 await page.exposeFunction('mixedRoot',async()=>{const value=await read('original-root',performance.now()+30000);assert.equal(value.operation_id,spec.operationId);const remainingMs=Math.floor(value.expires_at_ms-Date.now());assert.ok(remainingMs>0&&remainingMs<=8000);return {...value,remainingMs};});
 await page.exposeFunction('mixedCheckpoint',async(name,value,remainingMs)=>{assert.ok(['before-public-ready','public-rendered'].includes(name));assert.equal(value.operation_id,spec.operationId);assert.ok(remainingMs>0&&remainingMs<=8000);await emit(name,value);const ack=await read(name+'.ack',performance.now()+remainingMs);assert.equal(ack.operation_id,spec.operationId);});
 await page.exposeFunction('retainPublicPcmDiagnostics',async(stage,value)=>{
  assert.ok(['failure','closed'].includes(stage));const text=JSON.stringify(value);assert.ok(text.length<=2*1024*1024,'bounded original public PCM diagnostics');
  await fs.writeFile(path.join(directory,'rtc-audio-'+stage+'.json'),text,{flag:'wx',mode:0o600});
 });
 await page.goto(`http://127.0.0.1:${spec.assetPort}/`);
 result=await page.evaluate(async spec=>{
  const {BrowserSdkClient}=await import('/sdk/browser_sdk.js');const {BrowserClient}=await import('/sdk/browser_client.js');
  const {BrowserRtcConnection}=await import('/sdk/browser_sdk_webrtc.js');const {browserMatrixRtcObservation}=await import('/observer.mjs');const {strictPcmIdentity:pcmIdentity,PcmSampleLedger}=await import('/pcm-identity.mjs');const pcmLedger=new PcmSampleLedger();
  globalThis.connectRtcAudioFacts=true;
  const pcmOrigins=new Map(),pcmFailures=[];
  const check=(v,m)=>{if(!v)throw Error(m);};const NativePeer=RTCPeerConnection,nativeFrame=HTMLVideoElement.prototype.requestVideoFrameCallback,nativeDraw=CanvasRenderingContext2D.prototype.drawImage;
  const begin=BrowserRtcConnection.beginReplacement,continuation=BrowserClient.prototype.requestReplacementContinuation;
  let arming=null,peer,peerId,latest,warmed,publicCanvas,publicDraws=0,callbacks=0,client,capture,renderer,observer,audio,oscillator,timer,primary,measurement;
  let frozenPublicMarker,publicMarkerGate;
  const decoded=new Map(),failures=[];let active=false,audioSamples=0,videoFrames=0,lastVideo=0,lastAudio=0,changingVideo=0,previousTexture,minAudioExplainedEnergy=Infinity;
  globalThis.RTCPeerConnection=class extends NativePeer{constructor(...args){super(...args);check(arming&&!peer,'one exact original native B target peer');peer=this;peerId=arming;}};
  BrowserRtcConnection.beginReplacement=function(...args){check(!arming,'same synchronous original constructor');arming=args[1].connectionId;try{return begin.apply(this,args);}finally{arming=null;}};
  HTMLVideoElement.prototype.requestVideoFrameCallback=function(callback){const element=this;return nativeFrame.call(element,function(now,metadata){try{const tracks=element.srcObject instanceof MediaStream?element.srcObject.getVideoTracks():[];if(tracks.length===1){const track=tracks[0],found=peer?.getTransceivers().filter(t=>t.receiver.track===track)??[];check(found.length===1,'actual original receiver per decoded element');let old=decoded.get(track);if(!old){old={element,track,transceiver:found[0],count:0};decoded.set(track,old);}check(old.element===element&&old.transceiver===found[0],'same original decoder element');old.count++;callbacks++;}}catch(e){failures.push(String(e.message));}return callback.call(element,now,metadata);});};
  CanvasRenderingContext2D.prototype.drawImage=function(source,...args){if(this.canvas===publicCanvas){try{check(warmed&&source===warmed.element&&warmed.transceiver.receiver.track===warmed.track,'SAME warmed B native element is publicly adopted');publicDraws++;}catch(e){failures.push(String(e.message));}}return nativeDraw.call(this,source,...args);};
  const observe=frames=>{for(const frame of frames)if(frame.type==='operation.updated'&&frame.payload?.webrtc_public_video_phase)latest=structuredClone(frame.payload);};
  BrowserClient.prototype.requestReplacementContinuation=function(type,payload,scope,options){const original=()=>{const promise=continuation.call(this,type,payload,scope,options);void promise.then(observe,()=>{});return promise;};
   if(type!=='connection.offer'||payload.replacement_preparation?.profile!=='connect-replacement-public-video-ready-v1')return original();
   const marker=payload.replacement_preparation,bytes=JSON.stringify(marker),cutoff=performance.now()+options.timeoutMs;
   const forward=()=>{const remaining=Math.floor(cutoff-performance.now());check(remaining>0&&!options.signal?.aborted,'SAME Ready send budget');check(warmed&&warmed.transceiver.receiver.track===warmed.track&&warmed.track.readyState==='live'&&decoded.get(warmed.track)===warmed,'same B survives checkpoint/replay');const promise=continuation.call(this,type,payload,scope,{...options,timeoutMs:remaining});void promise.then(observe,()=>{});return promise;};
   if(frozenPublicMarker){check(bytes===frozenPublicMarker,'SAME frozen public marker on recovery, no new checkpoint/observation');return publicMarkerGate.then(forward);}
   const phase=latest?.webrtc_public_video_phase;
   check(latest?.state==='preparing'&&phase?.phase==='cut-bound-post-f'&&phase.operation_id===spec.operationId,'actual verified original CutBound phase');
   check(peer&&peerId===scope.connid&&peer.getTransceivers().length===3,'actual original audio+A+B peer');
   const found=peer.getTransceivers().filter(t=>t.mid===phase.binding.mid);check(found.length===1,'actual B MID binding');const t=found[0],value=decoded.get(t.receiver.track);
   check(t.direction==='recvonly'&&t.currentDirection==='recvonly'&&t.sender.track===null&&value?.count>0,'actual B decoded before Ready');
   check(peer.getTransceivers().filter(other=>other!==t).every(other=>other.direction==='sendrecv'&&other.currentDirection==='sendrecv'&&other.sender.track?.readyState==='live'),'original camera/A and audio duplex remain live');
   check(JSON.stringify(marker.public_video_readiness.binding)===JSON.stringify(phase.binding),'exact native B readiness tuple');warmed=value;frozenPublicMarker=bytes;
   publicMarkerGate=globalThis.mixedCheckpoint('before-public-ready',{operation_id:spec.operationId,record_id:marker.operation_record_id,target:scope.connid,marker,phase,native:{mid:t.mid,track_id:value.track.id,callbacks:value.count}},options.timeoutMs);
   return publicMarkerGate.then(forward);
  };
  // Whitelisted public values only; this observes the existing failure and
  // existing wire snapshot, with no operation.get or other recovery request.
  function nativeFailureDiagnostic(reason,snapshot){
   const read=(object,key)=>{try{return object?.[key];}catch{return undefined;}};
   const token=value=>typeof value==='string'&&/^[a-zA-Z0-9_.:-]{1,256}$/.test(value)?value:null;
   const kind=value=>typeof value==='string'&&/^[a-z][a-z0-9_-]{0,95}$/.test(value)?value:null;
   const problem=value=>{const code=read(value,'code');return {code:Number.isInteger(code)&&code>=100&&code<=599?code:kind(code),
    source:kind(read(value,'source')),category:kind(read(value,'category')),reason:kind(read(value,'reason'))};};
   const reference=read(reason,'operation'),scope=read(reference,'scope'),phase=read(snapshot,'webrtc_public_video_phase'),revision=read(snapshot,'revision');
   return {profile:'connect/native-original-operation-failure/1',
    primary:{kind:kind(read(reason,'kind')),certainty:kind(read(reason,'certainty')),problem:problem(read(reason,'problem')),
     reference:{operation_id:token(read(reference,'operation_id')),command:token(read(reference,'command')),
      scope:{conversation_id:token(read(scope,'conversation_id')),session_id:token(read(scope,'session_id')),connection_id:token(read(scope,'connection_id'))}}},
    latest_operation:{operation_id:token(read(snapshot,'operation_id')),record_id:token(read(snapshot,'record_id')),
     state:kind(read(snapshot,'state')),revision:Number.isSafeInteger(revision)?revision:null,
     problem:problem(read(snapshot,'problem')),phase:kind(read(phase,'phase'))}};
  }
  const retainPcm=async stage=>{try{const at=performance.now();await globalThis.retainPublicPcmDiagnostics(stage,{format:'connect/owned-public-pcm-diagnostics/1',pcm_measurement:pcmLedger.report(),fixture:'browser-native-public-b-target',operation_id:spec.operationId,at,render_origins:[...pcmOrigins].map(([connection_id,value])=>({connection_id,...value})),audio_failures:pcmFailures,observation_failures:failures,operation_failure:nativeFailureDiagnostic(primary,latest),receivers:[{connection_id:peerId,audio_samples:audioSamples,video:videoFrames,changing:changingVideo,lastAudio,lastVideo}],phase:latest?{operation_id:latest.operation_id,record_id:latest.record_id,state:latest.state,phase:latest.webrtc_public_video_phase?.phase}:null,audio_diagnostics:observer?.report()??null});}catch{}};
  try{
   const a=spec.actor,algorithm=a.signing.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
   const key=await crypto.subtle.importKey('jwk',a.signing.jwk,algorithm,false,['sign']),trust=await crypto.subtle.importKey('jwk',a.server.jwk,a.server.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'},false,['verify']);
   const enrollment={credential:a.credential,identityId:a.identityId,device:a.device,signing:{key,keyId:a.signing.keyId,algorithm:a.signing.algorithm},server:{key:trust,keyId:a.server.keyId,algorithm:a.server.algorithm,notBefore:a.server.notBefore,expiresAt:a.server.expiresAt}};
   client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:a.identityId,deviceId:a.device.id,endpoints:{'wss-webrtc':a.url},preferredTransport:'wss-webrtc',allowFallback:false,timeoutMs:8000,attemptTimeoutMs:3000,resolveEnrollment:async()=>enrollment});
   const toneIndex=[440,660,880,1100].indexOf(spec.expectedAudioHz);check(toneIndex>=0&&spec.expectedAudioHz===660&&spec.expectedVideoLuma===115,'declared actual Native programme source3 media identity');
   // Mixed-local competitors include the ACTUAL original Native source0
   // programme at330Hz, not only the shared four-tone matrix bank. This is
   // direct measured PCM projection; no generated signal enters the observer.
   const measuredPower=(samples,hz)=>{const coefficient=2*Math.cos(2*Math.PI*hz/48000);let a=0,b=0;for(const sample of samples){const value=sample+coefficient*a-b;b=a;a=value;}return (a*a+b*b-coefficient*a*b)/samples.length**2;};
   const expectedGray=(spec.expectedVideoLuma-16)*255/219;measurement=document.createElement('canvas');measurement.width=32;measurement.height=8;
   const originalEpoch=client.transport.physicalEpoch;check(Number.isSafeInteger(originalEpoch)&&originalEpoch>0,'actual original WSS control epoch');
   await globalThis.mixedReady();const before=performance.now(),input=await globalThis.mixedRoot(),cutoff=before+input.remainingMs;
   const opts=()=>{const timeoutMs=Math.floor(cutoff-performance.now());check(timeoutMs>0,'original Native source command cutoff');return {timeoutMs};};
   // Restore the original Rust SDK's authoritative serialized reference using
   // the public typed API. This is observation/target preparation, no new Root.
   const operation=client.operations.restore(input.reference,client.operationKinds.connectionReplace);
   const session=client.conversations.byId(input.conversation_id).sessions.byId(input.session_id);
   const prepared=await session.connections.prepareReplacement({operation:operation.reference,audio:'mixed-target-audio',video:{send:'mixed-target-video',receive:'mixed-target-incoming'}},opts());
   audio=new AudioContext({sampleRate:48000});await audio.resume();oscillator=audio.createOscillator();const gain=audio.createGain();oscillator.frequency.value=330;gain.gain.value=.15;oscillator.connect(gain);oscillator.start();
   const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;let ordinal=0;const draw=()=>{const c=canvas.getContext('2d');c.fillStyle='rgb(28,28,28)';c.fillRect(0,0,640,360);c.fillStyle='white';c.fillRect(40+(ordinal++*13)%540,100,20,100);};draw();timer=setInterval(draw,1000/30);
   capture=await prepared.connection.capture({audio:true,audioNode:gain,video:'camera',videoCanvas:canvas},opts());
   const receipts=await Promise.allSettled([operation.wait(opts()),prepared.completion]);for(const r of receipts)if(r.status==='rejected')throw r.reason;
   const value=receipts[1].value.value;check(value.connection.id===prepared.connection.id&&value.binding.generation===2&&value.retired[0]===input.source,'same original native→RTC replacement completes');
   check(latest?.state==='completed'&&latest.webrtc_public_video_phase?.phase==='activated'&&warmed,'actual Completed+Activated');
   publicCanvas=document.createElement('canvas');publicCanvas.width=640;publicCanvas.height=360;
   observer=browserMatrixRtcObservation(BrowserRtcConnection,{invalid(...args){if(failures.length<16)failures.push(String(args.at(-1)).slice(0,160));},measurementOrigin(id){return active&&id===peerId?pcmOrigins.get(id)?.started:null;},audio(id,ts,at,samples,clock){if(active&&id===prepared.connection.id){
    check(samples.length===960,'actual20ms PCM');const identity=pcmIdentity(samples,toneIndex,{competitors:[330]});
    const expectedPower=identity.powers[toneIndex],legacyNative330Power=measuredPower(samples,330),legacyBrowser440Power=measuredPower(samples,440),
     native330Power=identity.finite_support.used?identity.extra_powers.find(value=>value.hz===330).power:legacyNative330Power,
     browser440Power=identity.finite_support.used?identity.powers[0]:legacyBrowser440Power;
    // A sinusoid's mean-square contribution is twice its complex-bin power.
    // Require expected660 to explain at least half of TOTAL measured energy,
    // in addition to the unchanged RMS/shared-bank identity and8x dominance.
    const explainedEnergy=identity.expected_explained_energy_fraction;
    const nativeQualified=identity.identified&&expectedPower>8*Math.max(native330Power,browser440Power,1e-12)&&explainedEnergy>=.5;
    pcmLedger.push({owner:id,clock,timestamp:ts,at,identity,usable:nativeQualified&&identity.rms>.03});audioSamples=pcmLedger.availableSamples(id);
    if(!(identity.rms>.03))return identity.source_identified;
    if(!identity.identified||!(expectedPower>8*Math.max(native330Power,browser440Power,1e-12))||!(explainedEnergy>=.5)){if(pcmFailures.length<8)pcmFailures.push({id,timestamp:ts,at,clock,expected_hz:spec.expectedAudioHz,identity,native330Power,browser440Power,legacyNative330Power,legacyBrowser440Power,explainedEnergy});if(failures.length<16)failures.push('audible public PCM failed actual330/440 competitors or total-energy source3/660Hz identity');return false;}
    minAudioExplainedEnergy=Math.min(minAudioExplainedEnergy,explainedEnergy);lastAudio=at;return identity.source_identified;
   }},video(id,ts,at,frame,w,h){if(active&&id===prepared.connection.id){
    check(w===640&&h===360,'actual VP8 size');const ctx=measurement.getContext('2d');
    // SAME already painted public bitmap. Native mixed fixture's selected
    // original I420 inputs use neutral chroma; no pixel identity is inferred
    // from RTP/counters or filled from expected metadata.
    ctx.drawImage(frame,0,0,32,32,0,0,8,8);ctx.drawImage(frame,32,0,608,64,8,0,24,8);
    const pixels=ctx.getImageData(0,0,32,8).data,means=[0,0,0];
    for(let y=2;y<6;y++)for(let x=2;x<6;x++){const n=(y*32+x)*4;check(pixels[n+3]===255,'actual opaque source marker');for(let c=0;c<3;c++)means[c]+=pixels[n+c]/16;}
    if(means.some(value=>Math.abs(value-expectedGray)>=12)){if(failures.length<16)failures.push('public pixels did not identify actual Native source3 marker');return;}
    const texture=[];for(let y=0;y<8;y++)for(let x=8;x<32;x++){const n=(y*32+x)*4;texture.push(pixels[n],pixels[n+1],pixels[n+2]);}
    if(previousTexture&&texture.some((value,index)=>value!==previousTexture[index]))changingVideo++;previousTexture=texture;
    videoFrames++;lastVideo=at;
   }}});
   observer.attach(prepared.connection.id,publicCanvas);active=true;
   const timeline={started:performance.now(),returned:null};pcmOrigins.set(prepared.connection.id,timeline);
   renderer=await prepared.connection.render({audio:true,video:[{stream:prepared.connection.streams.byId('mixed-target-incoming'),canvas:publicCanvas}]},opts());
   timeline.returned=performance.now();
   const adoptedAt=performance.now();while(!(publicDraws>=5&&videoFrames>=5&&changingVideo>=3&&audioSamples>=4800&&lastVideo>=adoptedAt&&lastAudio>=adoptedAt)){check(performance.now()<cutoff,'actual public B and audio advance under SAME Root budget');await new Promise(r=>setTimeout(r,5));}
   check(failures.length===0,'original native ownership errors: '+failures.join(';'));
   check(client.transport.physicalEpoch===originalEpoch,'SAME original target control socket remains current');
   await client.conversations.byId(input.conversation_id).refresh(opts());
   const report={operation_id:spec.operationId,target:prepared.connection.id,source:input.source,activated:latest.webrtc_public_video_phase,same_warmed_element:true,native_mid:warmed.transceiver.mid,decoded_callbacks:callbacks,public_draws:publicDraws,video_frames:videoFrames,changing_video_frames:changingVideo,audio_samples:audioSamples,audio_identity_hz:spec.expectedAudioHz,audio_source_identity_profile:'connect/decoded-tone-recognition/2',audio_sample_measurement:pcmLedger.report(),audio_min_explained_energy_fraction:minAudioExplainedEnergy,video_identity_luma:spec.expectedVideoLuma};
   await globalThis.mixedCheckpoint('public-rendered',report,opts().timeoutMs);return {passed:true,...report};
  }catch(error){primary=error;await retainPcm('failure');throw error;}
  finally{
   if(timer)clearInterval(timer);
   try{capture?.close();}catch(e){failures.push(String(e.message));}
   try{renderer?.close();}catch(e){failures.push(String(e.message));}
   if(renderer){try{await renderer.retirement;await renderer.finished;}catch(e){failures.push(String(e.message));}}
   if(observer){try{await observer.close();}catch(e){failures.push(String(e.message));}}
   await retainPcm('closed');
   if(oscillator){try{oscillator.stop();oscillator.disconnect();}catch(e){failures.push(String(e.message));}}
   if(audio){try{await audio.close();}catch(e){failures.push(String(e.message));}}
   if(client){try{await client.close({timeoutMs:8000});}catch(e){failures.push(String(e.message));}}
   BrowserClient.prototype.requestReplacementContinuation=continuation;BrowserRtcConnection.beginReplacement=begin;globalThis.RTCPeerConnection=NativePeer;HTMLVideoElement.prototype.requestVideoFrameCallback=nativeFrame;CanvasRenderingContext2D.prototype.drawImage=nativeDraw;
   decoded.clear();warmed=undefined;latest=undefined;frozenPublicMarker=undefined;publicMarkerGate=undefined;previousTexture=undefined;if(measurement){measurement.width=0;measurement.height=0;}if(publicCanvas){publicCanvas.width=0;publicCanvas.height=0;}if(!primary)check(peer?.connectionState==='closed'&&failures.length===0,'same native target closed after all local owners');
  }
 },spec);
}catch(error){failed=error;throw error;}
finally{const cleanup=await closeOwnedChromium({browser,context,assets});if(!failed&&cleanup.failures.length)throw new AggregateError(cleanup.failures.map(x=>Error(x.phase+': '+x.name)));}
process.stdout.write(JSON.stringify(result)+'\n');
