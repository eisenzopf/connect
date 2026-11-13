import assert from 'node:assert/strict';
import {test} from 'node:test';
import * as p from '../typescript/dist/browser_video_publication.js';
import {BrowserVp8Codec} from '../typescript/dist/browser_codecs.js';
import {MediaSendQueue,RtpSender} from '../typescript/dist/browser_rtp.js';
import {BrowserMediaPeer} from '../typescript/dist/browser_media.js';
import {BrowserMediaCapture} from '../typescript/dist/browser_media_capture.js';

// Controlled native codec/transport edges. These are not signed Root, network,
// physical-camera, real WebCodecs, decoder, or media-quality qualification.
async function globals(values,run) {
 const old=Object.entries(values).map(([name])=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
 for(const [name,value]of Object.entries(values))Object.defineProperty(globalThis,name,{configurable:true,value});
 try{return await run();}finally{for(const [name,value]of old)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}
const clock=run=>{let now=100;return globals({performance:{now:()=>now}},()=>run(value=>now=value));};
const frame=timestamp=>({timestamp,closed:0,close(){this.closed++;}});
const accepted=(publication,timestamp=100000)=>{const f=frame(timestamp),input=p.publicationInput(publication,f);assert.ok(input);p.acceptPublicationInput(input);return input;};
const sender=()=>new RtpSender({handle:1,streamId:'video',connectionId:'connection',sessionId:'session',generation:1,kind:'video',codec:'vp8',payloadType:96,direction:'sendrecv'});
function packets(input,key=true,count=2,created=performance.now()) {
 const values=sender().packetize(new Uint8Array(count===2?1500:3),9000);
 assert.equal(values.length,count);p.publicationPackets(input,key,values,created);return values;
}
function peerFor(publication,queue,write,registry={require(){}}) {
 const peer=Object.create(BrowserMediaPeer.prototype);
 Object.assign(peer,{sending:false,stopped:false,feedbackWrites:new Map(),queue,registry,
  routes:new Map([[1,{publication}]]),writer:{write},counters:{sent:0}});
 return peer;
}

test('private accepted encoder outputs remain private after boundary; new public acceptance retains timestamp and requests a key',()=>clock(async set=>{
 let native,closed=0;const requests=[],outputs=[];
 class Encoder {state='unconfigured';encodeQueueSize=0;constructor(callbacks){native=this;this.callbacks=callbacks;}configure(){this.state='configured';}encode(f,options){requests.push({timestamp:f.timestamp,...options});}close(){this.state='closed';}}
 class Decoder {state='unconfigured';configure(){this.state='configured';}close(){this.state='closed';}}
 await globals({VideoEncoder:Encoder,VideoDecoder:Decoder},async()=>{
  const publication=p.videoPublication(),codec=new BrowserVp8Codec({encoded(f){const input=p.takeEncodedPublication(f.bytes);outputs.push({f,input,current:p.currentPublicationOutput(publication,input,f.timestampUs,f.key)});},decoded(){},error(error){throw error;}});
  const route={publication,vp8:codec,sender:{},captureClock:{observe(timestamp){assert.ok(timestamp===123456||timestamp===156789);}}};
  const peer=Object.create(BrowserMediaPeer.prototype);peer.state=()=>({routes:new Map([['video',route]])});peer.videoControlling=()=>false;
  const emit=(timestamp,key)=>native.callbacks.output({timestamp,type:key?'key':'delta',byteLength:3,copyTo(bytes){bytes.set([key?0:1,2,3]);}});
  try {
   const original=frame(123456);assert.equal(peer.encodeVideo('connection','video',original),true);closed+=original.closed;
   assert.equal(publication.epoch,0,'no completed boundary means no public epoch');
   set(101);p.completeVideoPublication(publication);
   emit(123456,true);assert.equal(outputs[0].current,false);assert.equal(outputs[0].input.epoch,0);assert.equal(publication.needed,true);
   const current=frame(156789);assert.equal(peer.encodeVideo('connection','video',current),true);closed+=current.closed;
   assert.deepEqual(requests,[{timestamp:123456,keyFrame:true},{timestamp:156789,keyFrame:true}]);
   emit(156789,true);assert.equal(outputs[1].current,true);assert.equal(outputs[1].input.basis,'explicit-sdk-submission');assert.equal(outputs[1].f.timestampUs,156789);
   assert.equal(publication.needed,true,'encode/output is not complete local datagram submission');
   native.encodeQueueSize=2;const refused=frame(190122);assert.equal(peer.encodeVideo('connection','video',refused),false);closed+=refused.closed;assert.equal(requests.length,2);
  } finally {codec.close();p.closeVideoPublication(publication);}
  assert.equal(closed,3);assert.equal(codec.frameTimes.length,0);
 });
}));

test('queue clones retain frame provenance and only every successful original fragment clears the request',()=>clock(set=>{
 const publication=p.videoPublication();p.completeVideoPublication(publication);const input=accepted(publication),original=packets(input),queue=new MediaSendQueue();
 assert.equal(queue.push(original,false),true);const one=queue.pop();assert.notEqual(one,original[0]);assert.equal(p.currentPublicationPacket(original[0],publication),false);assert.equal(p.currentPublicationPacket(one,publication),true);
 p.settlePublicationPacket(one,true);p.settlePublicationPacket(one,true);assert.equal(publication.needed,true,'duplicate settlement cannot complete the second fragment');
 set(101);p.settlePublicationPacket(queue.pop(),true);assert.equal(publication.needed,false);queue.clear();
}));

test('queue expiry, eviction, clear, delta and exact original deadline never satisfy a full public key',()=>clock(set=>{
 for(const mode of ['expiry','eviction','clear','partial','delta','deadline']){
  set(100);const publication=p.videoPublication();p.completeVideoPublication(publication);const input=accepted(publication),queue=new MediaSendQueue(),original=packets(input,mode!=='delta');queue.push(original,false);
  if(mode==='expiry'){set(300);assert.equal(queue.pop(),undefined);}
  if(mode==='clear')queue.clear();
  if(mode==='eviction'){queue.push(Array.from({length:64},()=>new Uint8Array(20)),false);assert.equal(queue.droppedPackets,2);queue.clear();}
  if(mode==='partial'){p.settlePublicationPacket(queue.pop(),true);p.settlePublicationPacket(queue.pop(),false);}
  if(mode==='delta'||mode==='deadline'){if(mode==='deadline')set(300);for(let n=0;n<2;n++)p.settlePublicationPacket(queue.pop(100),true);}
  assert.equal(publication.needed,true,mode);queue.clear();p.closeVideoPublication(publication);
 }
}));

test('actual drain drops queued private bytes, preserves an already-started private write, and waits for every public key write',()=>clock(async set=>{
 const publication=p.videoPublication(),queue=new MediaSendQueue();queue.push(packets(accepted(publication)),false);
 let release;const written=[];const peer=peerFor(publication,queue,bytes=>{written.push(bytes);return written.length===1?new Promise(resolve=>release=resolve):Promise.resolve();});
 const original=peer.drain();assert.equal(written.length,1);set(101);p.completeVideoPublication(publication);release();await original;
 assert.equal(written.length,1,'second queued private fragment was not promoted');assert.equal(publication.needed,true);assert.equal(peer.counters.sent,1,'already-started original write retains normal accounting');
 queue.push(packets(accepted(publication,133333)),false);await peer.drain();assert.equal(written.length,3);assert.equal(publication.needed,false);
}));

test('actual drain failures, original age, revoked registry and retired owner cannot clear the public request',()=>clock(async set=>{
 for(const mode of ['write-error','revoked','retired','age']){
  set(100);const publication=p.videoPublication();p.completeVideoPublication(publication);const queue=new MediaSendQueue();queue.push(packets(accepted(publication)),false);
  const failure=Error(mode),peer=peerFor(publication,queue,()=>{if(mode==='age'){set(301);return Promise.resolve();}return Promise.reject(failure);},{require(){if(mode==='revoked')throw failure;}});
  if(mode==='retired')p.closeVideoPublication(publication);
  if(mode==='write-error')await assert.rejects(peer.drain(),error=>error===failure);else await peer.drain();
  assert.equal(publication.needed,true,mode);queue.clear();assert.equal(peer.sending,false);
 }
}));

test('owned registration, delivery, presentation and optional physical capture remain distinct and old tags survive another wrapper',()=>clock(set=>{
 const publication=p.videoPublication(),peer={},connection={},other={};p.publicationLookup(peer,()=>publication);p.publicationConnection(connection,peer,'connection');p.publicationConnection(other,peer,'connection');
 const old=p.capturePublicationRegistration(connection,'video'),privateFrame=frame(99000);p.ownPublicationFrame(privateFrame,old,99,99);assert.equal(p.publicationInput(undefined,privateFrame),false,'even current private owned input cannot be relabeled through an ordinary wrapper');set(101);p.completeVideoPublication(publication);set(102);
 const late=frame(97000);p.ownPublicationFrame(late,old,102,102);assert.equal(p.publicationInput(publication,late),false);assert.equal(p.publicationInput(undefined,late),false,'known old owned input cannot become a generic explicit request');
 const registration=p.capturePublicationRegistration(other,'video'),fallback=frame(103000);p.ownPublicationFrame(fallback,registration,undefined,103);const input=p.publicationInput(publication,fallback);assert.equal(input.basis,'owned-new-callback-presentation-only');assert.equal(input.captureTimeMs,undefined);assert.equal(input.timestampUs,103000);
 for(const captureTime of [100,NaN]){const f=frame(104000);p.ownPublicationFrame(f,registration,captureTime,104);assert.equal(p.publicationInput(publication,f),false);}
 const stalePresentation=frame(100000);p.ownPublicationFrame(stalePresentation,registration,undefined,100);assert.equal(p.publicationInput(publication,stalePresentation),false);
 const physical=frame(104000);p.ownPublicationFrame(physical,registration,103,104);assert.equal(p.publicationInput(publication,physical).basis,'owned-capture-clock');
 p.closeVideoPublication(publication);assert.equal(p.publicationInput(publication,physical),false);
}));

test('actual capture callback registered privately is dropped then its new registration permits unchanged presentation time',()=>clock(async set=>{
 const publication=p.videoPublication(),peer={},connection={},callbacks=[],received=[],draws=[];p.publicationLookup(peer,()=>publication);p.publicationConnection(connection,peer,'connection');
 connection.encodeVideo=(stream,input)=>{assert.equal(stream,'video');const value=p.publicationInput(publication,input);received.push({timestamp:input.timestamp,value});input.close();return value!==false;};
 const video={play:()=>Promise.resolve(),requestVideoFrameCallback(callback){callbacks.push(callback);return callbacks.length;}};
 await globals({MediaStream:class{constructor(tracks){this.tracks=tracks;}},VideoFrame:class{constructor(canvas,options){this.timestamp=options.timestamp;}close(){this.closed=true;}},document:{createElement:kind=>kind==='video'?video:{getContext:()=>({drawImage(){draws.push(true);}})}}},async()=>{
  const capture=Object.create(BrowserMediaCapture.prototype);Object.assign(capture,{connection,options:{videoStreamId:'video'},stopped:false,paused:false,bound:value=>value});
  try {
   await capture.camera({kind:'video'});assert.equal(callbacks.length,1);set(101);p.completeVideoPublication(publication);set(102);
   callbacks[0](102,{presentationTime:102,mediaTime:0});assert.equal(received[0].value,false);assert.equal(callbacks.length,2);
   set(136);callbacks[1](136,{presentationTime:136,mediaTime:.034});assert.equal(received[1].timestamp,136000);assert.equal(received[1].value.basis,'owned-new-callback-presentation-only');assert.equal(callbacks.length,3);
   assert.equal(draws.length,2);assert.equal(publication.needed,true);
  }finally{capture.stopped=true;p.closeVideoPublication(publication);}
 });
}));

test('ordinary publishers and untagged ordinary queued packets keep existing behavior without Root state',()=>clock(async()=>{
 assert.equal(p.publicationInput(undefined,frame(123)),undefined);const queue=new MediaSendQueue(),bytes=sender().packetize(new Uint8Array([0,2,3]),123);queue.push(bytes,false);let calls=0;
 const peer=peerFor(undefined,queue,()=>{calls++;return Promise.resolve();});await peer.drain();assert.equal(calls,1);assert.equal(peer.counters.sent,1);
 const publication=p.videoPublication(),privateInput=accepted(publication);assert.equal(privateInput.epoch,0);assert.equal(publication.needed,false);assert.equal(p.currentPublicationOutput(publication,privateInput,100000,true),true,'private readiness still submits under its original route authority');
 p.closeVideoPublication(publication);assert.equal(p.currentPublicationOutput(publication,privateInput,100000,true),false);
}));
