// Owned BrowserMediaPeer with real WHATWG stream backpressure. Signaling and
// codec setup are controlled here; actual signed Chromium transport is separate.
import test from 'node:test';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
const base=process.env.CONNECT_GATE6_TS_DIST?pathToFileURL(path.resolve(process.env.CONNECT_GATE6_TS_DIST)+'/').href:new URL('../typescript/dist/',import.meta.url).href;
const {BrowserMediaPeer}=await import(base+'browser_media.js');
const {decodeNativeVideoFeedback}=await import(base+'browser_video_wire.js');
const {envelope}=await import(base+'uctp_wire.js');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return{promise,resolve};};
async function until(check,label){const end=performance.now()+3000;while(!check()){assert.ok(performance.now()<end,label);await pause(2);}}
const profile='rtcp-pli-epoch/1',root={cid:'room',sid:'session',connid:'connection'},caps={video_feedback:profile};
const hex=n=>n.toString(16).padStart(32,'0');
const allocation={profile,original_operation:'offer',original_offer:'wire1',stream_generation:1,connection_generation:1};
const notice=n=>envelope('stream.epoch',{...allocation,strm_id:'video',stream_local_id:7,incarnation:hex(99),revision:n,direction:'receiver',epoch:hex(n),ssrc:83,first_sequence:n*10,first_timestamp:n*30000},root);

async function fixture(mode){
 const release=deferred(),errors=[],writes=[],readyAt=[],old=new Map();let seeded,aborted=0,cancelled=0;
 class Codec{state='unconfigured';encodeQueueSize=0;decodeQueueSize=0;static async isConfigSupported(){return{supported:true};}configure(){this.state='configured';}reset(){}close(){this.state='closed';}}
 for(const [name,value] of Object.entries({VideoEncoder:Codec,VideoDecoder:Codec,VideoFrame:class{},EncodedVideoChunk:class{}})){
  old.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,value});
 }
 const writable=new WritableStream({
  async write(bytes){
   if(bytes.length===1){await release.promise;return;}
   const value=decodeNativeVideoFeedback(bytes);writes.push({value,at:performance.now()});
   if(mode==='write')await release.promise;
  },abort(){aborted++;},
 });
 if(mode==='ready'){
  const writer=writable.getWriter();seeded=writer.write(Uint8Array.of(255));void seeded.catch(()=>{});writer.releaseLock();
 }
 // Observe access to the original native ready promise, without replacing its
 // behavior or returning a synthetic writer. Backpressure is real stream state.
 const getWriter=writable.getWriter.bind(writable),ready=Object.getOwnPropertyDescriptor(WritableStreamDefaultWriter.prototype,'ready').get;
 writable.getWriter=()=>{const writer=getWriter();Object.defineProperty(writer,'ready',{get(){readyAt.push(performance.now());return ready.call(writer);}});return writer;};
 class Control{
  closed=false;events=new Set();lifetimes=new Set();count=0;
  datagrams={maxDatagramSize:1200,readable:new ReadableStream({cancel(){cancelled++;}}),writable};
  onEvent(f){this.events.add(f);return()=>this.events.delete(f);}
  onClose(f){this.lifetimes.add(f);return()=>this.lifetimes.delete(f);}
  emit(frame){for(const f of this.events)f(frame);}
  close(){if(this.closed)return;this.closed=true;for(const f of [...this.lifetimes])f();}
  begin(type,payload,scope){
   assert.equal(type,'connection.offer');assert.equal(payload.capabilities.video_feedback,profile);const id='wire'+ ++this.count;
   queueMicrotask(()=>this.emit(envelope('connection.offer',{by_participant:'participant',substrate:'webtransport',capabilities:caps,substrate_setup:{kind:'native-prepared/1',offer_id:id,revision:1,streams:[{strm_id:'video',kind:'video',direction:'sendrecv',stream_local_id:7,video_feedback:allocation,codec:{name:'vp8',params:{sample_rate:90000,channels:0,payload_type:96}}}]}},scope)));
   return{id,response:Promise.resolve([envelope('connection.answer',{}, {...scope,in_reply_to:id})])};
  }
  async send(){this.emit(notice(1));return'answer';}
  async operationStatus(){return{state:'completed'};}
 }
 const control=new Control(),peer=new BrowserMediaPeer(control,{error:error=>errors.push({error,at:performance.now()})});
 try{
  await peer.offerConnection({conversationId:'room',sessionId:'session',connectionId:'connection',operationId:'offer',video:'video',nativeVideoFeedback:true});
  await until(()=>readyAt.length>0,'owned feedback write must actually await readiness');
 }catch(error){release.resolve();await peer.shutdown();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];throw error;}
 return{peer,control,release,writes,errors,readyAt,get aborted(){return aborted;},get cancelled(){return cancelled;},async finish(){release.resolve();await seeded;await peer.shutdown();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}};
}
async function joinedAfterRelease(f){
 let joined=false;const closing=f.peer.shutdown().then(()=>joined=true);
 await pause(10);assert.equal(joined,false,'shutdown must join the outstanding stream operation');
 assert.equal(f.control.datagrams.writable.locked,true,'writer lock stays owned until join');
 f.release.resolve();await closing;
 assert.equal(f.control.datagrams.writable.locked,false);assert.equal(f.control.datagrams.readable.locked,false);
 assert.equal(f.control.events.size,0);assert.equal(f.control.lifetimes.size,0);assert.equal(f.aborted,1);assert.equal(f.cancelled,1);
}

test('browser feedback ready delay expires at original age without a late write and joins ownership', {timeout:5000},async()=>{
 const f=await fixture('ready');try{
  await until(()=>f.errors.length===1,'original feedback age must expire');
  assert.equal(f.errors[0].error.code,'timeout');assert.match(f.errors[0].error.message,/original age/);
  assert.ok(f.errors[0].at-f.readyAt[0]>=150 && f.errors[0].at-f.readyAt[0]<1000);
  assert.equal(f.writes.length,0);await joinedAfterRelease(f);await pause(10);assert.equal(f.writes.length,0,'resolving readiness after expiry cannot submit');
 }finally{await f.finish();}
});
test('browser feedback ready delay discards captured history and consumes a new serial for the new epoch', {timeout:5000},async()=>{
 const f=await fixture('ready');try{
  f.control.emit(notice(2));f.release.resolve();
  await until(()=>f.writes.length===1,'new history feedback must arrive');
  assert.equal(f.writes[0].value.epoch,hex(2));assert.equal(f.writes[0].value.datagramSequence,1,'old captured serial is consumed even though never submitted');
  await f.peer.shutdown();assert.equal(f.errors.length,0);assert.equal(f.writes.length,1);assert.equal(f.control.datagrams.writable.locked,false);
 }finally{await f.finish();}
});
test('browser feedback uncertain write is submitted once then closes and joins without retry', {timeout:5000},async()=>{
 const f=await fixture('write');try{
  await until(()=>f.writes.length===1,'first actual sink submission');
  assert.equal(f.writes[0].value.datagramSequence,0);assert.equal(f.writes[0].value.epoch,hex(1));
  await until(()=>f.errors.length===1,'uncertain write must consume original deadline');
  assert.equal(f.errors[0].error.code,'timeout');assert.equal(f.control.closed,true);
  f.control.emit(notice(2));await joinedAfterRelease(f);await pause(10);
  assert.equal(f.writes.length,1,'no retry or retarget after uncertain submission');assert.equal(f.errors.length,1);
 }finally{await f.finish();}
});
