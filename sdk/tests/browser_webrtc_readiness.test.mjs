import assert from 'node:assert/strict';
import {test} from 'node:test';
import {observeWebRtcDecodedReadiness} from '../typescript/dist/browser_webrtc_readiness.js';
import {BrowserDeadline} from '../typescript/dist/browser_deadline.js';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
async function fixture(run,{video=false}={}) {
 const saved=new Map(),nodes=[],elements=[],contexts=[];let callback,next=0;
 class Node{gain={value:1};disconnects=0;connect(){}disconnect(){this.disconnects++;}}
 class Context{sampleRate=48000;destination={};audioWorklet={async addModule(){}};closes=0;constructor(){contexts.push(this);}createMediaStreamSource(){return new Node();}createGain(){return new Node();}async resume(){}async close(){this.closes++;}}
 class Worklet extends Node{port={onmessage:null,sent:[],closed:false,postMessage(value){this.sent.push(value);},close(){this.closed=true;}};constructor(){super();nodes.push(this);}}
 class Stream{constructor(tracks){this.tracks=tracks;}}
 const values={AudioContext:Context,AudioWorkletNode:Worklet,MediaStream:Stream,document:{createElement(kind){const element={kind,muted:false,srcObject:null,pauses:0,async play(){},pause(){this.pauses++;},requestVideoFrameCallback(fn){callback=fn;return++next;},cancelVideoFrameCallback(){callback=undefined;}};elements.push(element);return element;}}};
 for(const [name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});}
 const audio={id:'audio',track:{id:'rtc-audio',kind:'audio',readyState:'live'}},picture=video?{id:'receive',track:{id:'rtc-video',kind:'video',readyState:'live'}}:undefined;
 const stats={audio:{type:'inbound-rtp',kind:'audio',trackIdentifier:'rtc-audio',totalSamplesReceived:0,concealedSamples:0},video:{type:'inbound-rtp',kind:'video',trackIdentifier:'rtc-video',framesDecoded:0,keyFramesDecoded:0}};
 const peer={connectionState:'connected',async getStats(){return new Map(Object.entries(structuredClone(stats)));}},abort=new AbortController(),tasks=new Set();
 const track=(promise)=>{tasks.add(promise);void promise.finally(()=>tasks.delete(promise)).catch(()=>{});return promise;};
 let work;
 try{await run({audio,picture,stats,peer,abort,nodes,elements,contexts,
  begin(){work=observeWebRtcDecodedReadiness(peer,audio,picture,new BrowserDeadline(500),abort.signal,track);void work.catch(()=>{});return work;},
  async started(){while(!nodes.length||(video&&elements.length<2))await tick();},
  pcm(values=new Float32Array(960)){nodes[0].port.onmessage({data:{epoch:0,timestampUs:1000,samples:values}});},
  frame(){const fn=callback;assert.equal(typeof fn,'function');fn(10,{width:640,height:360,mediaTime:1,presentedFrames:1});},
 });}finally{abort.abort();await work?.catch(()=>{});await Promise.allSettled(tasks);for(const[name,descriptor]of saved){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}}
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
