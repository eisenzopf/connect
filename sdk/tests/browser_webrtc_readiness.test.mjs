import assert from 'node:assert/strict';
import {test} from 'node:test';
import {observeWebRtcDecodedReadiness} from '../typescript/dist/browser_webrtc_readiness.js';
import {BrowserDeadline} from '../typescript/dist/browser_deadline.js';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
import {BrowserTasks} from '../typescript/dist/browser_tasks.js';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
async function fixture(run,{video=false}={}) {
 const saved=new Map(),nodes=[],elements=[],contexts=[],sources=[],gains=[],publicOwners=[],publicWorks=[],documentListeners=new Set(),connectObservations=[];let callback,next=0,renderHookActive=false;
 class Node{gain={value:1};disconnects=0;connections=[];connect(node){this.connections.push(node);connectObservations.push({source:this,node,synchronousRenderHook:renderHookActive});}disconnect(node){this.disconnects++;this.connections=node?this.connections.filter(value=>value!==node):[];}}
 class Context{sampleRate=48000;destination={};audioWorklet={async addModule(){}};closes=0;constructor(){contexts.push(this);}createMediaStreamSource(){const node=new Node();node.context=this;sources.push(node);return node;}createGain(){const node=new Node();gains.push(node);return node;}async resume(){}async close(){this.closes++;}}
 class Worklet extends Node{port={onmessage:null,sent:[],closed:false,postMessage(value){this.sent.push(value);},close(){this.closed=true;}};constructor(){super();nodes.push(this);}}
 class Stream{constructor(tracks){this.tracks=tracks;}}
 const values={AudioContext:Context,AudioWorkletNode:Worklet,MediaStream:Stream,document:{visibilityState:'visible',addEventListener(type,fn){assert.equal(type,'visibilitychange');documentListeners.add(fn);},removeEventListener(type,fn){assert.equal(type,'visibilitychange');documentListeners.delete(fn);},createElement(kind){const element={kind,muted:false,srcObject:null,readyState:2,pauses:0,async play(){},pause(){this.pauses++;},requestVideoFrameCallback(fn){callback=fn;return++next;},cancelVideoFrameCallback(){callback=undefined;}};elements.push(element);return element;}}};
 for(const [name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});}
 const audio={id:'audio',track:{id:'rtc-audio',kind:'audio',readyState:'live'}},picture=video?{id:'receive',track:{id:'rtc-video',kind:'video',readyState:'live'}}:undefined;
 for(const entry of [audio,picture].filter(Boolean))entry.track.stop=()=>{entry.track.stops=(entry.track.stops??0)+1;entry.track.readyState='ended';};
 const stats={audio:{type:'inbound-rtp',kind:'audio',trackIdentifier:'rtc-audio',totalSamplesReceived:0,concealedSamples:0},video:{type:'inbound-rtp',kind:'video',trackIdentifier:'rtc-video',framesDecoded:0,keyFramesDecoded:0}};
 const peer={connectionState:'connected',getTransceivers(){return[audio,picture].filter(Boolean).map(entry=>({receiver:{track:entry.track}}));},close(){this.connectionState='closed';},async getStats(){return new Map(Object.entries(structuredClone(stats)));}},abort=new AbortController(),tasks=new Set();
 const track=(promise)=>{tasks.add(promise);void promise.finally(()=>tasks.delete(promise)).catch(()=>{});return promise;};
 let work;
 try{await run({audio,picture,stats,peer,abort,nodes,elements,contexts,sources,gains,connectObservations,
  begin(retainMutedDecoderSinks){work=observeWebRtcDecodedReadiness(peer,audio,picture,new BrowserDeadline(500),abort.signal,track,retainMutedDecoderSinks?value=>retainMutedDecoderSinks(typeof value==='function'?value:()=>value.release()):undefined);void work.catch(()=>{});return work;},
  beginDecoder(retain){work=observeWebRtcDecodedReadiness(peer,audio,picture,new BrowserDeadline(500),abort.signal,track,retain);void work.catch(()=>{});return work;},
  async started(){while(!nodes.length||(video&&elements.length<2))await tick();},
  pcm(values=new Float32Array(960)){nodes[0].port.onmessage({data:{epoch:0,timestampUs:1000,samples:values}});},
  frame(presentedFrames=1){const fn=callback;callback=undefined;assert.equal(typeof fn,'function');fn(10,{width:640,height:360,mediaTime:0,presentedFrames});},
  pendingFrame(){return callback;},
  publicOwner(decoder,completion){
   // Controlled internal owner: invoke the ACTUAL renderer and ACTUAL retained
   // lease. This does not fake a signed Root receipt or construct a second peer.
   const owner=Object.assign(Object.create(BrowserRtcConnection.prototype),{
    tasks:new BrowserTasks(),stopped:false,stop:new AbortController(),retired:new Set(),
    remote:new Map([['audio',audio.track],...(picture?[['receive',picture.track]]:[])]),
    client:{closed:false},input:{connectionId:'target',audio:'audio',receiveVideo:'receive'},
    peer,preparedDecoder:decoder,replacementCompletion:completion,replacementVideoPresented:false,
   });publicOwners.push(owner);return owner;
  },
  render(owner,canvas,options={timeoutMs:200},audioOnly=false){
   let work;renderHookActive=true;
   try{work=owner.render(audioOnly==='both'?{audio:true,video:{receive:canvas}}:audioOnly?{audio:true}:{audio:false,video:{receive:canvas}},options);}finally{renderHookActive=false;}
   void work.catch(()=>{});publicWorks.push(work);return work;
  },
 });}finally{
  for(const owner of publicOwners){owner.renderOwner?.close();owner.stop.abort();owner.retireMutedDecoderSinks();}
  abort.abort();await work?.catch(()=>{});await Promise.allSettled(publicWorks);
  await Promise.all(publicOwners.map(owner=>owner.tasks.join()));await Promise.allSettled(tasks);
  assert.equal(documentListeners.size,0,'All original renderer visibility listeners retired');
  for(const[name,descriptor]of saved){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
 }
}
test('replacement audio needs actual PCM and non-concealed receiver samples, not connection or RTP readiness',()=>fixture(async f=>{
 const work=f.begin();let done=false;void work.then(()=>done=true);await f.started();await tick();assert.equal(done,false);
 f.stats.audio.totalSamplesReceived=1920;f.stats.audio.concealedSamples=1920;f.pcm();await new Promise(r=>setTimeout(r,30));assert.equal(done,false);
 f.stats.audio.concealedSamples=0;const evidence=await work;assert.deepEqual(evidence,{audio:{stream_id:'audio',decoded_samples:960},video:null});
 assert.equal(f.contexts[0].closes,1);assert.equal(f.nodes[0].port.closed,true);assert.ok(f.elements.every(e=>e.srcObject===null&&e.pauses===1));assert.equal(f.audio.track.readyState,'live');assert.equal(f.peer.connectionState,'connected');
}));
test('replacement video needs a decoded keyframe and actual rendered frame before readiness',()=>fixture(async f=>{
 const work=f.begin();let done=false;void work.then(()=>done=true);await f.started();f.stats.audio.totalSamplesReceived=1920;f.pcm();f.stats.video.framesDecoded=1;f.frame();await new Promise(r=>setTimeout(r,30));assert.equal(done,false);
 f.stats.video.keyFramesDecoded=1;const evidence=await work;assert.deepEqual(evidence.video,{stream_id:'receive',decoded_frames:1,decoded_keyframes:1,decoder_startup:'fresh-peer-first-decoded-frame'});assert.equal(f.elements[1].srcObject,null);assert.equal(f.picture.track.readyState,'live');
},{video:true}));
test('decoder statistics without decoded video output cannot submit readiness',()=>fixture(async f=>{
 const work=f.begin();await f.started();f.stats.audio.totalSamplesReceived=960;f.pcm();f.stats.video.framesDecoded=f.stats.video.keyFramesDecoded=1;
 await new Promise(r=>setTimeout(r,30));f.abort.abort();await assert.rejects(work);assert.equal(f.contexts[0].closes,1);assert.equal(f.nodes[0].port.closed,true);
},{video:true}));
test('foreign-track and malformed PCM cannot establish replacement readiness',()=>fixture(async f=>{
 const work=f.begin();await f.started();f.stats.audio.totalSamplesReceived=960;f.stats.audio.trackIdentifier='foreign';f.pcm();await new Promise(r=>setTimeout(r,30));f.pcm(new Float32Array([NaN]));await assert.rejects(work,{code:'media'});assert.equal(f.contexts[0].closes,1);assert.equal(f.peer.connectionState,'connected');
}));

test('fresh decoder frame fallback records absent keyframe counter instead of inventing one',()=>fixture(async f=>{
 const work=f.begin();await f.started();f.stats.audio.totalSamplesReceived=960;f.pcm();delete f.stats.video.keyFramesDecoded;delete f.stats.video.framesDecoded;f.frame();
 const evidence=await work;assert.deepEqual(evidence.video,{stream_id:'receive',decoded_frames:1,decoded_keyframes:null,decoder_startup:'fresh-peer-first-decoded-frame'});
},{video:true}));

test('RTC readiness preserves coherent decoder counters when frames decode between rendered callbacks',()=>fixture(async f=>{
 const work=f.begin();await f.started();f.stats.audio.totalSamplesReceived=960;f.pcm();f.stats.video.framesDecoded=5;f.stats.video.keyFramesDecoded=2;f.frame();
 const evidence=await work;assert.deepEqual(evidence.video,{stream_id:'receive',decoded_frames:5,decoded_keyframes:2,decoder_startup:'fresh-peer-first-decoded-frame'});
},{video:true}));
test('RTC readiness waits rather than reports contradictory decoder counters',()=>fixture(async f=>{
 const work=f.begin();let done=false;void work.then(()=>done=true);await f.started();f.stats.audio.totalSamplesReceived=960;f.pcm();f.stats.video.framesDecoded=1;f.stats.video.keyFramesDecoded=2;f.frame();await new Promise(resolve=>setTimeout(resolve,30));assert.equal(done,false);
 f.stats.video.framesDecoded=3;const evidence=await work;assert.equal(evidence.video.decoded_frames,3);assert.equal(evidence.video.decoded_keyframes,2);
},{video:true}));


test("RTC decoded readiness releases observation graph but retains only muted decoder sinks for its owner", () =>
  fixture(
    async (f) => {
      let release;
      const work = f.begin((value) => {
        assert.equal(release, undefined);
        release = value;
      });
      await f.started();
      f.stats.audio.totalSamplesReceived = 1920;
      f.pcm();
      f.stats.video.framesDecoded = f.stats.video.keyFramesDecoded = 1;
      f.frame();
      const evidence = await work;
      assert.equal(evidence.audio.decoded_samples, 960);
      assert.equal(typeof release, "function");
      assert.equal(f.contexts[0].closes, 0);
      assert.equal(f.nodes[0].port.closed, true);
      assert.ok(
        f.elements.every(
          (element) =>
            element.muted && element.srcObject !== null && element.pauses === 0,
        ),
      );
      await release();
      await release();
      assert.equal(f.contexts[0].closes, 1);
      assert.ok(
        f.elements.every(
          (element) => element.srcObject === null && element.pauses === 1,
        ),
      );
      assert.equal(f.audio.track.readyState, "live");
      assert.equal(f.picture.track.readyState, "live");
      assert.equal(f.peer.connectionState, "connected");
    },
    { video: true },
  ));

test("retained RTC muted decoder sinks follow the original cancellation after decoded success", () =>
  fixture(async (f) => {
    let release;
    const work = f.begin((value) => {
      release = value;
    });
    await f.started();
    f.stats.audio.totalSamplesReceived = 1920;
    f.pcm();
    await work;
    assert.equal(typeof release, "function");
    assert.equal(f.elements[0].srcObject !== null, true);
    f.abort.abort();
    assert.equal(f.elements[0].srcObject, null);
    assert.equal(f.elements[0].pauses, 1);
    await release();
    assert.equal(f.elements[0].pauses, 1);
    assert.equal(f.contexts[0].closes, 1);
    assert.equal(f.audio.track.readyState, "live");
    assert.equal(f.peer.connectionState, "connected");
  }));

test("refused RTC decoder handoff and pre-evidence cancellation release all original private outputs", async () => {
  for (const phase of ["cancel", "register"])
    await fixture(async (f) => {
      const original = Error("original handoff refused");
      let transfers = 0;
      const work = f.begin(() => {
        transfers++;
        throw original;
      });
      await f.started();
      if (phase === "cancel") f.abort.abort();
      else {
        f.stats.audio.totalSamplesReceived = 1920;
        f.pcm();
      }
      await assert.rejects(work, (error) =>
        phase === "register"
          ? error === original
          : error.code === "closed" || error.code === "cancelled",
      );
      assert.equal(transfers, phase === "register" ? 1 : 0);
      assert.equal(f.contexts[0].closes, 1);
      assert.equal(f.nodes[0].port.closed, true);
      assert.ok(
        f.elements.every(
          (element) => element.srcObject === null && element.pauses === 1,
        ),
      );
      assert.equal(f.audio.track.readyState, "live");
      assert.equal(f.peer.connectionState, "connected");
    });
});

test("decoded RTC graph transfers once to the exact original track and releases only unadopted outputs", () =>
  fixture(async (f) => {
    let decoder, adopted;
    try {
      const work = f.beginDecoder((value) => (decoder = value));
      await f.started();
      f.stats.audio.totalSamplesReceived = 1920;
      f.pcm();
      await work;
      assert.equal(typeof decoder.takeAudio, "function");
      assert.equal(f.contexts[0].closes, 0);
      assert.equal(f.nodes[0].port.closed, true);
      assert.equal(f.gains[0].gain.value, 0);
      assert.deepEqual(f.sources[0].connections, [f.gains[0]]);
      assert.equal(decoder.takeAudio({ ...f.audio.track }), undefined);
      adopted = decoder.takeAudio(f.audio.track);
      assert.equal(adopted.context, f.contexts[0]);
      assert.equal(adopted.source, f.sources[0]);
      assert.equal(adopted.mutedGain, f.gains[0]);
      assert.equal(adopted.playout, f.elements[0]);
      assert.equal(adopted.playout.muted, true);
      assert.equal(decoder.takeAudio(f.audio.track), undefined);
      f.abort.abort();
      await decoder.release();
      await decoder.release();
      assert.equal(adopted.context.closes, 0);
      assert.notEqual(adopted.playout.srcObject, null);
    } finally {
      if (adopted) {
        adopted.playout.pause();
        adopted.playout.srcObject = null;
        adopted.source.disconnect();
        adopted.mutedGain.disconnect();
        await adopted.context.close();
      }
    }
  }));

test("original cancellation before RTC graph adoption refuses transfer and joins its late close", () =>
  fixture(async (f) => {
    let decoder;
    const work = f.beginDecoder((value) => (decoder = value));
    await f.started();
    f.stats.audio.totalSamplesReceived = 1920;
    f.pcm();
    await work;
    assert.equal(typeof decoder.takeAudio, "function");
    let resolve;
    const cleanup = new Promise((done) => (resolve = done));
    f.contexts[0].close = () => {
      f.contexts[0].closes++;
      return cleanup;
    };
    try {
      f.abort.abort();
      assert.equal(f.elements[0].srcObject, null);
      assert.equal(decoder.takeAudio(f.audio.track), undefined);
      await decoder.release();
      assert.equal(f.contexts[0].closes, 1);
    } finally {
      resolve();
    }
  }));


function originalDeferred(){let resolve;const promise=new Promise(done=>{resolve=done;});return{promise,resolve};}
function originalCanvas(){const frames=[];return{frames,getContext(){return{drawImage(element){frames.push(element);}};}};}
async function retainedVideo(f){let decoder;const work=f.beginDecoder(value=>{decoder=value;});await f.started();f.stats.audio.totalSamplesReceived=1920;f.pcm();f.stats.video.framesDecoded=f.stats.video.keyFramesDecoded=1;f.frame(1);await work;return decoder;}

test('prepared video keeps a current presentation floor after readiness and transfers the exact element once',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),picture=f.elements[1];
 f.frame(12);f.frame(25);
 const latePrivate=f.pendingFrame();
 assert.equal(decoder.takeVideo({...f.picture.track}),undefined,'Another object is not the original track');
 assert.equal(decoder.canTakeVideo(f.picture.track),true);
 const adopted=decoder.takeVideo(f.picture.track);
 try{
  assert.equal(adopted.element,picture);assert.equal(adopted.presentedFloor,25,'Not the stale readiness counter1');
  assert.equal(decoder.takeVideo(f.picture.track),undefined);
  assert.equal(decoder.canTakeVideo(f.picture.track),false);
  assert.equal(f.pendingFrame(),undefined);
  latePrivate(20,{width:640,height:360,mediaTime:0,presentedFrames:999});
  assert.equal(f.pendingFrame(),undefined,'Cancelled private callback cannot rearm or update new ownership');
  await decoder.release();assert.notEqual(picture.srcObject,null);assert.equal(picture.pauses,0);
  assert.equal(f.picture.track.readyState,'live');
 }finally{adopted.element.pause();adopted.element.srcObject=null;}
},{video:true}));

test('prepared video refuses stopped or cancelled originals without transferring another track',async()=>{
 for(const phase of ['track-ended','cancel','released','cancel-callback-throws'])await fixture(async f=>{
  const decoder=await retainedVideo(f),picture=f.elements[1];
  if(phase==='track-ended')f.picture.track.readyState='ended';
  if(phase==='cancel')f.abort.abort();
  if(phase==='released')await decoder.release();
  if(phase==='cancel-callback-throws'){
   const original=Error('original callback cancellation failed');
   picture.cancelVideoFrameCallback=()=>{throw original;};
   assert.throws(()=>decoder.takeVideo(f.picture.track),error=>error===original);
   assert.notEqual(picture.srcObject,null,'Failed transfer keeps original lease custody');
   await assert.rejects(decoder.release(),error=>error===original);
  }else{
   assert.equal(decoder.takeVideo(f.picture.track),undefined);
   await decoder.release();
  }
  assert.equal(picture.srcObject,null);assert.equal(picture.pauses,1);
  // A failed native cancel is an explicit cleanup error, not an invented
  // callback-join receipt. Its genuinely late delivery still cannot rearm.
  if(phase==='cancel-callback-throws')f.frame(2);
  assert.equal(f.pendingFrame(),undefined);
 },{video:true});
});

test('actual replacement renderer waits its original completion and a new same-element presentation beyond the take floor',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),root=originalDeferred(),canvas=originalCanvas(),picture=f.elements[1];
 const owner=f.publicOwner(decoder,root.promise),work=f.render(owner,canvas);
 await tick();assert.equal(canvas.frames.length,0);assert.equal(f.elements.length,2);
 f.frame(40);const latePrivate=f.pendingFrame();
 root.resolve({recordId:'original-root',generation:2});await tick();
 let done=false;void work.then(()=>{done=true;});
 assert.equal(done,false);assert.equal(canvas.frames.length,0);assert.equal(f.elements.length,2,'Reuse exact original element');
 const publicCallback=f.pendingFrame();
 latePrivate(20,{width:640,height:360,mediaTime:0,presentedFrames:999});
 assert.equal(f.pendingFrame(),publicCallback,'Late private callback cannot replace the public callback');
 f.frame(40);await tick();assert.equal(canvas.frames.length,0,'A cached presentation at the floor cannot paint');
 f.frame(41);const render=await work;
 try{
  assert.deepEqual(canvas.frames,[picture]);assert.equal(render.diagnostics.video,1);
  f.frame(42);assert.deepEqual(canvas.frames,[picture,picture]);
 }finally{render.close();await render.retirement;await render.finished;}
 assert.equal(picture.pauses,1);assert.equal(picture.srcObject,null);assert.equal(f.pendingFrame(),undefined);
},{video:true}));

test('pending original completion cannot activate video and cancellation or deadline settles the same private owners',async()=>{
 for(const phase of ['cancel','deadline'])await fixture(async f=>{
  const decoder=await retainedVideo(f),root=originalDeferred(),canvas=originalCanvas(),abort=new AbortController();
  const owner=f.publicOwner(decoder,root.promise),work=f.render(owner,canvas,{timeoutMs:phase==='deadline'?15:200,signal:abort.signal});
  try{
   await tick();assert.equal(canvas.frames.length,0);f.frame(30);
   const originalTerminal=owner.renderOwner.finished.catch(error=>error);let reported;
   if(phase==='cancel')abort.abort();
   await assert.rejects(work,error=>{reported=error;return error.code===(phase==='cancel'?'cancelled':'timeout');});
   assert.equal(reported,await originalTerminal,'Caller cancellation/deadline preserve the same original terminal Error');
   assert.equal(canvas.frames.length,0);assert.equal(f.elements.length,2);
   await owner.tasks.join();assert.equal(f.elements[1].srcObject,null);assert.equal(f.elements[1].pauses,1);
   assert.equal(f.pendingFrame(),undefined);
  }finally{root.resolve({recordId:'original-root',generation:2});}
 },{video:true});
});

test('replacement first-public-presentation counter failure or no advancing frame fails under original startup budget',async()=>{
 for(const phase of ['invalid-counter','no-next-frame'])await fixture(async f=>{
  const decoder=await retainedVideo(f),canvas=originalCanvas(),owner=f.publicOwner(decoder,Promise.resolve({recordId:'original-root',generation:2}));
  const work=f.render(owner,canvas,{timeoutMs:phase==='no-next-frame'?15:200});
  await tick();
  const originalTerminal=owner.renderOwner.finished.catch(error=>error);let reported;
  if(phase==='invalid-counter')f.frame(NaN);else f.frame(1);
  await assert.rejects(work,error=>{reported=error;return error.code===(phase==='invalid-counter'?'media':'timeout');});
  assert.equal(reported,await originalTerminal,'Internal cleanup cancellation must not replace the original media/deadline Error');
  if(phase==='invalid-counter')assert.equal(reported.message,'Original video presentation counter unavailable');
  assert.equal(canvas.frames.length,0);await owner.tasks.join();
  assert.equal(f.elements[1].srcObject,null);assert.equal(f.elements[1].pauses,1);assert.equal(f.pendingFrame(),undefined);
 },{video:true});
});

test('ordinary nonreplacement renderer retains its original first-presentation path',()=>fixture(async f=>{
 const decoder=await retainedVideo(f);await decoder.release();
 const canvas=originalCanvas(),owner=f.publicOwner(undefined,undefined);
 const render=await f.render(owner,canvas);
 try{
  assert.equal(f.elements.length,3,'Ordinary rendering still creates its original new output');
  assert.equal(canvas.frames.length,0);f.frame(1);
  assert.deepEqual(canvas.frames,[f.elements[2]]);assert.equal(render.diagnostics.video,1);
 }finally{render.close();await render.retirement;await render.finished;}
 assert.equal(f.elements[2].srcObject,null);assert.equal(f.elements[2].pauses,1);assert.equal(f.pendingFrame(),undefined);
},{video:true}));


test('same replacement owner permits another normal video render after its original adopted renderer closes',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),owner=f.publicOwner(decoder,Promise.resolve({recordId:'original-root',generation:2})),first=originalCanvas();
 const work=f.render(owner,first);await tick();f.frame(2);const original=await work;
 original.close();await original.retirement;await original.finished;
 const second=originalCanvas(),render=await f.render(owner,second);
 try{assert.equal(f.elements.length,3);f.frame(1);assert.deepEqual(second.frames,[f.elements[2]]);}
 finally{render.close();await render.retirement;await render.finished;}
 assert.equal(f.elements[1].pauses,1);assert.equal(f.elements[2].pauses,1);
},{video:true}));

test('successful replacement audio-only render may later create its normal public video output',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),owner=f.publicOwner(decoder,Promise.resolve({recordId:'original-root',generation:2}));
 const audio=await f.render(owner,undefined,{timeoutMs:200},true);
 audio.close();await audio.retirement;await audio.finished;
 assert.equal(f.elements[1].srcObject,null,'Unrequested prepared picture was deliberately retired');
 const canvas=originalCanvas(),work=f.render(owner,canvas);await tick();
 assert.equal(owner.replacementVideoPresented,false);f.frame(1);await tick();
 assert.equal(canvas.frames.length,0,'Audio-only retirement cannot turn its first cached video callback into a public paint');
 f.frame(1);assert.equal(canvas.frames.length,0,'Equal baseline cannot paint');f.frame(2);const video=await work;
 try{assert.equal(f.elements.length,3);assert.deepEqual(canvas.frames,[f.elements[2]]);assert.equal(owner.replacementVideoPresented,true);}
 finally{video.close();await video.retirement;await video.finished;}
},{video:true}));

test('replacement render can read-only recover its original uncertain completion before taking the prepared video',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),original=Error('controlled original uncertain completion');
 const rejected=Promise.reject(original);void rejected.catch(()=>{});
 const owner=f.publicOwner(decoder,rejected),canvas=originalCanvas(),calls=[];
 owner.signaling={async recoverReplacementCompletion(error,options){calls.push({error,options});return{recordId:'original-root',generation:2};}};
 const work=f.render(owner,canvas);await tick();assert.equal(calls.length,1);assert.equal(calls[0].error,original);
 assert.ok(calls[0].options.timeoutMs>0&&calls[0].options.timeoutMs<=200);assert.equal(canvas.frames.length,0);
 f.frame(2);const render=await work;
 try{assert.deepEqual(canvas.frames,[f.elements[1]]);}finally{render.close();await render.retirement;await render.finished;}
},{video:true}));


test('a pending read-only original recovery keeps its target lease for a later completed render',async()=>{for(const mode of [false,'both'])await fixture(async f=>{
 const decoder=await retainedVideo(f),original=Error('controlled original uncertainty'),rejected=Promise.reject(original);void rejected.catch(()=>{});
 const owner=f.publicOwner(decoder,rejected),canvas=originalCanvas();let completed=false,reads=0;
 owner.signaling={async recoverReplacementCompletion(error){assert.equal(error,original);reads++;return completed?{recordId:'original-root',generation:2}:undefined;}};
 await assert.rejects(f.render(owner,canvas,{timeoutMs:200},mode),{code:'conflict'});
 assert.equal(canvas.frames.length,0);assert.notEqual(f.elements[1].srcObject,null,'Original target retains its private picture');
 assert.equal(owner.preparedDecoder,decoder);f.frame(40);completed=true;
 const work=f.render(owner,canvas,{timeoutMs:200},mode);await tick();f.frame(40);assert.equal(canvas.frames.length,0);f.frame(41);const render=await work;
 try{assert.equal(reads,2);assert.deepEqual(canvas.frames,[f.elements[1]]);}finally{render.close();await render.retirement;await render.finished;}
},{video:true});});


test('replacement audio-plus-video preserves synchronous public graph discovery before its video completion wait',()=>fixture(async f=>{
 const decoder=await retainedVideo(f),root=originalDeferred(),owner=f.publicOwner(decoder,root.promise),canvas=originalCanvas();
 const work=f.render(owner,canvas,{timeoutMs:200},'both');
 // This is the actual renderer under the observer's SAME synchronous call
 // boundary, not a global discovery token retained across any await.
 const connects=f.connectObservations.filter(value=>value.source===f.sources[0]&&value.synchronousRenderHook);
 assert.equal(connects.length,1);assert.equal(connects[0].node,f.gains[1]);
 await tick();assert.equal(canvas.frames.length,0);f.frame(40);root.resolve({recordId:'original-root',generation:2});await tick();f.frame(41);
 const render=await work;try{assert.deepEqual(canvas.frames,[f.elements[1]]);}finally{render.close();await render.retirement;await render.finished;}
},{video:true}));


test('timeout or cancellation before first public video paint cannot bypass its floor on a later renderer',async()=>{
 for(const phase of ['timeout','cancel'])await fixture(async f=>{
  const decoder=await retainedVideo(f),owner=f.publicOwner(decoder,Promise.resolve({recordId:'original-root',generation:2})),first=originalCanvas(),abort=new AbortController();
  const failed=f.render(owner,first,{timeoutMs:phase==='timeout'?15:200,signal:abort.signal});await tick();
  assert.equal(owner.replacementVideoSource,f.picture.track);assert.notEqual(owner.replacementVideoPresented,true);
  f.frame(1);const lateFirst=f.pendingFrame();assert.equal(first.frames.length,0);if(phase==='cancel')abort.abort();
  await assert.rejects(failed,{code:phase==='timeout'?'timeout':'cancelled'});await owner.tasks.join();
  assert.equal(first.frames.length,0);assert.equal(f.elements[1].srcObject,null);assert.notEqual(owner.replacementVideoPresented,true);
  const second=originalCanvas(),retry=f.render(owner,second);await tick();assert.equal(f.elements.length,3);
  const current=f.pendingFrame();lateFirst(20,{width:640,height:360,mediaTime:0,presentedFrames:999});
  assert.equal(f.pendingFrame(),current);assert.equal(second.frames.length,0,'Late failed-render callback cannot paint or replace retry owner');
  f.frame(1);await tick();assert.equal(second.frames.length,0,'First NEW-element callback is baseline only');
  f.frame(1);assert.equal(second.frames.length,0,'Cached equal counter cannot bypass floor');
  assert.notEqual(owner.replacementVideoPresented,true);f.frame(2);const render=await retry;
  try{assert.deepEqual(second.frames,[f.elements[2]]);assert.equal(owner.replacementVideoPresented,true);}
  finally{render.close();await render.retirement;await render.finished;}
  assert.equal(f.elements[1].pauses,1);assert.equal(f.elements[2].pauses,1);assert.equal(f.pendingFrame(),undefined);
 },{video:true});
});

test('a missing initial prepared lease or another track cannot manufacture replacement retry eligibility',async()=>{
 for(const phase of ['missing-initial','other-track'])await fixture(async f=>{
  const decoder=await retainedVideo(f),owner=f.publicOwner(phase==='missing-initial'?undefined:decoder,Promise.resolve({recordId:'original-root',generation:2})),canvas=originalCanvas();
  if(phase==='missing-initial')await decoder.release();
  else{owner.replacementVideoSource={...f.picture.track};await decoder.release();}
  await assert.rejects(f.render(owner,canvas),{code:'unsupported-media-profile'});
  assert.equal(canvas.frames.length,0);assert.notEqual(owner.replacementVideoPresented,true);assert.equal(f.elements.length,2);
 },{video:true});
});


test('connection retirement clears exact-track retry roots before and after the first successful public video',async()=>{
 for(const phase of ['before-firstpaint','after-firstpaint'])await fixture(async f=>{
  const decoder=await retainedVideo(f),owner=f.publicOwner(decoder,Promise.resolve({recordId:'original-root',generation:2})),canvas=originalCanvas();
  const work=f.render(owner,canvas);await tick();
  let render;if(phase==='after-firstpaint'){f.frame(2);render=await work;assert.equal(owner.replacementVideoPresented,true);}
  const late=f.pendingFrame(),paints=canvas.frames.length;
  assert.equal(owner.replacementVideoSource,f.picture.track);owner.close();
  assert.equal(owner.replacementVideoSource,undefined);assert.equal(owner.replacementVideoPresented,false);assert.equal(owner.replacementCompletion,undefined);
  assert.equal(owner.remote.size,0);assert.equal(f.peer.connectionState,'closed');
  assert.equal(f.picture.track.stops,1);assert.equal(f.audio.track.stops,1);
  late(20,{width:640,height:360,mediaTime:0,presentedFrames:999});assert.equal(canvas.frames.length,paints);assert.equal(f.pendingFrame(),undefined);
  if(render){await render.retirement;await render.finished;}
  else await assert.rejects(work,error=>['closed','cancelled'].includes(error.code));
  await owner.shutdown();assert.equal(f.picture.track.stops,1);assert.equal(f.audio.track.stops,1);
 },{video:true});
 await fixture(async f=>{
  const decoder=await retainedVideo(f),original=Error('original uncertain completion'),rejected=Promise.reject(original);void rejected.catch(()=>{});
  const owner=f.publicOwner(decoder,rejected),late=originalDeferred(),canvas=originalCanvas();
  owner.signaling={recoverReplacementCompletion(){return late.promise;},close(){},async shutdown(){}};
  const work=f.render(owner,canvas);await tick();owner.close();
  await assert.rejects(work,error=>['closed','cancelled'].includes(error.code));
  late.resolve({recordId:'original-root',generation:2});await tick();await owner.shutdown();
  assert.equal(owner.replacementCompletion,undefined,'Late recovered receipt cannot repopulate closed connection');
  assert.equal(owner.replacementVideoSource,undefined);assert.equal(canvas.frames.length,0);
 },{video:true});
});
