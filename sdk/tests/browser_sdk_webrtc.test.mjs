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
 const document=Object.assign(new EventTarget(),{visibilityState:'visible'});
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
 try{return await fixture(async f=>{const client=await BrowserClient.open(f.options);let begun;try{begun=BrowserRtcConnection.begin(client,{...input},'recipient',{}, {timeoutMs:300});return await run({client,f,begun,peer:()=>peer,permissions:()=>permissions,setPermission:fn=>permission=fn});}finally{begun?.owner.close();client.close();}},{handler:async({peer,frame})=>{
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
