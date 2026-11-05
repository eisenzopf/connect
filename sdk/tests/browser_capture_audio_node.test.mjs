import {BrowserTasks} from '../typescript/dist/browser_tasks.js';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaCapture} from '../typescript/dist/browser_media_capture.js';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
async function graphEnvironment(run){
 const originals=new Map(),contexts=[],worklets=[];let permissions=0;
 class Track extends EventTarget{kind='audio';readyState='live';enabled=true;stop(){this.readyState='ended';}}
 class Stream{constructor(tracks){this.tracks=tracks;}getTracks(){return this.tracks;}getAudioTracks(){return this.tracks.filter(t=>t.kind==='audio');}getVideoTracks(){return this.tracks.filter(t=>t.kind==='video');}}
 class Node{numberOfOutputs=1;edges=new Set();gain={value:1};constructor(context){this.context=context;}
  connect(other){assert.equal(other.context,this.context);this.edges.add(other);return other;}
  disconnect(other){assert.ok(other,'Caller source must never receive disconnect-all');assert.ok(this.edges.delete(other),'Only an owned existing edge can disconnect');}}
 class OwnedNode extends Node{disconnect(other){if(other)super.disconnect(other);else this.edges.clear();}}
 class Context extends EventTarget{
  sampleRate=48000;state='running';currentTime=12.5;closes=0;suspends=0;resumes=0;loads=0;moduleGate=Promise.resolve();
  constructor(){super();contexts.push(this);this.destination=new OwnedNode(this);this.audioWorklet={addModule:()=>{this.loads++;return this.moduleGate;}};}
  createGain(){return new OwnedNode(this);}
  createMediaStreamSource(){throw Error('Graph input must not cross a MediaStream source bridge');}
  createMediaStreamDestination(){const node=new OwnedNode(this);node.stream=new Stream([new Track()]);return node;}
  async suspend(){this.suspends++;this.state='suspended';this.dispatchEvent(new Event('statechange'));}
  async resume(){this.resumes++;this.state='running';this.dispatchEvent(new Event('statechange'));}
  async close(){this.closes++;this.state='closed';this.dispatchEvent(new Event('statechange'));}}
 class Worklet extends OwnedNode{constructor(context,name){super(context);assert.equal(name,'connect-pcm-48k');this.messages=[];this.port={closed:false,onmessage:null,postMessage:v=>this.messages.push(v),close:()=>{this.port.closed=true;}};worklets.push(this);}}
 const document=Object.assign(new EventTarget(),{visibilityState:'visible'});
 const values={AudioNode:Node,AudioContext:Context,AudioWorkletNode:Worklet,MediaStream:Stream,document,
  navigator:{mediaDevices:{getUserMedia(){permissions++;throw Error('Graph input must not request personal devices');}}}};
 for(const [name,value]of Object.entries(values)){originals.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{value,writable:true,configurable:true});}
 const owners=[];
 function connection(){const listeners=new Set(),cleanup=[],encoded=[],owner={streams:[{streamId:'audio',kind:'audio',direction:'sendrecv'}],
  onClose(fn){listeners.add(fn);return()=>listeners.delete(fn);},ownCleanup(p){cleanup.push(p);},
  encodeAudio(id,samples,timestamp){encoded.push({id,samples,timestamp});return true;},
  close(){for(const fn of [...listeners])fn();}};owners.push(owner);return {owner,encoded,cleanup,listeners};}
 try{return await run({Context,Node,Track,Stream,contexts,worklets,connection,document,permissions:()=>permissions});}
 finally{for(const owner of owners)owner.close();for(const [name,value]of originals)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}
test('graph capture uses the original context/worklet, preserves actual PCM times, and retires only its own edge',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),other=new e.Node(context);source.connect(other);const c=e.connection();
 const capture=await BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source});
 assert.equal(e.contexts.length,1);assert.equal(context.loads,1);assert.equal(context.resumes,0);assert.equal(e.permissions(),0);
 const worklet=e.worklets[0],reset=worklet.messages.find(v=>v.type==='reset'),samples=Float32Array.from({length:960},(_,i)=>i/960);
 worklet.port.onmessage({data:{epoch:reset.epoch,timestampUs:987654,samples}});
 assert.equal(c.encoded[0].samples,samples);assert.equal(c.encoded[0].timestamp,987654);
 capture.pause();assert.equal(context.state,'running');assert.equal(context.suspends,0);assert.deepEqual([...source.edges],[other]);
 worklet.port.onmessage({data:{epoch:reset.epoch,timestampUs:1007654,samples}});assert.equal(c.encoded.length,1);
 await capture.resume();assert.equal(source.edges.has(worklet),true);assert.equal(context.resumes,0);
 capture.close();assert.deepEqual([...source.edges],[other]);assert.equal(context.closes,0);assert.equal(worklet.port.closed,true);assert.equal(c.listeners.size,0);
}));
test('concurrent prepared graph captures share module registration and old retirement leaves replacement live',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),gate=deferred();context.moduleGate=gate.promise;
 const first=e.connection(),second=e.connection();
 const old=BrowserMediaCapture.open(first.owner,{audioStreamId:'audio',audioNode:source});
 const next=BrowserMediaCapture.open(second.owner,{audioStreamId:'audio',audioNode:source});
 assert.equal(context.loads,1);first.owner.close();await assert.rejects(old,{code:'closed'});gate.resolve();
 const replacement=await next;assert.equal(e.worklets.length,1);assert.equal(source.edges.size,1);assert.equal(context.closes,0);
 replacement.close();await Promise.all([...first.cleanup,...second.cleanup]);assert.equal(source.edges.size,0);assert.equal(context.state,'running');
}));
test('cancelled and expired waiters join physical cleanup while the shared module remains unresolved',()=>graphEnvironment(async e=>{
 for(const mode of['cancel','expire']){
  const context=new e.Context(),source=new e.Node(context),gate=deferred(),first=e.connection(),second=e.connection(),tasks=new BrowserTasks(),abort=new AbortController();context.moduleGate=gate.promise;
  first.owner.ownCleanup=work=>tasks.track(work,true);
  const old=BrowserMediaCapture.open(first.owner,{audioStreamId:'audio',audioNode:source,timeoutMs:mode==='expire'?5:1000,signal:abort.signal});
  const next=BrowserMediaCapture.open(second.owner,{audioStreamId:'audio',audioNode:source,timeoutMs:1000});void next.catch(()=>{});
  try{
   if(mode==='cancel')abort.abort();await assert.rejects(old,{code:mode==='cancel'?'cancelled':'timeout'});
   let timer;const joined=await Promise.race([tasks.join().then(()=>true),new Promise(resolve=>{timer=setTimeout(()=>resolve(false),25);})]);clearTimeout(timer);
   assert.equal(joined,true,'Original physical tasks must join before the caller module resolves');
   assert.equal(context.loads,1);assert.equal(context.state,'running');assert.equal(context.closes,0);assert.equal(e.worklets.filter(node=>node.context===context).length,0);
   gate.resolve();const replacement=await next;assert.equal(e.worklets.filter(node=>node.context===context).length,1);assert.equal(source.edges.size,1);
   replacement.close();assert.equal(source.edges.size,0);
  }finally{gate.resolve();second.owner.close();await next.catch(()=>{});await tasks.join();}
 }
}));
test('failed shared module registration can be retried without leaving graph edges or closing the caller',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),c=e.connection(),gate=deferred();context.moduleGate=gate.promise;
 const first=BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source});gate.reject(Error('original module failed'));
 await assert.rejects(first,{code:'media-capture'});await Promise.all(c.cleanup);assert.equal(source.edges.size,0);assert.equal(context.closes,0);
 context.moduleGate=Promise.resolve();const next=await BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source});
 assert.equal(context.loads,2);assert.equal(source.edges.size,1);next.close();
}));
test('caller suspension pauses only graph capture and caller closure closes that capture',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),c=e.connection(),states=[];
 const capture=await BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source,state:v=>states.push(v)});
 await context.suspend();assert.equal(source.edges.size,0);assert.equal(capture.closed,false);assert.equal(states.at(-1),'paused');
 await assert.rejects(()=>capture.resume(),{code:'media-capture'});assert.equal(context.resumes,0);
 await context.resume();assert.equal(source.edges.size,0,'Caller context resume grants no capture resume');
 await capture.resume();assert.equal(source.edges.size,1);await context.close();assert.equal(capture.closed,true);assert.equal(context.closes,1);assert.equal(source.edges.size,0);
}));
test('close during shared module loading prevents late node creation without closing caller context',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),gate=deferred(),c=e.connection();context.moduleGate=gate.promise;
 const pending=BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source});await context.close();
 await assert.rejects(pending,{code:'media-capture'});gate.resolve();await Promise.all(c.cleanup);
 assert.equal(e.worklets.length,0);assert.equal(source.edges.size,0);assert.equal(context.closes,1);
}));
test('a graph capture deadline does not restart or close a shared registration',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),gate=deferred(),c=e.connection();context.moduleGate=gate.promise;
 await assert.rejects(()=>BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source,timeoutMs:5}),{code:'timeout'});
 assert.equal(context.closes,0);assert.equal(context.loads,1);gate.resolve();await Promise.all(c.cleanup);assert.equal(e.worklets.length,0);
 const next=await BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source});assert.equal(context.loads,1);next.close();
}));
test('ordinary owned-device context startup suspension is not mistaken for an explicit capture pause',()=>graphEnvironment(async e=>{
 class DeviceContext extends e.Context{
  constructor(){super();this.state='suspended';this.audioWorklet={addModule:()=>{this.loads++;queueMicrotask(()=>this.dispatchEvent(new Event('statechange')));return Promise.resolve();}};}
  createMediaStreamSource(){const node=new e.Node(this);node.disconnect=function(target){if(target)this.edges.delete(target);else this.edges.clear();};return node;}
 }
 Object.defineProperty(globalThis,'AudioContext',{value:DeviceContext,writable:true,configurable:true});
 const c=e.connection(),track=new e.Track(),states=[];
 const capture=await BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',stream:new e.Stream([track]),state:value=>states.push(value)});
 assert.equal(states.at(-1),'active');assert.equal(e.contexts[0].suspends,0);assert.equal(e.contexts[0].resumes,1);
 capture.close();await Promise.all(c.cleanup);assert.equal(track.readyState,'ended');assert.equal(e.contexts[0].closes,1);
}));
test('graph sources reject ambiguous tracks/devices and unsupported contexts before taking input ownership',()=>graphEnvironment(async e=>{
 const context=new e.Context(),source=new e.Node(context),c=e.connection(),track=new e.Track();
 for(const addition of[{audioDeviceId:'microphone'},{stream:new e.Stream([track])}])
  await assert.rejects(()=>BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source,...addition}),{code:'invalid-request'});
 assert.equal(track.readyState,'live');assert.equal(source.edges.size,0);assert.equal(context.loads,0);
 for(const [key,value]of[['sampleRate',44100],['state','suspended'],['state','closed']]){const before=context[key];context[key]=value;
  await assert.rejects(()=>BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source}),{code:'unsupported-media-profile'});context[key]=before;}
 await assert.rejects(()=>BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:new e.Node({sampleRate:48000,state:'running'})}),{code:'unsupported-media-profile'});
 source.numberOfOutputs=0;await assert.rejects(()=>BrowserMediaCapture.open(c.owner,{audioStreamId:'audio',audioNode:source}),{code:'unsupported-media-profile'});
 assert.equal(e.permissions(),0);assert.equal(c.listeners.size,0);
}));

const sdp='v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=Owned fixture\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendrecv\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\n';
async function rtcGraph(run){return graphEnvironment(async e=>{
 const originals=new Map(),peers=[],begun=[];
 class Peer extends EventTarget{connectionState='new';iceConnectionState='new';iceGatheringState='complete';transceivers=[];
  constructor(configuration){super();this.configuration=configuration;peers.push(this);}getConfiguration(){return this.configuration??{};}
  addTransceiver(){const tx={sender:{track:null,async replaceTrack(track){this.track=track;}},receiver:{track:new e.Track(),jitterBufferTarget:0},setCodecPreferences(){}};this.transceivers.push(tx);return tx;}
  getTransceivers(){return this.transceivers;}async createOffer(){return {type:'offer',sdp};}async setLocalDescription(v){this.localDescription=v;}
  async setRemoteDescription(v){this.remoteDescription=v;for(const tx of this.transceivers)this.ontrack?.({track:tx.receiver.track,transceiver:tx});this.connectionState=this.iceConnectionState='connected';this.dispatchEvent(new Event('connectionstatechange'));}
  close(){this.connectionState='closed';this.dispatchEvent(new Event('connectionstatechange'));}}
 for(const [key,value]of Object.entries({RTCPeerConnection:Peer,RTCRtpSender:{getCapabilities:()=>({codecs:[{mimeType:'audio/opus',clockRate:48000,channels:2}]})}})){originals.set(key,Object.getOwnPropertyDescriptor(globalThis,key));Object.defineProperty(globalThis,key,{value,writable:true,configurable:true});}
 try{return await fixture(async f=>{const client=await BrowserClient.open(f.options);
  async function open(id){const value=BrowserRtcConnection.begin(client,{conversationId:'room',sessionId:'session',connectionId:id,operationId:id+'-offer',audio:'audio'},'recipient',{}, {timeoutMs:1000});begun.push(value);return value.ready;}
  try{return await run({...e,client,peers,open,f});}finally{for(const value of begun)value.owner.close();client.close();}
 },{handler:async({peer,frame})=>{if(frame.type!=='connection.offer')return false;await peer.emit('connection.answer',{substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp},capabilities:{audio_codecs:[{name:'opus',params:{sample_rate:48000,channels:1}}]},streams_answered:[{id:'audio',kind:'audio',direction:'sendrecv',codec:{name:'opus',params:{sample_rate:48000,channels:1}}}]},frame.id,frame);return true;}});}
 finally{for(const [key,value]of originals)if(value)Object.defineProperty(globalThis,key,value);else delete globalThis[key];}
});}
test('RTC graph captures own separate same-context destinations and old physical retirement preserves the replacement edge',()=>rtcGraph(async e=>{
 const context=new e.Context(),source=new e.Node(context),external=new e.Node(context);source.connect(external);
 const first=await e.open('old'),second=await e.open('next');
 const old=await first.capture({audio:true,audioNode:source},{timeoutMs:100}),next=await second.capture({audio:true,audioNode:source},{timeoutMs:100});
 const oldTrack=e.peers[0].transceivers[0].sender.track,nextTrack=e.peers[1].transceivers[0].sender.track;
 assert.notEqual(oldTrack,nextTrack);assert.equal(source.edges.size,3);assert.equal(e.permissions(),0);assert.equal(e.contexts.length,1);assert.equal(context.loads,0);
 first.close();assert.equal(old.closed,true);assert.equal(oldTrack.readyState,'ended');assert.equal(nextTrack.readyState,'live');assert.equal(source.edges.size,2);assert.equal(context.state,'running');
 next.pause();assert.equal(nextTrack.enabled,false);assert.deepEqual([...source.edges],[external]);assert.equal(context.suspends,0);
 await next.resume();assert.equal(nextTrack.enabled,true);assert.equal(source.edges.size,2);next.close();assert.equal(source.edges.size,1);assert.equal(context.closes,0);assert.equal(e.client.closed,false);
}));
test('RTC caller suspension requires explicit context and capture resumes and closure retires only owned tracks',()=>rtcGraph(async e=>{
 const context=new e.Context(),source=new e.Node(context),owner=await e.open('original'),capture=await owner.capture({audio:true,audioNode:source},{timeoutMs:100});
 const track=e.peers[0].transceivers[0].sender.track;await context.suspend();assert.equal(track.enabled,false);assert.equal(source.edges.size,0);
 await assert.rejects(()=>capture.resume(),{code:'media-capture'});await context.resume();assert.equal(track.enabled,false);await capture.resume();assert.equal(track.enabled,true);
 await context.close();assert.equal(capture.closed,true);assert.equal(track.readyState,'ended');assert.equal(context.closes,1);assert.equal(e.client.closed,false);
}));
test('RTC cancellation of uncertain graph replaceTrack closes its own physical and edge, never the source context',()=>rtcGraph(async e=>{
 const context=new e.Context(),source=new e.Node(context),owner=await e.open('original'),gate=deferred(),abort=new AbortController();
 const sender=e.peers[0].transceivers[0].sender;sender.replaceTrack=async track=>{await gate.promise;sender.track=track;};
 const pending=owner.capture({audio:true,audioNode:source},{timeoutMs:100,signal:abort.signal});await new Promise(resolve=>setTimeout(resolve,0));
 abort.abort();await assert.rejects(pending);gate.resolve();await Promise.resolve();assert.equal(source.edges.size,0);assert.equal(context.closes,0);
 assert.equal(e.peers[0].connectionState,'closed');assert.equal(e.client.closed,false);
}));
