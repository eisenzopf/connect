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
assert.equal(spec.format,'connect/chromium-original-terminal-integration/1');
assert.equal(spec.operationId,'browser-wire-original-move');
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
const assets=http.createServer(async(request,response)=>{
 try{
  assert.equal(request.method,'GET');const url=new URL(request.url,'http://127.0.0.1');
  if(url.pathname==='/'){response.writeHead(200,{'Content-Type':'text/html'});response.end('<!doctype html><meta charset=utf-8><title>Owned terminal integration</title>');return;}
  let file;
  if(url.pathname==='/rtc-observer.mjs')file=tapFile;
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
  assert.ok(['candidate-answer','terminal-observed'].includes(stage));
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
  const {browserMatrixRtcObservation}=await import('/rtc-observer.mjs');
  const check=(value,message)=>{if(!value)throw Error(message);};
  const opts=()=>({timeoutMs:8000}),mutation=operationId=>({...opts(),operationId});
  const done=async promise=>(await (await promise).wait(opts())).value;
  const wait=async(predicate,until,message)=>{while(!predicate()){check(performance.now()<until,message);await new Promise(resolve=>setTimeout(resolve,5));}};
  const peers=new Map(),clients=[],captures=[],renders=[],timers=[],oscillators=[],canvases=[];
  const audio=new AudioContext({sampleRate:48000});await audio.resume();
  const progress=Array.from({length:4},()=>({audio:0,video:0,lastAudio:null,lastVideo:null,lastPixels:null,changing:0}));
  const expected=[1,0,3,2],sourceHz=[440,660,880,1100],observationFailures=[];
  let measuring=false;const invalid=message=>{if(measuring&&observationFailures.length<16)observationFailures.push(message);};
  let arming=null,closeProof=null,observer,originalBegin=BrowserRtcConnection.begin,
      originalReplacement=BrowserRtcConnection.beginReplacement;
  const NativePeer=globalThis.RTCPeerConnection;
  globalThis.RTCPeerConnection=class extends NativePeer{
   constructor(...args){super(...args);if(!arming)return;
    check(!peers.has(arming.id)&&peers.size<5,'five exact original RTC peers');
    const binding={...arming,peer:this};peers.set(binding.id,binding);
    if(!binding.replacement)return;
    const apply=this.setRemoteDescription;
    this.setRemoteDescription=async description=>{
     check(description?.type==='answer','normal unchanged target answer');
     await globalThis.terminalCheckpoint('candidate-answer',{operation_id:spec.operationId,
      connection_id:binding.id,close_already_attempted:closeProof!==null},binding.cutoff-performance.now());
     return apply.call(this,description);
    };
    this.addEventListener('connectionstatechange',()=>{
     if(this.connectionState!=='connected'||closeProof!==null)return;
     const states=[...new Set(this.getTransceivers().flatMap(t=>[t.sender.transport?.state,t.receiver.transport?.state]).filter(Boolean))];
     check(states.length>0&&states.every(state=>state==='connected'),'actual candidate DTLS connected before close');
     closeProof={operation_id:spec.operationId,connection_id:binding.id,dtls:states,
      connection_state:this.connectionState,ice_state:this.iceConnectionState,at_ms:performance.now(),
      target_control_epoch:clients[2].transport.physicalEpoch,only_native_peer_closed:true};
     // Synchronous native close in the actual Connected event, before the SDK
     // prepared/readiness promise microtasks can submit a decoded Ready.
     NativePeer.prototype.close.call(this);
    });
   }
  };
  for(const [name,original] of [['begin',originalBegin],['beginReplacement',originalReplacement]]){
   BrowserRtcConnection[name]=function(...args){
    check(arming===null,'original synchronous peer construction');
    arming={id:args[1].connectionId,replacement:name==='beginReplacement',cutoff:performance.now()+args.at(-1).timeoutMs};
    try{return original.apply(this,args);}finally{arming=null;}
   };
  }
  let functionalFailure,cleanupFailures=[];
  try{
   for(const actor of spec.actors){
    const algorithm=actor.signing.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
    const key=await crypto.subtle.importKey('jwk',actor.signing.jwk,algorithm,false,['sign']);
    const trust=await crypto.subtle.importKey('jwk',actor.server.jwk,algorithm,false,['verify']);
    const enrollment={credential:actor.credential,identityId:actor.identityId,device:actor.device,
     signing:{algorithm:actor.signing.algorithm,keyId:actor.signing.keyId,key},
     server:{algorithm:actor.server.algorithm,keyId:actor.server.keyId,key:trust,
      notBefore:actor.server.notBefore,expiresAt:actor.server.expiresAt}};
    clients.push(await BrowserSdkClient.open({tenantId:'tnt_default',identityId:actor.identityId,deviceId:actor.device.id,
     endpoints:{'wss-webrtc':actor.url},preferredTransport:'wss-webrtc',allowFallback:false,
     resolveEnrollment:async()=>enrollment,timeoutMs:8000,attemptTimeoutMs:3000}));
   }
   const connections=[];
   for(let i=0;i<4;i++){
    const session=clients[i%2].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[Math.floor(i/2)]);
    connections.push(await done(session.connections.offer({connectionId:`browser-wire-connection-${i}`,
     streams:{first:{id:`browser-wire-audio-${i}`,kind:'audio',direction:'sendrecv',codecs:{first:'opus',rest:[]}},
      rest:[{id:`browser-wire-video-${i}`,kind:'video',direction:'sendrecv',codecs:{first:'vp8',rest:[]}}]}},mutation(`browser-wire-offer-${i}`))));
   }
   for(let i=0;i<4;i++){
    const view=await connections[i].refresh(opts()),peer=expected[i];
    await done(connections[i].streams.subscribe({selectors:{first:{stream:`browser-wire-audio-${peer}`},rest:[{stream:`browser-wire-video-${peer}`}]},
     expectedStreamRevision:view.streamRevision},mutation(`browser-wire-select-${i}`)));
   }
   const bindingById=new Map(connections.map((connection,i)=>[connection.id,i]));
   observer=browserMatrixRtcObservation(BrowserRtcConnection,{
    invalid(message){invalid(String(message).slice(0,128));},
    audio(id,timestamp,at,samples){
     const i=bindingById.get(id);if(i===undefined)return;
     if(samples.length!==960){invalid('actual20ms decoded PCM');return;}
     let energy=0,real=0,imaginary=0;const hz=sourceHz[expected[i]];
     for(let n=0;n<samples.length;n++){energy+=samples[n]*samples[n];real+=samples[n]*Math.cos(2*Math.PI*hz*n/48000);imaginary+=samples[n]*Math.sin(2*Math.PI*hz*n/48000);}
     if(!(energy>0&&Math.sqrt(energy/960)>.03)){invalid('old receiver actual audible decoded PCM');return;}
     if(!(2*(real*real+imaginary*imaginary)/(samples.length*energy)>.55)){invalid('old receiver actual source tone');return;}
     progress[i].audio++;progress[i].lastAudio=at;
    },
    video(id,timestamp,at,frame,width,height){
     const i=bindingById.get(id);if(i===undefined)return;
     if(width!==640||height!==360){invalid('actual decoded640x360 VP8');return;}
     const canvas=canvases[i].measurement,ctx=canvas.getContext('2d');ctx.drawImage(frame,0,0,16,16);
     const pixel=ctx.getImageData(0,0,1,1).data,mark=60+expected[i]*40;
     if(!(Math.abs(pixel[0]-mark)<12&&Math.abs(pixel[1]-mark)<12)){invalid('old receiver decoded source pixels');return;}
     const moving=ctx.getImageData(0,8,16,1).data;
     if(progress[i].lastPixels&&moving.some((v,n)=>v!==progress[i].lastPixels[n]))progress[i].changing++;
     progress[i].lastPixels=[...moving];progress[i].video++;progress[i].lastVideo=at;
    }
   });
   for(let i=0;i<4;i++){
    const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
    const output=document.createElement('canvas');output.width=640;output.height=360;
    const measurement=document.createElement('canvas');measurement.width=16;measurement.height=16;canvases.push({canvas,output,measurement});
    let ordinal=0;const draw=()=>{const ctx=canvas.getContext('2d'),mark=60+i*40;ctx.fillStyle=`rgb(${mark},${mark},${mark})`;ctx.fillRect(0,0,640,360);
     ctx.fillStyle='white';ctx.fillRect((ordinal++*13)%600,180,20,100);};draw();timers.push(setInterval(draw,1000/30));
    const oscillator=audio.createOscillator(),gain=audio.createGain();oscillator.frequency.value=sourceHz[i];gain.gain.value=.15;oscillator.connect(gain);oscillator.start();oscillators.push(oscillator);
    captures.push(await connections[i].capture({audio:true,audioNode:gain,video:'camera',videoCanvas:canvas},opts()));
    observer.attach(connections[i].id,output);
    const rendered=await connections[i].render({audio:true,video:[{stream:connections[i].streams.byId(`browser-wire-video-${i}`),canvas:output}]},opts());
    renders.push({owner:rendered,connectionId:connections[i].id,retirement:rendered.retirement,finished:rendered.finished});
   }
   const startup=performance.now()+8000;
   await wait(()=>progress.every(p=>p.audio>=3&&p.video>=3),startup,'all four real original AV receivers');
   measuring=true;
   const before=progress.map(p=>({audio:p.audio,video:p.video,changing:p.changing}));
   const originalEpochs=clients.map(client=>client.transport.physicalEpoch);
   check(originalEpochs.every(epoch=>Number.isSafeInteger(epoch)&&epoch>0),'known original physical control epochs');
   const session=clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[0]);
   const current=await session.refresh(opts()),moveCutoff=performance.now()+8000;
   const remaining=()=>{const timeoutMs=Math.floor(moveCutoff-performance.now());check(timeoutMs>0,'original8s move deadline');return {timeoutMs};};
   const operation=await connections[0].replaceDevice({retain:connections[1].id,target:{endpoint:spec.targetEndpoint},
    expectedTopologyRevision:current.topologyRevision,
    video:{publications:[{previous:'browser-wire-video-0',replacement:'browser-wire-target-video'}],
     receivers:[{previous:'browser-wire-video-1',replacement:'browser-wire-target-incoming'}]}},
    {...remaining(),operationId:spec.operationId});
   const reference=operation.referenceJson();
   let targetFailure;
   try{await clients[2].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[0]).connections.prepareReplacement({operation:operation.reference,
    audio:'browser-wire-target-audio',video:{send:'browser-wire-target-video',receive:'browser-wire-target-incoming'}},remaining());}
   catch(error){targetFailure={name:error.name,kind:error.kind,certainty:error.certainty};}
   check(closeProof&&targetFailure,'actual connected candidate must close, and original preparation must refuse');
   const terminalCutoff=Math.min(moveCutoff,closeProof.at_ms+5000);let status;
   for(;;){status=await operation.status({...remaining(),timeoutMs:Math.min(1000,remaining().timeoutMs)});
    check(performance.now()<terminalCutoff,'original root terminal from wire close');if(['failed','cancelled'].includes(status.state))break;await new Promise(r=>setTimeout(r,10));}
   check(operation.referenceJson().operation_id===reference.operation_id&&status.recordId,'same original operation and record');
   const closedAt=performance.now(),post=progress.map(p=>({audio:p.audio,video:p.video,changing:p.changing}));
   await new Promise(resolve=>setTimeout(resolve,1000));
   for(let i=0;i<4;i++){const now=performance.now(),p=progress[i];
    check(p.audio>=post[i].audio+3&&p.video>=post[i].video+3&&p.changing>post[i].changing,'old actual AV has new output after server terminal');
    check(p.lastAudio>=closedAt&&p.lastVideo>=closedAt&&now-p.lastAudio<200&&now-p.lastVideo<200,'old actual AV remains fresh after candidate close');}
   check(observationFailures.length===0,'original decoded privacy/format/continuity observation failed');
   for(let i=0;i<clients.length;i++){
    check(clients[i].transport.physicalEpoch===originalEpochs[i],'original WSS epoch unchanged');
    const view=await clients[i].conversations.byId(spec.conversationId).refresh(opts());check(view.id===spec.conversationId,'original control remains usable');
   }
   const unrelated=await clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[1]).refresh(opts());check(unrelated.state==='accepted','unrelated Session intact');
   await globalThis.terminalCheckpoint('terminal-observed',{operation_id:spec.operationId,record_id:status.recordId,
    connection_id:closeProof.connection_id,root_state:status.state,close:closeProof,
    wss_epochs_unchanged:true,old_decoder_outputs:progress.map(p=>({audio:p.audio,video:p.video})),unrelated_session_id:spec.sessions[1]},8000);
   return {format:spec.format,passed:true,original_root:{operation_id:spec.operationId,record_id:status.recordId,state:status.state},
    actual_connected_close:closeProof,target_failure:targetFailure,old_decoded_outputs:progress.map(p=>({audio:p.audio,video:p.video})),wss_epochs:originalEpochs};
  }catch(error){functionalFailure=error;throw error;}
  finally{
   for(const timer of timers)clearInterval(timer);
   for(const owner of captures){owner.close();if(owner.closed!==true)cleanupFailures.push('original-capture-close');}
   for(const entry of renders)entry.owner.close();
   for(const entry of renders){try{const receipt=await entry.retirement;
    check(receipt&&Object.isFrozen(receipt)&&receipt.connectionId===entry.connectionId&&receipt.scope==='renderer-local'
     &&receipt.outputsDetached===true&&['closed','not-created'].includes(receipt.audioGraph),'exact original render retirement');
    await entry.finished;}catch(error){cleanupFailures.push(error.name);}}
   for(const oscillator of oscillators){try{oscillator.stop();oscillator.disconnect();}catch{cleanupFailures.push('oscillator-close');}}
   try{await audio.close();}catch{cleanupFailures.push('source-audio-close');}
   if(observer){try{await observer.close();}catch{cleanupFailures.push('actual-observer-close');}}
   if(!functionalFailure&&clients[0])for(let i=0;i<2;i++){
    try{const session=clients[0].conversations.byId(spec.conversationId).sessions.byId(spec.sessions[i]);const view=await session.refresh(opts());
     await done(session.end({reasonCode:0,reason:'original integration cleanup',expectedTopologyRevision:view.topologyRevision},mutation(`browser-wire-session-end-${i}`)));}
    catch{cleanupFailures.push('original-session-end');}
   }
   for(const client of clients){try{await client.close(opts());}catch{cleanupFailures.push('original-sdk-close');}}
   BrowserRtcConnection.begin=originalBegin;BrowserRtcConnection.beginReplacement=originalReplacement;globalThis.RTCPeerConnection=NativePeer;
   peers.clear();for(const group of canvases)for(const canvas of Object.values(group)){canvas.width=0;canvas.height=0;}
   if(!functionalFailure)check(cleanupFailures.length===0,'original browser owners did not join cleanly');
  }
 },spec);
}catch(error){primaryFailure=error;throw error;}
finally{
 const cleanup=await closeOwnedChromium({browser,context,assets});
 if(result)result.browser_close=cleanup.browserClose;
 if(!primaryFailure&&cleanup.failures.length)throw new AggregateError(cleanup.failures.map(value=>Error(value.phase+': '+value.name)),'Original browser cleanup failed');
}
process.stdout.write(JSON.stringify(result)+'\n');
