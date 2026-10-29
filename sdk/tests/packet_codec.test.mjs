import assert from 'node:assert/strict';
import test from 'node:test';
import {SdkRtpPacket} from '../typescript/dist/index.js';
import {MAX_FRAME,parseIvf,ivfHeader,frameRecord,packetizeVp8,Vp8Assembler,rtp} from '../../examples/sdk-media/packet-codec.mjs';
const key=()=>Buffer.concat([Buffer.from([0,0,0,0x9d,1,0x2a,0x80,2,0x68,1]),Buffer.alloc(4000,42)]);
const packets=(data=key(),time=9000,start=65534,source=77)=>packetizeVp8(data,time,start,source).map((p,i)=>new SdkRtpPacket(i,p));
test('example VP8 fragments preserve wrap sequence, exact bytes and source timestamps',()=>{
 const a=new Vp8Assembler(),p=packets();assert.equal(p.length,4);assert.deepEqual(p.map(x=>x.header.sequenceNumber),[65534,65535,0,1]);assert.ok(p.every(x=>x.bytes.length<=1180));
 for(const v of p.slice(0,-1))assert.equal(a.push(v,0),undefined);const complete=a.push(p.at(-1),10);assert.equal(complete.timestamp,9000);assert.deepEqual(complete.encoded,key());
 assert.equal(a.push(p.at(-1),11),undefined);
});
test('example missing fragment or expired assembly never yields a dependent frame',()=>{
 const a=new Vp8Assembler(),p=packets();a.push(p[0],0);assert.equal(a.push(p[2],1),undefined);assert.equal(a.push(p[3],2),undefined);
 const inter=Buffer.from([1,3,4]);assert.equal(a.push(packets(inter,18000,20)[0],3),undefined);
 const next=packets(key(),27000,30);a.push(next[0],5);for(const v of next.slice(1))assert.equal(a.push(v,206),undefined);
 const recovered=packets(key(),36000,40);let f;for(const v of recovered)f=a.push(v,210);assert.deepEqual(f.encoded,key());
});
test('example malformed formats, foreign SSRC and frame limits fail before completing',()=>{
 assert.throws(()=>packetizeVp8(Buffer.alloc(MAX_FRAME+1),0,0,1));assert.throws(()=>packetizeVp8(Buffer.alloc(0),0,0,1));
 const a=new Vp8Assembler();a.push(packets()[0],0);assert.throws(()=>a.push(packets(key(),9000,65535,88)[1],1));
 const bad=Buffer.from(packets()[0].bytes);bad[12]=0x80;assert.throws(()=>new Vp8Assembler().push(new SdkRtpPacket(1,bad)));
 const wrong=key();wrong[6]=0;assert.throws(()=>{const b=new Vp8Assembler();for(const p of packets(wrong))b.push(p,0);});
 const bounded=new Vp8Assembler();assert.throws(()=>{for(let i=0;i<226;i++)bounded.push(new SdkRtpPacket(i,rtp(96,i,0,99,Buffer.concat([Buffer.from([i===0?0x10:0]),Buffer.alloc(1167)]))),0);});
 let recovered;for(const p of packets(key(),9000,300,99))recovered=bounded.push(p,1);assert.deepEqual(recovered.encoded,key());
});
test('example IVF preserves exact frame PTS and rejects truncated, extended and dishonest counts',()=>{
 const encoded=key(),header=ivfHeader(),frame={encoded,timestamp:9000};const file=Buffer.concat([header,frameRecord(frame,0)]);assert.deepEqual(parseIvf(file),[frame]);
 for(const bad of [file.subarray(0,-1),Buffer.concat([file,Buffer.from([0])]),Buffer.alloc(32)])assert.throws(()=>parseIvf(bad));
 const wrong=Buffer.from(file);wrong.writeUInt32LE(2,24);assert.throws(()=>parseIvf(wrong));wrong.writeUInt32LE(1,24);wrong[8]=0xd6;assert.throws(()=>parseIvf(wrong));
});
