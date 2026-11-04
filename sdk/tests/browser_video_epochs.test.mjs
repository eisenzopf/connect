import test from 'node:test';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
const base = process.env.CONNECT_GATE6_TS_DIST ? pathToFileURL(path.resolve(process.env.CONNECT_GATE6_TS_DIST) + '/').href : new URL('../typescript/dist/', import.meta.url).href;
const {BrowserVideoEpochs, videoNotice, videoAllocation} = await import(base + 'browser_video_epochs.js');
const {BrowserMediaPeer} = await import(base + 'browser_media.js');
const {encodeNativeVideoFeedback, decodeNativeVideoFeedback, decodeNativeRtpDatagram, decodeNativeVideoEpoch} = await import(base + 'browser_video_wire.js');
const {RtpSender} = await import(base + 'browser_rtp.js');
const {envelope} = await import(base + 'uctp_wire.js');
const profile='rtcp-pli-epoch/1', root={cid:'room',sid:'session',connid:'connection'};
const binding={handle:7,streamId:'video',connectionId:'connection',sessionId:'session',generation:1,kind:'video',codec:'vp8',payloadType:96,direction:'sendrecv'};
const wireAllocation={profile,original_operation:'offer',original_offer:'wire1',stream_generation:1,connection_generation:1};
const caps={video_feedback:profile};
const allocation=()=>videoAllocation({kind:'video',video_feedback:wireAllocation},caps,caps,'offer','wire1',1);
const epoch=n=>n.toString(16).padStart(32,'0');
const notice=(n=1,role='receiver',first=10,ssrc=83)=>envelope('stream.epoch',{...wireAllocation,strm_id:'video',stream_local_id:7,incarnation:epoch(99),revision:n,direction:role,epoch:epoch(n),ssrc,first_sequence:first,first_timestamp:first*3000},root);
function packet(tag=1,sequence=10,timestamp=sequence*3000,ssrc=83,key=true){
 const bytes=new Uint8Array(8+16+12+4),v=new DataView(bytes.buffer);
 bytes.set([1,1,0,7]);v.setUint32(4,sequence);bytes.set(Buffer.from(epoch(tag),'hex'),8);
 bytes.set([0x80,0xe0],24);v.setUint16(26,sequence);v.setUint32(28,timestamp);v.setUint32(32,ssrc);bytes.set([0x10,key?0:1,2,3],36);return bytes;
}
const parsed=(...args)=>decodeNativeVideoEpoch(packet(...args));

test('browser video history joins only the exact signed allocation and packet boundary with original bounded age',()=>{
 const owner=new BrowserVideoEpochs(root,binding,allocation());
 assert.equal(owner.packet(parsed(),0,0),undefined);assert.equal(owner.pendingPackets,1);
 const wrong=notice();wrong.payload.original_offer='other';assert.equal(owner.observe(videoNotice(wrong)),false);
 assert.equal(owner.drain(20).length,0);
 assert.equal(owner.observe(videoNotice(notice())),true);
 const admitted=owner.drain(199);assert.equal(admitted.length,1);assert.equal(admitted[0].arrived,0);assert.equal(admitted[0].packet.timestamp,30000);
 assert.equal(owner.observe(videoNotice(notice())),false,'replay cannot reinitialize history');
 assert.equal(owner.packet(parsed(2,11),200,200),undefined);
 assert.equal(owner.observe(videoNotice(notice(2,'receiver',11))),true);
 assert.equal(owner.drain(400).length,0,'notice cannot renew original packet age');
 assert.equal(owner.packet(parsed(1,10),401,401),undefined,'older same-SSRC history is not current');
 assert.equal(owner.packet(parsed(2,11,123),401,401),undefined,'first packet must bind the announced original timestamp');
 assert.equal(owner.packet(parsed(2,11,33000,84),401,401),undefined,'matching tag with foreign SSRC is refused');
 assert.equal(owner.packet(parsed(2,12),401,401).ssrc,83);
 owner.unselect();assert.equal(owner.packet(parsed(2,13),402,402),undefined);
 assert.equal(owner.observe(videoNotice(notice(2,'receiver',11))),false);
 assert.equal(owner.observe(videoNotice(notice(3,'receiver',14))),true);
 assert.equal(owner.packet(parsed(3,14),403,403).timestamp,42000);
 for(let i=0;i<100;i++)owner.packet(parsed(4,15+i),404,404);
 assert.equal(owner.pendingPackets,32);owner.expire(604);assert.equal(owner.pendingPackets,0);
 owner.packet(parsed(4,20),605,605,false);assert.equal(owner.pendingPackets,0);
 for(const field of ['revision','ssrc','first_sequence','first_timestamp']){const v=notice();v.payload[field]=-1;assert.throws(()=>videoNotice(v));}
 for(const patch of [{in_reply_to:'request'},{operation_id:'operation'},{cid:undefined}])assert.throws(()=>videoNotice({...notice(),...patch}));
 assert.throws(()=>videoAllocation({kind:'video',video_feedback:wireAllocation},{},caps,'offer','wire1',1));
 assert.throws(()=>videoAllocation({kind:'video',video_feedback:wireAllocation},caps,caps,'other','wire1',1));
 assert.throws(()=>videoAllocation({kind:'video'},caps,caps,'offer','wire1',1));
 assert.throws(()=>videoAllocation({kind:'video',video_feedback:{...wireAllocation,stream_generation:2}},caps,caps,'offer','wire1',1));
 owner.close();assert.equal(owner.observe(videoNotice(notice(5))),false);
});

test('browser feedback expires from physical arrival, consumes once, and cannot survive replay, hold or captured target change',()=>{
 const owner=new BrowserVideoEpochs(root,binding,allocation());owner.observe(videoNotice(notice()));owner.observe(videoNotice(notice(1,'publisher')));
 const hint={handle:7,datagramSequence:0xffffffff,senderSsrc:44,mediaSsrc:83,epoch:epoch(1)};
 owner.feedback(hint,0);assert.equal(owner.takeFeedback(199,83),true);assert.equal(owner.takeFeedback(199,83),false);
 owner.feedback(hint,199);assert.equal(owner.takeFeedback(199,83),false,'same sequence does not receive a new arrival time');
 owner.feedback({...hint,datagramSequence:0},200);assert.equal(owner.takeFeedback(400,83),false,'wrap is accepted but age is not renewed');
 owner.feedback({...hint,datagramSequence:1},401);owner.invalidateRequests();assert.equal(owner.takeFeedback(402,83),false);
 const write=owner.request(500);assert.ok(write);assert.equal(decodeNativeVideoFeedback(write.bytes).epoch,epoch(1));assert.ok(owner.current(write,501));
 assert.equal(owner.request(599),undefined);owner.observe(videoNotice(notice(2,'receiver',11)));assert.equal(owner.current(write,501),false);
 const next=owner.request(600);assert.equal(decodeNativeVideoFeedback(next.bytes).datagramSequence,1,'capture consumes serial even if never written');
 owner.feedback({...hint,datagramSequence:2},601);owner.gates(false,false);assert.equal(owner.takeFeedback(602,83),false,'held hint refused before expiry');
 assert.equal(owner.current(next,602),false);owner.gates(true,true);assert.equal(owner.request(800),undefined,'resume cannot replay old notice');
 owner.observe(videoNotice(notice(3,'receiver',12)));owner.observe(videoNotice(notice(3,'publisher',12)));
 owner.feedback({...hint,datagramSequence:3,epoch:epoch(3)},801);assert.equal(owner.takeFeedback(802,83),true);
 const captured=owner.request(900);owner.invalidateRequests();assert.equal(owner.current(captured,901),false);
 owner.close();assert.equal(owner.request(1100),undefined);
});

const tick=()=>new Promise(resolve=>setImmediate(resolve));
async function until(check){for(let i=0;i<100;i++){if(check())return;await new Promise(r=>setTimeout(r,2));}assert.fail('bounded browser worker observation did not arrive');}
test('browser physical owner stages signed epochs, resets same-SSRC decode history, writes current feedback and forces only a fresh frame',async()=>{
 const names=['VideoEncoder','VideoDecoder','VideoFrame','EncodedVideoChunk'];const old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
 const encoded=[],decoded=[];let resets=0,closedFrames=0;
 class Encoder{
  state='unconfigured';encodeQueueSize=0;static async isConfigSupported(){return{supported:true};}
  constructor(callbacks){this.callbacks=callbacks;}configure(){this.state='configured';}
  encode(frame,options){encoded.push({timestamp:frame.timestamp,key:options.keyFrame});const bytes=new Uint8Array([options.keyFrame?0:1,2,3]);this.callbacks.output({byteLength:3,timestamp:frame.timestamp,type:options.keyFrame?'key':'delta',copyTo:b=>b.set(bytes)});}
  close(){this.state='closed';}
 }
 class Decoder{state='unconfigured';decodeQueueSize=0;static async isConfigSupported(){return{supported:true};}configure(){this.state='configured';}reset(){resets++;}decode(frame){decoded.push(frame);}close(){this.state='closed';}}
 class Frame{constructor(timestamp){this.timestamp=timestamp;}close(){closedFrames++;}}
 for(const[n,value]of Object.entries({VideoEncoder:Encoder,VideoDecoder:Decoder,VideoFrame:Frame,EncodedVideoChunk:class{constructor(v){Object.assign(this,v);}}}))Object.defineProperty(globalThis,n,{configurable:true,value});
 let releaseStatus;const status=new Promise(resolve=>releaseStatus=resolve);
 class Control{
  closed=false;events=new Set();lifetimes=new Set();writes=[];count=0;result=undefined;
  constructor(){this.datagrams={maxDatagramSize:1200,readable:new ReadableStream({start:c=>this.incoming=c}),writable:new WritableStream({write:b=>this.writes.push(b.slice())})};}
  onEvent(f){this.events.add(f);return()=>this.events.delete(f);}onClose(f){this.lifetimes.add(f);return()=>this.lifetimes.delete(f);}
  emit(v){for(const f of this.events)f(v);}close(){if(this.closed)return;this.closed=true;for(const f of [...this.lifetimes])f();}
  begin(type,payload,scope){const id='wire'+ ++this.count;
   if(type==='connection.offer'){
    assert.equal(payload.capabilities.video_feedback,profile);
    queueMicrotask(()=>this.emit(envelope('connection.offer',{by_participant:'participant',substrate:'webtransport',capabilities:caps,
     substrate_setup:{kind:'native-prepared/1',offer_id:id,revision:1,streams:[{strm_id:'video',kind:'video',direction:'sendrecv',stream_local_id:7,video_feedback:wireAllocation,codec:{name:'vp8',params:{sample_rate:90000,channels:0,payload_type:96}}}]}},scope)));
    return{id,response:Promise.resolve([envelope('connection.answer',{}, {...scope,in_reply_to:id})])};
   }
   assert.equal(type,'connection.update');this.result={connection_id:'connection',connection_generation:1,stream_revision:2,resources_changed:false,action:'hold',gate_streams:[{binding:{stream_id:'video',medium:'video',direction:'sendrecv',stream_local_id:7,stream_generation:1,connection_generation:1,codec:{name:'vp8',params:{sample_rate:90000,channels:0,payload_type:96}}},state:'paused',source_muted:false,held_direction:null}]};
   return{id,response:Promise.resolve([envelope('ack',{}, {...scope,in_reply_to:id})])};
  }
  async send(){this.emit(notice());return 'answer';}
  async operationStatus(){await status;return{state:'completed',result:this.result};}
 }
 let peer;const c=new Control();
 try{
  peer=new BrowserMediaPeer(c,{error:e=>assert.fail(e)});
  const opening=peer.offerConnection({conversationId:'room',sessionId:'session',connectionId:'connection',operationId:'offer',video:'video',nativeVideoFeedback:true});
  await tick();assert.equal(peer.diagnostics.activeBindings,0,'signed notice alone cannot install prepared media');releaseStatus();const connection=await opening;
  c.incoming.enqueue(packet(1,10,30000,83,false));await until(()=>peer.routeDiagnostics[0].counters.complete===1);
  assert.equal(decoded.length,0,'initial delta cannot bootstrap');await until(()=>c.writes.some(b=>b.length===48));
  assert.equal(decodeNativeVideoFeedback(c.writes.find(b=>b.length===48)).epoch,epoch(1));
  c.incoming.enqueue(packet(1,11,33000,83,true));await until(()=>decoded.length===1);const firstTimestamp=decoded[0].timestamp;
  const before=resets;c.emit(notice(2,'receiver',12));assert.ok(resets>before);
  c.incoming.enqueue(packet(2,12,36000,83,false));await until(()=>peer.routeDiagnostics[0].counters.complete===3);assert.equal(decoded.length,1);
  c.incoming.enqueue(packet(1,11,33000,83,true));await tick();assert.equal(decoded.length,1,'late previous history cannot decode after reselection');
  c.incoming.enqueue(packet(2,13,39000,83,true));await until(()=>decoded.length===2);assert.ok(decoded[1].timestamp>firstTimestamp,'receive clock is not reset with history');
  assert.equal(connection.encodeVideo('video',new Frame(0)),true);await until(()=>c.writes.some(b=>b.length!==48));
  const source=c.writes.find(b=>b[9]===224),own=decodeNativeRtpDatagram(source);
  const published=notice(1,'publisher',own.datagramSequence,own.ssrc);published.payload.first_timestamp=own.timestamp;c.emit(published);
  c.incoming.enqueue(encodeNativeVideoFeedback({handle:7,datagramSequence:5,senderSsrc:44,mediaSsrc:own.ssrc,epoch:epoch(1)}));await tick();
  assert.equal(connection.encodeVideo('video',new Frame(33333)),true);assert.equal(encoded.at(-1).key,true);assert.equal(encoded.at(-1).timestamp,33333);
  c.incoming.enqueue(encodeNativeVideoFeedback({handle:7,datagramSequence:5,senderSsrc:44,mediaSsrc:own.ssrc,epoch:epoch(1)}));await tick();
  connection.encodeVideo('video',new Frame(66666));assert.equal(encoded.at(-1).key,false,'replayed feedback cannot force another source frame');
  c.incoming.enqueue(encodeNativeVideoFeedback({handle:7,datagramSequence:6,senderSsrc:44,mediaSsrc:own.ssrc,epoch:epoch(1)}));await tick();
  await peer.beginControl('connection','hold','connection.update',{action:'hold',streams:['video'],details:{direction:'inactive'}},1,{timeoutMs:1000}).ready;
  connection.encodeVideo('video',new Frame(99999));assert.equal(encoded.at(-1).key,false,'hold clears already-read pending feedback');
  assert.equal(closedFrames,4);await peer.shutdown();assert.equal(c.events.size,0);assert.equal(peer.diagnostics.activeBindings,0);assert.equal(c.datagrams.readable.locked,false);assert.equal(c.datagrams.writable.locked,false);
 }finally{await peer?.shutdown();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});
