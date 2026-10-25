import './media_diagnostics.test.mjs';
import './browser_uctp.test.mjs';
import './browser_wss.test.mjs';
import './browser_rtp.test.mjs';
import './browser_codecs.test.mjs';
import './browser_media.test.mjs';
import './browser_capture_media.test.mjs';
import './browser_renderer.test.mjs';
import './tasks.test.mjs';
import './record_context.test.mjs';
import './contacts.test.mjs';
import './reference_client.test.mjs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {fileURLToPath} from 'node:url';
import {readFileSync,readdirSync} from 'node:fs';
import {createServer} from 'node:http';
import {NativeAudioClient,NativeClient,HttpClient,ConnectError} from '../typescript/dist/index.js';
import {decodeResponse} from '../typescript/dist/protocol.js';
import './browser_voice.test.mjs';
const helper=fileURLToPath(new URL('./helper_fixture.py',import.meta.url));
const rejects=(code)=>error=>error instanceof ConnectError&&error.code===code;
test('browser entry point loads without Node helper modules',async()=>{const browser=await import('../typescript/dist/browser.js');assert.equal(typeof browser.BrowserHttpClient,'function');assert.equal(typeof browser.BrowserVoiceSession,'function');});
test('browser voice reports microphone denial and rejects non-loopback signaling',async()=>{
 const {BrowserVoiceSession}=await import('../typescript/dist/browser.js');
 await assert.rejects(()=>BrowserVoiceSession.open('wss://public.example.test/voice','fixture'),/loopback/);
 const descriptor=Object.getOwnPropertyDescriptor(globalThis,'navigator');
 Object.defineProperty(globalThis,'navigator',{configurable:true,value:{mediaDevices:{getUserMedia:async()=>{throw new Error('denied');}}}});
 try{await assert.rejects(()=>BrowserVoiceSession.open('ws://127.0.0.1:1234','fixture'),rejects('media-denied'));}
 finally{if(descriptor)Object.defineProperty(globalThis,'navigator',descriptor);else delete globalThis.navigator;}
});
test('browser voice applies its deadline to microphone acquisition',async()=>{
 const {BrowserVoiceSession}=await import('../typescript/dist/browser.js');
 const descriptor=Object.getOwnPropertyDescriptor(globalThis,'navigator');
 Object.defineProperty(globalThis,'navigator',{configurable:true,value:{mediaDevices:{getUserMedia:()=>new Promise(()=>{})}}});
 try{await assert.rejects(()=>BrowserVoiceSession.open('ws://127.0.0.1:1234','fixture',10),rejects('timeout'));}
 finally{if(descriptor)Object.defineProperty(globalThis,'navigator',descriptor);else delete globalThis.navigator;}
});
test('browser voice reports remote playback failure',async()=>{
 const {BrowserVoiceSession}=await import('../typescript/dist/browser.js');
 const session=Object.create(BrowserVoiceSession.prototype);
 session.remoteStream={id:'fixture-stream'};
 const element={srcObject:null,autoplay:false,pause:()=>{},play:async()=>{throw new Error('blocked');}};
 await assert.rejects(()=>session.attach(element),rejects('media-output-failed'));
 assert.equal(element.srcObject,null);
 assert.equal(element.autoplay,true);
});
test('browser playback deadline clears an output that never starts',async()=>{
 const {BrowserVoiceSession}=await import('../typescript/dist/browser.js');
 const session=Object.create(BrowserVoiceSession.prototype);session.remoteStream={};
 let paused=false;const element={srcObject:null,autoplay:false,pause:()=>{paused=true;},play:()=>new Promise(()=>{})};
 await assert.rejects(()=>session.attach(element,10),rejects('media-output-failed'));
 assert.equal(element.srcObject,null);assert.equal(paused,true);
});
test('browser diagnostics report selected path and codec without addresses',async()=>{
 const {voiceDiagnostics}=await import('../typescript/dist/browser_voice_stats.js');
 const stats=new Map([
  ['transport',{type:'transport',selectedCandidatePairId:'pair'}],
  ['pair',{id:'pair',type:'candidate-pair',localCandidateId:'local',remoteCandidateId:'remote'}],
  ['local',{candidateType:'host',protocol:'udp',address:'private-address'}],
  ['remote',{candidateType:'host',address:'private-address'}],
  ['codec',{mimeType:'audio/opus'}],
  ['inbound',{type:'inbound-rtp',kind:'audio',codecId:'codec',packetsReceived:12,bytesReceived:120}],
  ['outbound',{type:'outbound-rtp',kind:'audio',codecId:'codec',packetsSent:15,bytesSent:150}],
 ]);
 const result=await voiceDiagnostics({connectionState:'connected',iceConnectionState:'connected',getStats:async()=>stats});
 assert.equal(result.selectedPair.id,'pair');assert.equal(result.audio[0].codec,'audio/opus');
 assert.equal(result.audio[1].packets,15);assert.ok(!JSON.stringify(result).includes('private-address'));
});
test('binary media helper carries bounded raw PCMU',async()=>{
 const media=await NativeAudioClient.open('media-ok','conversation','session','remote',helper,1000);
 try{assert.equal(media.publisherStreamId,'fixture-stream');await media.subscribe('other-stream');const frame=Uint8Array.from({length:160},(_,i)=>i);await media.sendPcmu(frame);assert.deepEqual(await media.receive(),frame);await assert.rejects(()=>media.sendPcmu(new Uint8Array(1)),rejects('invalid-request'));}finally{await media.close();}
 for(const mode of ['media-stall','media-oversize'])await assert.rejects(()=>NativeAudioClient.open(mode,'conversation','session','remote',helper,200),error=>error instanceof ConnectError);
});
test('native audio accepts coalesced pipe frames without weakening per-frame limits',async()=>{
 const media=await NativeAudioClient.open('media-burst','conversation','session','remote',helper,1000);
 try{const frame=new Uint8Array(160).fill(42);await media.sendPcmu(frame);
  for(let i=0;i<32;i++)assert.deepEqual(await media.receive(),frame);
 }finally{await media.close();}
});
for(const mode of ['fragment','stall'])test(`native ${mode} obeys deadline and closes`,async()=>{
 const c=new NativeClient(mode,helper,400);const start=Date.now();
 try{await assert.rejects(()=>c.health(),rejects('transport-uncertain'));assert.ok(Date.now()-start<1500);await assert.rejects(()=>c.health(),rejects('transport-uncertain'));}finally{await c.close();await c.close();}
});
for(const mode of ['oversize','malformed','wrong-id','wrong-kind','bad-fields','invalid-utf8'])test(`native ${mode} rejects with protocol error`,async()=>{
 const c=new NativeClient(mode,helper);try{await assert.rejects(()=>c.health(),rejects('protocol'));await assert.rejects(()=>c.health(),rejects('transport-uncertain'));}finally{await c.close();}
});
test('chunked frames and concurrent calls serialize',async()=>{const c=new NativeClient('chunked',helper);try{const results=await Promise.all(Array.from({length:8},()=>c.health()));assert.ok(results.every(r=>r.installation_id==='fixture'));}finally{await c.close();}});
test('application denial does not close native client',async()=>{const c=new NativeClient('denied',helper);try{await assert.rejects(()=>c.health(),rejects('forbidden'));assert.equal((await c.health()).schema,7);}finally{await c.close();}});
test('missing executable settles close',async()=>{const c=new NativeClient('fixture','/no-such-connect-helper');await assert.rejects(()=>c.health(),rejects('transport-uncertain'));await c.close();});
test('invalid deadlines are rejected',()=>{for(const value of [0,-1,Infinity,NaN,60001])assert.throws(()=>new NativeClient('fixture',helper,value),RangeError);});
test('HTTP validation, bounds, deadline, no redirect/retry',async()=>{
 let mode='denied',count=0;
 const server=createServer((req,res)=>{count++;req.resume();res.writeHead(mode==='redirect'?302:mode==='denied'?403:200,{location:'/must-not-follow'});if(mode==='stall'){res.write('{');return;}res.end(mode==='oversize'?'x'.repeat(600001):mode==='malformed'?'{}':JSON.stringify({profile:'connect-control/1',result:{Err:{code:'forbidden',message:'fixture'}}}));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 try{for(const [next,code]of [['denied','forbidden'],['malformed','protocol'],['oversize','protocol'],['redirect','protocol'],['stall','transport-uncertain']]){mode=next;const before=count;await assert.rejects(()=>new HttpClient(`http://127.0.0.1:${server.address().port}`,'fixture',200).health(),rejects(code));assert.equal(count,before+1);}}
 finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
test('HTTP artifact helpers preserve raw bytes and framing',async()=>{
 let uploaded;
 const artifact={id:'artifact',conversation_id:'conversation',owner_id:'owner',name:'sample.bin',media_type:'application/octet-stream',size:4,received:4,sha256:'0'.repeat(64),state:'uploading',created_at:1,expires_at:null};
 const server=createServer((req,res)=>{const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{if(req.method==='PUT'){uploaded=Buffer.concat(chunks);const body=JSON.stringify({profile:'connect-control/1',result:{Ok:{kind:'artifact',data:artifact}}});res.writeHead(200,{'content-type':'application/json','content-length':Buffer.byteLength(body)});res.end(body);}else{const body=Buffer.from('raw!');res.writeHead(200,{'content-type':'application/octet-stream','content-length':body.length,'x-connect-offset':'0','x-connect-eof':'true'});res.end(body);}});});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 try{const client=new HttpClient(`http://127.0.0.1:${server.address().port}`,'fixture');const result=await client.uploadArtifactChunk('conversation','artifact','append-1',0,Buffer.from('raw!'));assert.equal(result.data.id,'artifact');assert.deepEqual(uploaded,Buffer.from('raw!'));const chunk=await client.downloadArtifactChunk('conversation','artifact',0,4);assert.deepEqual(Buffer.from(chunk.data),Buffer.from('raw!'));assert.equal(chunk.offset,0);assert.equal(chunk.eof,true);}
 finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
test('Rust result fixtures validate in TypeScript',()=>{const dir=new URL('../../contracts/fixtures/results/',import.meta.url);for(const name of readdirSync(dir)){const response=JSON.parse(readFileSync(new URL(name,dir),'utf8')).payload;const result=response.result;if(result.Err)assert.throws(()=>decodeResponse(response,'health'),rejects(result.Err.code));else assert.deepEqual(decodeResponse(response,result.Ok.kind),result.Ok);}});

test('native audio reports remote end promptly and preserves the terminal code',async()=>{
 const media=await NativeAudioClient.open('media-ended','conversation','session','remote',helper,1000);
 try{await assert.rejects(()=>media.receive(),rejects('media-ended'));
  await assert.rejects(()=>media.receive(),rejects('media-ended'));
  await assert.rejects(()=>media.sendPcmu(new Uint8Array(160)),rejects('media-ended'));
 }finally{await media.close();}
});

test('health readiness validates observations and still accepts older servers',()=>{
 const health={installation_id:'fixture',schema:10,profile:'connect-control/1'};
 const wrap=data=>({profile:'connect-control/1',result:{Ok:{kind:'health',data}}});
 assert.equal(decodeResponse(wrap(health),'health').data.schema,10);
 const readiness={state:'unavailable',components:[{component:'provider-delivery',state:'unavailable',reason:'worker-exited',observed_at:1791600000}]};
 assert.equal(decodeResponse(wrap({...health,readiness}),'health').data.readiness.state,'unavailable');
 for(const configured of [true,false]) {
  const observed={...readiness,components:[{...readiness.components[0],configured}]};
  assert.equal(decodeResponse(wrap({...health,readiness:observed}),'health').data.readiness.components[0].configured,configured);
 }
 for(const configured of [null,0,'false',{},[]]) {
  const observed={...readiness,components:[{...readiness.components[0],configured}]};
  assert.throws(()=>decodeResponse(wrap({...health,readiness:observed}),'health'),rejects('protocol'));
 }
 for(const invalid of [null,42,{state:'healthy',components:[]},{state:'ready',components:[{component:'x',state:'ready',reason:'bound',observed_at:'yesterday'}]}])
  assert.throws(()=>decodeResponse(wrap({...health,readiness:invalid}),'health'),rejects('protocol'));
});

import "./browser_opus_clock.test.mjs";

import "./browser_rtcp.test.mjs";

import "./browser_source_clock.test.mjs";

import "./browser_presentation_clock.test.mjs";
