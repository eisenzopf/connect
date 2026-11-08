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
assert.equal(spec.format,'connect/chromium-original-roundtrip-integration/1');
assert.equal(spec.operationId,'browser-wire-roundtrip');
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
  assert.ok(stage==='roundtrip-complete');
  assert.ok(Number.isFinite(remainingMs)&&remainingMs>0&&remainingMs<=8000);
  assert.equal(proof.operation_id,spec.operationId);
  const event=path.join(privateRoot,stage+'.json'),ack=path.join(privateRoot,stage+'.ack.json');
  const pending=event+'.pending',file=await fs.open(pending,'wx',0o600);
  try{await file.writeFile(JSON.stringify(proof));await file.sync();}finally{await file.close();}
  await fs.rename(pending,event);
  const until=performance.now()+remainingMs;
  for(;;){
   try{const value=JSON.parse(await fs.readFile(ack,'utf8'));assert.equal(value.operation_id,spec.operationId);assert.equal(value.stage,stage);return;}
   catch(error){if(error.code!=='ENOENT')throw error;}
   assert.ok(performance.now()<until,'same original fixture checkpoint deadline');
   await new Promise(resolve=>setTimeout(resolve,5));
  }
 });
 await page.goto(`http://127.0.0.1:${spec.assetPort}/`,{waitUntil:'domcontentloaded'});
 result=await page.evaluate(async spec=>{
  const {BrowserSdkClient}=await import('/sdk/browser_sdk.js');
  const {BrowserRtcConnection}=await import('/sdk/browser_sdk_webrtc.js');
  const {CanonicalWebRtcSignaling}=await import('/sdk/browser_webrtc_signaling.js');
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
  const audio=new AudioContext({sampleRate:48000});await audio.resume();
  let observer,functionalFailure,cleanupFailures=[],measuring=false;
  const complete=CanonicalWebRtcSignaling.prototype.completeReplacement;
  // Passive copy of the actual SDK decoder evidence immediately before its
  // unchanged signed continuation. This wrapper never creates or edits Ready.
  CanonicalWebRtcSignaling.prototype.completeReplacement=function(evidence,...args){
   check(!ready.has(evidence.audio.stream_id),'one original decoded readiness per target');
   ready.set(evidence.audio.stream_id,structuredClone(evidence));return complete.call(this,evidence,...args);
  };
  const invalid=message=>{if(measuring&&observationFailures.length<16)observationFailures.push(String(message).slice(0,160));};
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
    audio(id,timestamp,at,samples){
     const i=byConnection.get(id);if(i===undefined||active.get(i)!==id)return;
     if(samples.length!==960){invalid('actual20ms decoded PCM');return;}
     const identity=pcmIdentity(samples,expected[i]);
     if(!(identity.rms>.03))return;
     if(!identity.identified){
      if(audioFailures.length<8)audioFailures.push({id,at,render:renderTimes.get(id),
       receivedAudio:progress.get(id)?.audio,identity,boundary:pcmBoundary(samples)});
      invalid(`actual selected source tone id=${id.slice(0,32)} hz=${sourceHz[expected[i]]} rms=${identity.rms.toFixed(3)} powers=${identity.powers.map(power=>power.toExponential(3)).join('/')}`);return;
     }
     const p=progress.get(id);p.audio++;p.lastAudio=at;
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
     for(const receipt of receipts)if(receipt.status==='rejected')throw receipt.reason;
     const a=receipts[0].value.value,b=receipts[1].value.value;
     for(const result of [a,b]){
      check(result.connection.id===prepared.connection.id&&result.activationState==='ready','both original receipts name one ready target');
      check(result.binding.generation===phase+2&&result.binding.deviceId===spec.actors[targetActor].device.id&&result.binding.endpointId===targetEndpoint,'exact new generation Device and Endpoint');
      check(result.retired.length===1&&result.retired[0]===before.connection.id&&result.retained.length===1&&result.retained[0].id===retain.connection.id,'exact old source retirement and unchanged retained peer');
     }
     const evidence=ready.get(streams.audio);check(evidence?.audio.decoded_samples>=960&&evidence.video?.stream_id===streams.receive&&evidence.video.decoded_frames>0&&evidence.video.decoded_keyframes>0&&evidence.video.decoder_startup==='fresh-peer-first-decoded-frame','actual SDK fresh candidate decoded audio and keyframe');
     check(captures.find(value=>value.id===before.connection.id)?.owner.closed===true,'exact original capture retired by actual SDK receipt');
     check(capture.closed===false,'actual candidate capture remains live');
     const next={connection:prepared.connection,actor:targetActor,audio:streams.audio,send:streams.send,receive:streams.receive};
     await startRender(i,next,options());
     await wait(()=>fresh(next.connection.id,started),cutoff,'actual ready target renders fresh changing audio/video within original move budget');
     const retired=await before.connection.refresh(options());check(retired.state==='retired','original source retired in normal SDK snapshot');
     const retained=await retain.connection.refresh(options());check(retained.state==='ready'&&retained.generation===1,'original retained peer stays generation1');
     const status=await operation.status(options());check(status.state==='completed'&&status.recordId,'same original durable root completed');
     current[i]=next;
     return {operation_id:operationId,record_id:status.recordId,source:before.connection.id,target:next.connection.id,
      retained:retain.connection.id,source_actor:before.actor,target_actor:targetActor,generation:phase+2,
      source_generation:phase+1,endpoint_id:targetEndpoint,device_id:spec.actors[targetActor].device.id,
      readiness:evidence,reference,elapsed_ms:performance.now()-started,public_output:{audio:progress.get(next.connection.id).audio,video:progress.get(next.connection.id).video}};
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
  }catch(error){functionalFailure=error;throw error;}
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
   if(!functionalFailure&&clients[0])for(let sid=0;sid<2;sid++){
    try{const session=clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[sid]),view=await session.refresh(opts());
     await done(session.end({reasonCode:0,reason:'original roundtrip cleanup',expectedTopologyRevision:view.topologyRevision},mutation(`browser-wire-session-end-${sid}`)));}
    catch{cleanupFailures.push('session-end');}
   }
   for(const client of clients){try{await client.close(opts());}catch{cleanupFailures.push('sdk-close');}}
   CanonicalWebRtcSignaling.prototype.completeReplacement=complete;ready.clear();byConnection.clear();active.clear();progress.clear();
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
