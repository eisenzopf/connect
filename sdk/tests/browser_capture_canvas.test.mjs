import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserMediaCapture} from '../typescript/dist/browser_media_capture.js';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
import {BrowserTasks} from '../typescript/dist/browser_tasks.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const tick=()=>new Promise(resolve=>setImmediate(resolve));
const sdp='v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=Original canvas capture\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendrecv\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=sendrecv\r\na=mid:video\r\na=rtpmap:96 VP8/90000\r\n';

// Browser API/ownership unit fixture only: no decoded-media or Firefox pass.
async function environment(run){
 const originals=new Map(),tracks=[],watchers=[],canvases=[],videos=[],peers=[],contexts=[],worklets=[],workers=[],owners=[];
 let permissions=0;
 class Track extends EventTarget{
  readyState='live';enabled=true;stops=0;constraints=0;
  constructor(kind='video',settings={width:640,height:360}){super();this.kind=kind;this.settings=settings;tracks.push(this);}
  getSettings(){return this.settings;}
  async applyConstraints(){this.constraints++;throw new DOMException('Canvas source does not constrain','OverconstrainedError');}
  stop(){this.stops++;this.readyState='ended';}
 }
 class Stream{
  constructor(tracks=[]){this.tracks=tracks;}
  getTracks(){return this.tracks;}getAudioTracks(){return this.tracks.filter(t=>t.kind==='audio');}getVideoTracks(){return this.tracks.filter(t=>t.kind==='video');}
 }
 class Canvas{
  width=640;height=360;requests=[];
  constructor(){canvases.push(this);}
  captureStream(rate){this.requests.push(rate);return new Stream([new Track('video',{width:this.width,height:this.height})]);}
  getContext(){return {drawImage(){}};}
 }
 class Observer{
  disconnected=false;
  constructor(fn){this.fn=fn;watchers.push(this);}observe(canvas,options){this.canvas=canvas;this.options=options;}
  disconnect(){this.disconnected=true;}
 }
 class Video{
  constructor(){videos.push(this);}async play(){}pause(){this.paused=true;}
  requestVideoFrameCallback(fn){this.callback=fn;return 1;}cancelVideoFrameCallback(){this.callback=undefined;}
 }
 class Node{
  numberOfOutputs=1;edges=new Set();gain={value:1};
  constructor(context){this.context=context;}connect(other){assert.equal(this.context,other.context);this.edges.add(other);return other;}
  disconnect(other){if(other)assert.ok(this.edges.delete(other));else this.edges.clear();}
 }
 class Context extends EventTarget{
  sampleRate=48000;state='running';currentTime=1;closes=0;loads=0;moduleGate=Promise.resolve();
  constructor(){super();contexts.push(this);this.destination=new Node(this);this.audioWorklet={addModule:()=>{this.loads++;return this.moduleGate;}};}
  createGain(){return new Node(this);}createMediaStreamDestination(){const node=new Node(this);node.stream=new Stream([new Track('audio',{})]);return node;}
  async close(){this.closes++;this.state='closed';this.dispatchEvent(new Event('statechange'));}
 }
 class Worklet extends Node{
  constructor(context){super(context);worklets.push(this);this.port={postMessage(){},close(){},onmessage:null};}
 }
 class Peer extends EventTarget{
  connectionState='new';iceConnectionState='new';iceGatheringState='complete';transceivers=[];
  constructor(configuration){super();this.configuration=configuration;peers.push(this);}getConfiguration(){return this.configuration??{};}
  addTransceiver(kind,options){const parameters={encodings:structuredClone(options.sendEncodings??[{}])};
   const sender={track:null,transform:null,getParameters:()=>structuredClone(parameters),async setParameters(value){Object.assign(parameters,structuredClone(value));},async replaceTrack(track){this.track=track;}};
   const tx={sender,receiver:{track:new Track(kind,{}),jitterBufferTarget:0},setCodecPreferences(){}};this.transceivers.push(tx);return tx;}
  getTransceivers(){return this.transceivers;}getSenders(){return this.transceivers.map(tx=>tx.sender);}
  async createOffer(){return {type:'offer',sdp};}async setLocalDescription(value){this.localDescription=value;}
  async setRemoteDescription(value){this.remoteDescription=value;for(const transceiver of this.transceivers)this.ontrack?.({track:transceiver.receiver.track,transceiver});this.connectionState=this.iceConnectionState='connected';this.dispatchEvent(new Event('connectionstatechange'));}
  close(){this.connectionState='closed';this.dispatchEvent(new Event('connectionstatechange'));}
 }
 class Worker{constructor(){workers.push(this);}terminate(){this.closed=true;}}
 const document=Object.assign(new EventTarget(),{visibilityState:'visible',createElement:kind=>kind==='video'?new Video():new Canvas()});
 const values={MediaStream:Stream,HTMLCanvasElement:Canvas,MutationObserver:Observer,HTMLVideoElement:Video,
  VideoFrame:class{constructor(_canvas,{timestamp}){this.timestamp=timestamp;}close(){this.closed=true;}},
  AudioNode:Node,AudioContext:Context,AudioWorkletNode:Worklet,RTCPeerConnection:Peer,Worker,
  RTCRtpScriptTransform:class{},RTCRtpSender:{getCapabilities:kind=>({codecs:[{mimeType:kind==='audio'?'audio/opus':'video/vp8',clockRate:kind==='audio'?48000:90000,channels:2}]})},document,
  navigator:{mediaDevices:{getUserMedia(){permissions++;throw Error('Unexpected device permission');},getDisplayMedia(){permissions++;throw Error('Unexpected screen permission');}}}};
 for(const [name,value]of Object.entries(values)){originals.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});}
 function wt(){const listeners=new Set(),tasks=new BrowserTasks(),frames=[];
  const owner={streams:[{streamId:'audio',kind:'audio',direction:'sendrecv'},{streamId:'video',kind:'video',direction:'sendrecv'}],
   onClose(fn){listeners.add(fn);return()=>listeners.delete(fn);},ownCleanup(work){tasks.track(work,true);},encodeAudio(){return true;},
   encodeVideo(id,frame){frames.push({id,timestamp:frame.timestamp});frame.close();return true;},
   close(){for(const fn of [...listeners])fn();},async shutdown(){this.close();await tasks.join();}};owners.push(owner);return {owner,listeners,frames,tasks};}
 try{await fixture(async f=>{const client=await BrowserClient.open(f.options);
  const open=async id=>{const begun=BrowserRtcConnection.begin(client,{conversationId:'room',sessionId:'session',connectionId:id,operationId:id+'-offer',audio:'audio',video:'video'},'recipient',{}, {timeoutMs:1000});owners.push(begun.owner);return begun.ready;};
  try{return await run({Track,Stream,Canvas,Context,Node,tracks,watchers,canvases,videos,peers,contexts,worklets,workers,wt,open,client,permissions:()=>permissions});}
  finally{await Promise.all(owners.map(owner=>owner.shutdown()));await client.shutdown();}
 },{handler:async({peer,frame})=>{if(frame.type!=='connection.offer')return false;
  await peer.emit('connection.answer',{substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp},capabilities:{audio_codecs:[{name:'opus',params:{sample_rate:48000,channels:1}}],video_codecs:[{name:'vp8',params:{sample_rate:90000,channels:0}}]},streams_answered:[{id:'audio',kind:'audio',direction:'sendrecv',codec:{name:'opus',params:{sample_rate:48000,channels:1}}},{id:'video',kind:'video',direction:'sendrecv',codec:{name:'vp8',params:{sample_rate:90000,channels:0}}}]},frame.id,frame);return true;}});}
 finally{for(const[name,value]of originals)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}

test('RTC explicit canvas accepts Firefox-style source settings through original bounded senders',()=>environment(async e=>{
 const owner=await e.open('canvas'),canvas=new e.Canvas(),capture=await owner.capture({video:'camera',videoCanvas:canvas},{timeoutMs:100});
 const sender=e.peers[0].getSenders()[1],track=sender.track;
 assert.equal(track.constraints,0);assert.deepEqual(track.getSettings(),{width:640,height:360});assert.deepEqual(canvas.requests,[undefined]);
 assert.equal(sender.getParameters().encodings[0].maxFramerate,30);assert.equal(sender.getParameters().encodings[0].maxBitrate,1000000);
 assert.equal(sender.getParameters().degradationPreference,'maintain-resolution');
 capture.pause();assert.equal(track.enabled,false);await capture.resume();assert.equal(track.enabled,true);assert.deepEqual(canvas.requests,[undefined]);
 capture.close();assert.equal(track.readyState,'ended');assert.equal(track.stops,1);assert.ok(e.watchers.every(w=>w.disconnected));assert.equal(canvas.width,640);
}));
test('RTC arbitrary supplied canvas-like tracks still fail their actual constraints',()=>environment(async e=>{
 const owner=await e.open('generic'),track=new e.Track();
 await assert.rejects(()=>owner.capture({video:'camera',stream:new e.Stream([track])},{timeoutMs:100}),{code:'media'});
 assert.equal(track.constraints,1);assert.equal(track.readyState,'ended');assert.equal(e.peers[0].getSenders()[1].track,null);assert.equal(e.watchers.length,0);
}));
test('two RTC canvas captures retain distinct tracks and retiring the old owner leaves the original source and replacement live',()=>environment(async e=>{
 const canvas=new e.Canvas(),context=new e.Context(),node=new e.Node(context),old=await e.open('old'),next=await e.open('next');
 const first=await old.capture({audio:true,audioNode:node,video:'camera',videoCanvas:canvas},{timeoutMs:100});
 const second=await next.capture({audio:true,audioNode:node,video:'camera',videoCanvas:canvas},{timeoutMs:100});
 const a=e.peers[0].getSenders()[1].track,b=e.peers[1].getSenders()[1].track;assert.notEqual(a,b);assert.equal(node.edges.size,2);
 await old.shutdown();assert.equal(first.closed,true);assert.equal(a.readyState,'ended');assert.equal(b.readyState,'live');assert.equal(second.closed,false);
 assert.equal(node.edges.size,1);assert.equal(context.state,'running');assert.equal(context.closes,0);assert.deepEqual(canvas.requests,[undefined,undefined]);
 second.close();assert.equal(b.readyState,'ended');assert.equal(node.edges.size,0);assert.equal(context.closes,0);
}));
test('canonical RTC resize closes only the derived capture and preserves an independent capture',()=>environment(async e=>{
 const first=await e.open('resize'),next=await e.open('independent'),canvas=new e.Canvas(),other=new e.Canvas(),states=[];
 const capture=await first.capture({video:'camera',videoCanvas:canvas,state:(state,error)=>states.push({state,code:error?.code})},{timeoutMs:100});
 const separate=await next.capture({video:'camera',videoCanvas:other},{timeoutMs:100});
 canvas.width=320;e.watchers.find(w=>w.canvas===canvas).fn();
 assert.equal(capture.closed,true);assert.equal(states.at(-1).code,'unsupported-media-profile');assert.equal(separate.closed,false);
 assert.equal(e.peers[0].getSenders()[1].track.readyState,'ended');assert.equal(e.peers[1].getSenders()[1].track.readyState,'live');
}));
test('canvas selections reject wrong dimensions, device/screen ambiguity and supplied video before ownership',()=>environment(async e=>{
 const rtc=await e.open('invalid'),wt=e.wt(),canvas=new e.Canvas(),unrelated=new e.Track(),stream=new e.Stream([unrelated]);
 for(const extra of[{stream},{videoDeviceId:'camera'},{video:'screen'}]){
  await assert.rejects(()=>rtc.capture({video:'camera',videoCanvas:canvas,...extra},{timeoutMs:100}),{code:'invalid-request'});
  await assert.rejects(()=>BrowserMediaCapture.open(wt.owner,{videoStreamId:'video',videoSource:'camera',videoCanvas:canvas,...(extra.video?{videoSource:extra.video}:extra)}),{code:'invalid-request'});
 }
 for(const width of[0,641]){canvas.width=width;
  await assert.rejects(()=>rtc.capture({video:'camera',videoCanvas:canvas},{timeoutMs:100}),{code:'unsupported-media-profile'});
  await assert.rejects(()=>BrowserMediaCapture.open(wt.owner,{videoStreamId:'video',videoCanvas:canvas}),{code:'unsupported-media-profile'});
 }
 assert.equal(unrelated.readyState,'live');assert.deepEqual(canvas.requests,[]);assert.equal(e.watchers.length,0);assert.equal(wt.listeners.size,0);
}));
test('WT explicit canvas retains original decoded frame timing and normal capture retirement',()=>environment(async e=>{
 const c=e.wt(),canvas=new e.Canvas(),capture=await BrowserMediaCapture.open(c.owner,{videoStreamId:'video',videoCanvas:canvas});
 const track=e.videos[0].srcObject.getVideoTracks()[0];assert.equal(track.constraints,0);
 e.videos[0].callback(1000,{captureTime:1.5,mediaTime:9,presentationTime:1000});
 assert.deepEqual(c.frames,[{id:'video',timestamp:1500}]);capture.close();
 assert.equal(track.readyState,'ended');assert.equal(track.stops,1);assert.equal(e.videos[0].srcObject,null);assert.equal(e.videos[0].callback,undefined);
 assert.equal(c.listeners.size,0);assert.ok(e.watchers.every(w=>w.disconnected));assert.deepEqual(canvas.requests,[undefined]);
}));
test('cancelled canvas/audio module waiter joins without stopping another capture or the caller source',()=>environment(async e=>{
 const context=new e.Context(),node=new e.Node(context),canvas=new e.Canvas(),module=deferred(),first=e.wt(),next=e.wt(),abort=new AbortController();context.moduleGate=module.promise;
 const old=BrowserMediaCapture.open(first.owner,{audioStreamId:'audio',audioNode:node,videoStreamId:'video',videoCanvas:canvas,signal:abort.signal,timeoutMs:1000});void old.catch(()=>{});
 const replacement=BrowserMediaCapture.open(next.owner,{audioStreamId:'audio',audioNode:node,videoStreamId:'video',videoCanvas:canvas,timeoutMs:1000});void replacement.catch(()=>{});
 try{await tick();assert.equal(context.loads,1);const [a,b]=e.tracks.filter(t=>t.kind==='video');abort.abort();await assert.rejects(old,{code:'cancelled'});
  await first.owner.shutdown();assert.equal(a.readyState,'ended');assert.equal(b.readyState,'live');assert.equal(context.state,'running');assert.equal(context.closes,0);assert.equal(e.worklets.length,0);
  module.resolve();const capture=await replacement;assert.equal(e.worklets.length,1);assert.equal(capture.closed,false);assert.equal(node.edges.size,1);
  capture.close();assert.equal(b.readyState,'ended');assert.equal(node.edges.size,0);assert.equal(context.closes,0);assert.ok(e.watchers.every(w=>w.disconnected));
 }finally{module.resolve();next.owner.close();await replacement.catch(()=>{});}
}));

test('missing generic track frameRate is still refused after successful constraints',()=>environment(async e=>{
 const owner=await e.open('unknown-settings'),track=new e.Track();track.applyConstraints=async()=>{track.constraints++;};
 await assert.rejects(()=>owner.capture({video:'camera',stream:new e.Stream([track])},{timeoutMs:100}),{code:'unsupported-media-profile'});
 assert.equal(track.constraints,1);assert.equal(track.readyState,'ended');assert.equal(e.peers[0].getSenders()[1].track,null);
}));
test('resize during shared audio-module loading retires the canvas and cannot attach after late module completion',()=>environment(async e=>{
 const context=new e.Context(),node=new e.Node(context),canvas=new e.Canvas(),module=deferred(),c=e.wt();context.moduleGate=module.promise;
 const pending=BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:node,videoStreamId:'video',videoCanvas:canvas,timeoutMs:1000});void pending.catch(()=>{});
 await tick();canvas.height=180;e.watchers[0].fn();await assert.rejects(pending,{code:'unsupported-media-profile'});
 await c.owner.shutdown();assert.equal(context.closes,0);assert.equal(e.worklets.length,0);assert.equal(e.tracks[0].readyState,'ended');
 module.resolve();await tick();assert.equal(e.worklets.length,0);assert.equal(node.edges.size,0);assert.equal(e.videos.length,0);assert.ok(e.watchers.every(w=>w.disconnected));
}));
