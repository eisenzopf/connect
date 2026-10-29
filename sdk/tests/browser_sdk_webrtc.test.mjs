import assert from 'node:assert/strict';
import {test} from 'node:test';
import {fixture} from './fixtures/browser_control_peer.mjs';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
const sdp='v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=Owned fixture\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendrecv\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\n';
const input={conversationId:'room',sessionId:'session',connectionId:'connection',operationId:'original-offer',audio:'audio'};
class Track extends EventTarget {kind='audio';readyState='live';enabled=true;stop(){this.readyState='ended';}}
class Stream {constructor(tracks){this.tracks=tracks;}getTracks(){return this.tracks;}getAudioTracks(){return this.tracks.filter(t=>t.kind==='audio');}getVideoTracks(){return this.tracks.filter(t=>t.kind==='video');}}
function deferred(){let resolve,reject;return{promise:new Promise((yes,no)=>{resolve=yes;reject=no;}),get resolve(){return resolve;},get reject(){return reject;}};}
async function scenario(run,{pending=false,gathering=false}={}){
 const originals=new Map(),names=['RTCPeerConnection','RTCRtpSender','MediaStream','document','navigator'];let peer,permissions=0,permission=()=>Promise.reject(new DOMException('denied','NotAllowedError'));
 const elements=[];let playback=()=>Promise.resolve();
 const document=Object.assign(new EventTarget(),{visibilityState:'visible',createElement(kind){assert.equal(kind,'audio');const element={muted:false,srcObject:null,plays:0,pauses:0,play(){this.plays++;return playback(this);},pause(){this.pauses++;}};elements.push(element);return element;}});
 class Peer extends EventTarget {
  connectionState='new';iceConnectionState='new';iceGatheringState=gathering?'gathering':'complete';transceivers=[];closeCount=0;
  constructor(configuration){super();peer=this;this.configuration=configuration;}
  getConfiguration(){return this.configuration??{};}
  addTransceiver(kind){assert.equal(kind,'audio');const t={sender:{track:null,async replaceTrack(track){this.track=track;}},receiver:{track:new Track(),jitterBufferTarget:0},setCodecPreferences(v){this.codecs=v;}};this.transceivers.push(t);return t;}
  getTransceivers(){return this.transceivers;}
  async createOffer(){return{type:'offer',sdp};}
  async setLocalDescription(v){this.localDescription=v;}
  async setRemoteDescription(v){this.remoteDescription=v;for(const t of this.transceivers)this.ontrack?.({track:t.receiver.track,transceiver:t});if(!pending)this.connect();}
  connect(){this.connectionState='connected';this.iceConnectionState='connected';this.dispatchEvent(new Event('connectionstatechange'));}
  close(){this.closeCount++;this.connectionState='closed';this.dispatchEvent(new Event('connectionstatechange'));}
 }
 const values={RTCPeerConnection:Peer,RTCRtpSender:{getCapabilities:()=>({codecs:[{mimeType:'audio/opus',clockRate:48000,channels:2}]})},MediaStream:Stream,document,navigator:{mediaDevices:{getUserMedia(){permissions++;return permission();}}}};
 for(const name of names){originals.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value:values[name]});}
 try{return await fixture(async f=>{const client=await BrowserClient.open(f.options);let begun;try{begun=BrowserRtcConnection.begin(client,{...input},'recipient',{}, {timeoutMs:300});return await run({client,f,begun,elements,setPlayback:fn=>playback=fn,peer:()=>peer,permissions:()=>permissions,setPermission:fn=>permission=fn});}finally{begun?.owner.close();client.close();}},{handler:async({peer,frame})=>{
  if(frame.type!=='connection.offer')return false;
  await peer.emit('connection.answer',{substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp},capabilities:{audio_codecs:[{name:'opus',params:{sample_rate:48000,channels:1}}]},streams_answered:[{id:'audio',kind:'audio',direction:'sendrecv',codec:{name:'opus',params:{sample_rate:48000,channels:1}}}]},frame.id,frame);return true;
 }});}finally{for(const name of names){const d=originals.get(name);if(d)Object.defineProperty(globalThis,name,d);else delete globalThis[name];}}
}
test('WSS typed media offer requests no devices and separates signed answer acceptance from connected local readiness',()=>scenario(async({begun,peer,f,permissions,client})=>{
 let ready=false;void begun.ready.then(()=>ready=true);await begun.accepted;assert.equal(ready,false);assert.equal(permissions(),0);assert.equal(begun.submissionAttempted,true);peer().connect();const owner=await begun.ready;
 assert.equal(f.sent.filter(v=>v.type==='connection.offer').length,1);assert.equal(f.sent.find(v=>v.type==='connection.offer').operation_id,'original-offer');
 const renderer=await owner.render({audio:false},{timeoutMs:100});owner.close();await renderer.finished;assert.equal(renderer.diagnostics.closed,true);assert.equal(client.closed,false);
},{pending:true}));
test('WSS explicit capture owns only selected tracks, pauses/resumes and ends with the Connection without closing control',()=>scenario(async({begun,peer,permissions,client})=>{
 const owner=await begun.ready,track=new Track(),capture=await owner.capture({audio:true,stream:new Stream([track])},{timeoutMs:100});assert.equal(permissions(),0);assert.equal(peer().getTransceivers()[0].sender.track,track);capture.pause();assert.equal(track.enabled,false);await capture.resume();assert.equal(track.enabled,true);
 document.visibilityState='hidden';document.dispatchEvent(new Event('visibilitychange'));assert.equal(track.enabled,false);await capture.resume();assert.equal(track.enabled,false);document.visibilityState='visible';await capture.resume();assert.equal(track.enabled,true);
 owner.close();assert.equal(capture.closed,true);assert.equal(track.readyState,'ended');assert.equal(client.closed,false);
}));
test('WSS denied capture sends no mutation and preserves permission-denial classification',()=>scenario(async({begun,f,permissions})=>{
 const owner=await begun.ready,n=f.sent.length;await assert.rejects(()=>owner.capture({audio:true},{timeoutMs:100}),{code:'media-denied'});assert.equal(permissions(),1);assert.equal(f.sent.length,n);
}));
test('WSS permission result after physical retirement stops its late tracks and cannot acquire sender custody',()=>scenario(async({begun,setPermission,peer,client})=>{
 const owner=await begun.ready,permission=deferred();setPermission(()=>permission.promise);const pending=owner.capture({audio:true},{timeoutMs:200});await Promise.resolve();owner.close();await assert.rejects(pending);
 const track=new Track();permission.resolve(new Stream([track]));await new Promise(resolve=>setTimeout(resolve,0));assert.equal(track.readyState,'ended');assert.equal(peer().getTransceivers()[0].sender.track,null);assert.equal(client.closed,false);
}));
test('WSS cancellation during uncertain replaceTrack retires only that RTC owner before a late replacement can affect another capture',()=>scenario(async({begun,peer,client})=>{
 const owner=await begun.ready,late=deferred(),abort=new AbortController(),track=new Track(),sender=peer().getTransceivers()[0].sender;
 sender.replaceTrack=async value=>{await late.promise;sender.track=value;};const pending=owner.capture({audio:true,stream:new Stream([track])},{timeoutMs:200,signal:abort.signal});await new Promise(resolve=>setTimeout(resolve,0));abort.abort();await assert.rejects(pending);late.resolve();await Promise.resolve();
 assert.equal(track.readyState,'ended');assert.equal(client.closed,false);assert.equal(peer().connectionState,'closed');await assert.rejects(()=>owner.capture({audio:true},{timeoutMs:100}),{code:'closed'});
}));
test('WSS original media deadline bounds ICE gathering without submitting an offer or prompting permission',()=>scenario(async({begun,f,permissions})=>{
 await assert.rejects(begun.accepted,{code:'timeout'});await assert.rejects(begun.ready,{code:'timeout'});assert.equal(begun.submissionAttempted,false);assert.equal(f.sent.some(v=>v.type==='connection.offer'),false);assert.equal(permissions(),0);
},{gathering:true}));

test('WSS pre-aborted local media startup cannot prompt or take ownership of supplied tracks',()=>scenario(async({begun,permissions,f})=>{
 const owner=await begun.ready,abort=new AbortController(),track=new Track(),n=f.sent.length;abort.abort();
 await assert.rejects(()=>owner.capture({audio:true,stream:new Stream([track])},{timeoutMs:100,signal:abort.signal}),{code:'cancelled'});
 await assert.rejects(()=>owner.render({audio:false},{timeoutMs:100,signal:abort.signal}),{code:'cancelled'});assert.equal(track.readyState,'live');assert.equal(permissions(),0);assert.equal(f.sent.length,n);
}));
test('WSS foreign or misclassified receiver track retires only the original RTC owner',()=>scenario(async({begun,peer,client})=>{
 await begun.ready;const track=new Track();track.kind='video';peer().ontrack({track,transceiver:peer().getTransceivers()[0]});assert.equal(peer().connectionState,'closed');assert.equal(client.closed,false);
}));
test('WSS single video RTP profile refuses simultaneous camera and screen before allocating or submitting another owner',()=>scenario(async({begun,client,peer,f,permissions})=>{
 await begun.ready;const original=peer(),sent=f.sent.length;
 assert.throws(()=>BrowserRtcConnection.begin(client,{...input,connectionId:'other',operationId:'other-offer',video:'camera',screen:'desktop'},'recipient',{}, {timeoutMs:100}),{code:'unsupported-media-profile'});
 assert.equal(peer(),original);assert.equal(f.sent.length,sent);assert.equal(permissions(),0);assert.equal(client.closed,false);
}));
test('WSS shutdown joins an uncertain track replacement after stopping its capture',()=>scenario(async({begun,peer})=>{
 const owner=await begun.ready,late=deferred(),entered=deferred(),track=new Track(),sender=peer().getTransceivers()[0].sender;
 sender.replaceTrack=async value=>{entered.resolve();await late.promise;sender.track=value;};
 const capture=owner.capture({audio:true,stream:new Stream([track])},{timeoutMs:200});void capture.catch(()=>{});await entered.promise;
 let joined=false;const closing=owner.shutdown().then(()=>joined=true);await new Promise(setImmediate);
 assert.equal(joined,false);assert.equal(track.readyState,'ended');assert.equal(peer().connectionState,'closed');
 late.resolve();await closing;await assert.rejects(capture);assert.equal(sender.track.readyState,'ended');
}));
for(const failure of [false,true])test(`WSS shutdown joins renderer audio cleanup${failure?' and reports its failure':''}`,()=>scenario(async({begun,peer})=>{
 const owner=await begun.ready,late=deferred(),original=Object.getOwnPropertyDescriptor(globalThis,'AudioContext');let reject;
 const cleanup=failure?new Promise((_,no)=>reject=no):late.promise;
 class Node{gain={value:1};connect(){}disconnect(){}}
 class Context{destination={};createMediaStreamSource(){return new Node();}createGain(){return new Node();}async resume(){}close(){return cleanup;}}
 Object.defineProperty(globalThis,'AudioContext',{configurable:true,value:Context});
 try{
  const renderer=await owner.render({audio:true},{timeoutMs:100});let joined=false;
  const closing=owner.shutdown().finally(()=>joined=true);void closing.catch(()=>{});await new Promise(setImmediate);assert.equal(joined,false);assert.equal(peer().connectionState,'closed');
  if(failure){reject(Error('synthetic RTC cleanup failure'));await assert.rejects(closing,/synthetic RTC cleanup failure/);await assert.rejects(renderer.finished,{code:'media'});}else{late.resolve();await closing;await renderer.finished;}
 }finally{late.resolve();if(original)Object.defineProperty(globalThis,'AudioContext',original);else delete globalThis.AudioContext;}
}));

async function audioGraph(run){
 const original=Object.getOwnPropertyDescriptor(globalThis,'AudioContext'),contexts=[];
 class Node{gain={value:1};connected=[];disconnects=0;connect(v){this.connected.push(v);}disconnect(){this.disconnects++;}}
 class Context{destination={};state='suspended';closes=0;sources=[];gains=[];constructor(){contexts.push(this);}createMediaStreamSource(stream){const node=new Node();node.stream=stream;this.sources.push(node);return node;}createGain(){const node=new Node();this.gains.push(node);return node;}async resume(){this.state='running';}async suspend(){this.state='suspended';}async close(){this.closes++;this.state='closed';}}
 Object.defineProperty(globalThis,'AudioContext',{configurable:true,value:Context});
 try{return await run(contexts);}finally{if(original)Object.defineProperty(globalThis,'AudioContext',original);else delete globalThis.AudioContext;}
}
test('WSS renderer owns the muted decoder sink and its graph across pause, visibility, resume and close',()=>scenario(async({begun,peer,client,elements,f})=>audioGraph(async contexts=>{
 const owner=await begun.ready,renderer=await owner.render({audio:true,volume:.25},{timeoutMs:100}),element=elements[0],context=contexts[0],track=peer().getTransceivers()[0].receiver.track,requests=f.sent.length;
 assert.equal(element.muted,true);assert.deepEqual(element.srcObject.getAudioTracks(),[track]);assert.equal(context.gains[0].gain.value,.25);assert.equal(element.plays,1);
 renderer.pause();assert.equal(element.pauses,1);assert.equal(context.state,'suspended');await renderer.resume();assert.equal(element.plays,2);
 document.visibilityState='hidden';document.dispatchEvent(new Event('visibilitychange'));assert.equal(renderer.diagnostics.paused,true);await renderer.resume();assert.equal(element.plays,2);document.visibilityState='visible';await renderer.resume();assert.equal(element.plays,3);
 renderer.close();await renderer.finished;assert.equal(element.srcObject,null);assert.equal(context.closes,1);assert.equal(context.sources[0].disconnects,1);assert.equal(track.readyState,'live');assert.equal(client.closed,false);assert.equal(f.sent.length,requests);assert.equal(peer().connectionState,'connected');
})));
for(const phase of ['startup','resume'])test(`WSS audio ${phase} policy failure settles the owned sink/graph and permits explicit renderer retry`,()=>scenario(async({begun,elements,setPlayback,peer,f})=>audioGraph(async contexts=>{
 const owner=await begun.ready,requests=f.sent.length;let renderer;
 if(phase==='resume')renderer=await owner.render({audio:true},{timeoutMs:100});
 setPlayback(()=>Promise.reject(new DOMException('synthetic playback policy','NotAllowedError')));
 if(renderer){renderer.pause();await assert.rejects(()=>renderer.resume(),{code:'media'});await assert.rejects(renderer.finished,{code:'media'});assert.equal(renderer.diagnostics.closed,true);}
 else await assert.rejects(()=>owner.render({audio:true},{timeoutMs:100}),{code:'media'});
 assert.equal(elements[0].srcObject,null);assert.equal(contexts[0].closes,1);assert.equal(peer().connectionState,'connected');
 setPlayback(()=>Promise.resolve());const next=await owner.render({audio:true},{timeoutMs:100});assert.equal(elements[1].plays,1);next.close();await next.finished;assert.equal(elements[1].srcObject,null);assert.equal(f.sent.length,requests);
})));
for(const phase of ['deadline','abort','retire'])test(`WSS pending audio play ${phase} clears its owned sink before late completion`,()=>scenario(async({begun,elements,setPlayback,f})=>audioGraph(async contexts=>{
 const owner=await begun.ready,late=deferred(),entered=deferred(),abort=new AbortController(),requests=f.sent.length;
 setPlayback(()=>{entered.resolve();return late.promise;});const work=owner.render({audio:true},{timeoutMs:phase==='deadline'?15:200,signal:abort.signal});void work.catch(()=>{});await entered.promise;
 if(phase==='abort')abort.abort();if(phase==='retire')owner.close();await assert.rejects(work,phase==='retire'?undefined:{code:phase==='deadline'?'timeout':'cancelled'});await new Promise(setImmediate);
 assert.equal(elements[0].srcObject,null);assert.equal(contexts[0].closes,1);late.resolve();await new Promise(setImmediate);assert.equal(elements[0].srcObject,null);assert.equal(f.sent.length,requests);
})));
