import assert from 'node:assert/strict';
import {test} from 'node:test';
import {boundedCanvasVideoCapture} from '../typescript/dist/browser_video_capture.js';
import {BrowserVoiceSession} from '../typescript/dist/browser.js';

async function fixture(run){
 const names=['HTMLCanvasElement','MediaStream','MutationObserver'];
 const descriptors=new Map(names.map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]));
 const state={rates:[],stops:0,disconnects:0,errors:[],settings:{width:640,height:360}};
 const derived=Object.assign(new EventTarget(),{kind:'video',readyState:'live',enabled:true,getSettings:()=>state.settings,stop(){state.stops++;this.readyState='ended';}});
 const audio={kind:'audio',readyState:'live'};
 class Stream {constructor(tracks){if(state.streamFailure)throw Error('fixture construction failure');this.tracks=tracks;}getTracks(){return this.tracks;}getAudioTracks(){return this.tracks.filter(t=>t.kind==='audio');}getVideoTracks(){return this.tracks.filter(t=>t.kind==='video');}}
 class Canvas {width=640;height=360;captureStream(rate){state.rates.push(rate);return {getTracks:()=>state.badTracks??[derived],getVideoTracks:()=>state.badTracks??[derived]};}}
 class Observer {constructor(fn){state.changed=fn;}observe(canvas,options){state.observed={canvas,options};if(state.observeFailure)throw Error('fixture observer failure');}disconnect(){state.disconnects++;}}
 const globals={HTMLCanvasElement:Canvas,MediaStream:Stream,MutationObserver:Observer};
 for(const key of names)Object.defineProperty(globalThis,key,{configurable:true,value:globals[key]});
 Object.assign(state,{canvas:new Canvas(),audio,derived,input:new Stream([audio])});
 try{return await run(state);}finally{for(const [key,d]of descriptors)if(d)Object.defineProperty(globalThis,key,d);else delete globalThis[key];}
}
test('explicit canvas capture owns a bounded output without camera settings or input mutation',()=>fixture(f=>{
 const capture=boundedCanvasVideoCapture(f.input,f.canvas,e=>f.errors.push(e));
 assert.deepEqual(f.rates,[30]);assert.equal(capture.stream.getAudioTracks()[0],f.audio);assert.equal(capture.stream.getVideoTracks()[0],f.derived);
 assert.deepEqual(f.input.getTracks(),[f.audio]);assert.deepEqual(f.observed.options,{attributes:true,attributeFilter:['width','height']});
 capture.close();capture.close();assert.equal(f.stops,1);assert.equal(f.disconnects,1);assert.equal(f.errors.length,0);
}));
test('canvas intrinsic resize retires the derived track and reports failure once',()=>fixture(f=>{
 const capture=boundedCanvasVideoCapture(f.input,f.canvas,e=>f.errors.push(e));f.canvas.width=320;f.changed();
 assert.equal(f.stops,1);assert.equal(f.errors[0].code,'unsupported-media-profile');f.changed();capture.close();assert.equal(f.stops,1);assert.equal(f.errors.length,1);
}));
test('invalid canvas dimensions refuse before any output allocation',()=>fixture(f=>{
 for(const [width,height]of [[0,360],[641,360],[640,361],[NaN,360],[640,0]]){
   Object.assign(f.canvas,{width,height});assert.throws(()=>boundedCanvasVideoCapture(f.input,f.canvas,()=>{}),{code:'unsupported-media-profile'});
 }assert.deepEqual(f.rates,[]);assert.equal(f.stops,0);
}));
test('contradictory actual output settings refuse and release the newly owned track',()=>fixture(f=>{
 for(const settings of [{width:1280},{height:720},{frameRate:31},{frameRate:NaN},{frameRate:0}]){
   f.settings=settings;f.derived.readyState='live';assert.throws(()=>boundedCanvasVideoCapture(f.input,f.canvas,()=>{}),{code:'unsupported-media-profile'});
 }assert.equal(f.stops,5);
}));
test('partial observer or stream construction failure retires every derived resource',()=>fixture(f=>{
 f.observeFailure=true;assert.throws(()=>boundedCanvasVideoCapture(f.input,f.canvas,()=>{}),{code:'unsupported-media-profile'});assert.equal(f.stops,1);assert.equal(f.disconnects,1);
 f.observeFailure=false;f.derived.readyState='live';f.streamFailure=true;assert.throws(()=>boundedCanvasVideoCapture(f.input,f.canvas,()=>{}),{code:'unsupported-media-profile'});assert.equal(f.stops,2);assert.equal(f.disconnects,2);
}));
test('public mute affects original and derived owned tracks together',()=>fixture(f=>{
 const original={kind:'video',enabled:true};const owner=Object.create(BrowserVoiceSession.prototype);
 owner.ownedTracks=[original,f.audio];owner.localStream={getTracks:()=>[f.audio,f.derived]};
 owner.setMuted('video',true);assert.equal(original.enabled,false);assert.equal(f.derived.enabled,false);
 owner.setMuted('video',false);assert.equal(original.enabled,true);assert.equal(f.derived.enabled,true);
}));

test('invalid explicit canvas selections refuse before capture ownership',()=>fixture(async f=>{
 const client={controlTransport:'websocket',closed:false,onClose(){throw Error('must not own control/capture');}};
 const options={conversationId:'conversation',sessionId:'session',connectionId:'connection',operationId:'offer',expectedRevision:0,participantId:'participant',audioStreamId:'audio',videoStreamId:'video',video:true,videoCanvas:f.canvas,inputStream:f.input};
 for(const altered of [{videoDeviceId:'camera'},{inputStream:{getTracks:()=>[f.audio,f.derived]}}])
   await assert.rejects(BrowserVoiceSession.open(client,{...options,...altered}),{code:'invalid-request'});
 f.canvas.width=641;await assert.rejects(BrowserVoiceSession.open(client,options),{code:'invalid-request'});
 assert.deepEqual(f.rates,[]);assert.equal(f.stops,0);
}));
