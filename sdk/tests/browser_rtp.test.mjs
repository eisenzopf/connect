import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {decodeRtpDatagram,MediaBindings,RtpSender,Vp8Assembler,MediaSendQueue,MEDIA_LIMITS} from '../typescript/dist/browser_rtp.js';
const vectors=JSON.parse(readFileSync(new URL('../../.deps/rvoip/crates/uctp/rvoip-uctp/tests/fixtures/cross_runtime_media.json',import.meta.url)));
for(const v of vectors.valid)test('SDK independently parses shared complete RTP vector: '+v.name,()=>{
  const p=decodeRtpDatagram(Uint8Array.from(Buffer.from(v.hex,'hex')));
  assert.deepEqual([p.flags,p.handle,p.datagramSequence,p.payloadType,p.marker,p.sequence,p.timestamp,p.ssrc,p.csrc,p.extensionProfile??null,p.padding,Buffer.from(p.payload).toString('hex')],
    [v.flags,v.stream_local_id,v.datagram_seq,v.payload_type,v.marker,v.sequence_number,v.timestamp,v.ssrc,v.csrc,v.extension_profile,v.padding_size,v.payload_hex]);
});
for(const v of vectors.invalid)test('SDK refuses shared malformed RTP vector: '+v.name,()=>assert.throws(()=>decodeRtpDatagram(Uint8Array.from(Buffer.from(v.hex,'hex'))),{code:'protocol'}));
const binding={handle:1,streamId:'stream_test',connectionId:'connection_test',sessionId:'session_test',generation:1,kind:'video',codec:'vp8',payloadType:96,direction:'sendrecv'};
test('physical bindings reject retired/reused handles, wrong direction, codec and scope',()=>{
  const b=new MediaBindings();b.register(binding);b.require(1,'send',96);
  assert.throws(()=>b.require(1,'send',111),{code:'protocol'});b.retireConnection('connection_test');
  assert.throws(()=>b.require(1,'receive'),{code:'protocol'});assert.throws(()=>b.register({...binding,generation:2}),{code:'protocol'});
  b.register({...binding,handle:2,direction:'recvonly'});assert.throws(()=>b.require(2,'send'),{code:'protocol'});
  for(const bad of [{...binding,handle:0},{...binding,kind:'audio'},{...binding,sessionId:''},{...binding,payloadType:128}])assert.throws(()=>new MediaBindings().register(bad));
});
test('VP8 packetizer obeys MTU and receiver assembles reordered fragments, without duplicate decode',()=>{
  const original=Uint8Array.from({length:10000},(_,i)=>i%251);original[0]=0;
  const sender=new RtpSender(binding,1000),packets=sender.packetize(original,90000);
  assert.ok(packets.length>1);assert.ok(packets.every(p=>p.length<=1000));
  const assembler=new Vp8Assembler();let result;
  for(const p of [...packets].reverse())result=assembler.push(decodeRtpDatagram(p),0)??result;
  assert.deepEqual(result.bytes,original);assert.equal(result.firstSequence,decodeRtpDatagram(packets[0]).sequence);assert.equal(result.lastSequence,decodeRtpDatagram(packets.at(-1)).sequence);assert.equal(result.key,true);assert.equal(assembler.queuedFrames,0);
  for(const p of packets)assert.equal(assembler.push(decodeRtpDatagram(p),1),undefined);
  assert.throws(()=>sender.packetize(new Uint8Array(256*1024+1),0),{code:'protocol'});
});
test('VP8 loss, timestamp interleaving, expiry and sequence wrap stay within fixed bounds',()=>{
  const sender=new RtpSender(binding),a=new Vp8Assembler();
  const packets=sender.packetize(new Uint8Array(4000),100);
  a.push(decodeRtpDatagram(packets[0]),0);a.push(decodeRtpDatagram(packets.at(-1)),1);
  a.expire(200);assert.equal(a.queuedFrames,0);assert.equal(a.queuedFragments,0);
  for(let i=0;i<20;i++)a.push(decodeRtpDatagram(sender.packetize(new Uint8Array(2000),1000+i)[0]),201+i);
  assert.ok(a.queuedFrames<=8);assert.ok(a.queuedFragments<=64);
  a.clear();
  const wrap=sender.packetize(new Uint8Array(3000),12345);let result;
  for(let i=0;i<wrap.length;i++){new DataView(wrap[i].buffer).setUint16(10,(65535+i)&65535);result=a.push(decodeRtpDatagram(wrap[i]),1000)??result;}
  assert.equal(result.bytes.length,3000);assert.equal(a.queuedFragments,0);
  const next=sender.packetize(new Uint8Array(2000),22222);const bad=decodeRtpDatagram(next[0]);bad.payload=new Uint8Array([128,128,128]);assert.throws(()=>a.push(bad,1001),{code:'protocol'});
});
test('audio packets remain complete and media queues drop stale work at fixed packet and byte limits',()=>{
  const sender=new RtpSender({...binding,kind:'audio',codec:'opus',payloadType:111});
  const packet=sender.packetize(new Uint8Array(80),960)[0];assert.equal(decodeRtpDatagram(packet).payload.length,80);
  assert.throws(()=>sender.packetize(new Uint8Array(2000),0),{code:'protocol'});
  const q=new MediaSendQueue();for(let i=0;i<100;i++)assert.equal(q.push([packet],true,i),true);
  assert.equal(q.queuedPackets,10);assert.ok(q.queuedBytes<=MEDIA_LIMITS.sendBytes);
  assert.equal(q.pop(400),undefined);assert.equal(q.queuedBytes,0);
  assert.equal(q.push(Array(65).fill(packet),false,401),false);
  const video=new RtpSender(binding).packetize(new Uint8Array(60000),90000);
  for(let i=0;i<10;i++)q.push(video,false,402+i);
  assert.ok(q.queuedPackets<=64);assert.ok(q.queuedBytes<=76032);
  q.clear();assert.equal(q.queuedBytes,0);
});
