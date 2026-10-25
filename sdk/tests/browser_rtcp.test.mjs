import assert from 'node:assert/strict';
import {test} from 'node:test';
import {encodeRtcpSenderReport,decodeRtcpSenderReport,wrapRtcpSenderReport,isRtcpDatagram,unixMicrosecondsToNtp,ntpAtRtpTimestamp} from '../typescript/dist/browser_rtcp.js';
const report=()=>({ssrc:0x10203040,ntp:(100n<<32n)+0x80000000n,rtpTimestamp:0xfffffc40,packets:0xffffffff,octets:3456,cname:'clock-🕰️@fixture.invalid'});
const refuses=fn=>assert.throws(fn,{code:'protocol'});
test('RTCP SR/SDES preserves exact NTP, RTP wrap, counters, UTF8 CNAME and sliced backing offsets',()=>{
 const value=report(),bytes=encodeRtcpSenderReport(value),backing=new Uint8Array(bytes.length+17);backing.set(bytes,9);assert.deepEqual(decodeRtcpSenderReport(backing.subarray(9,9+bytes.length)),value);
 assert.equal(bytes[0],0x80);assert.equal(bytes[1],200);assert.equal(new DataView(bytes.buffer).getUint16(2),6);assert.equal(bytes[29],202);
});
test('RTCP common UCTP wrapper retains the allocated handle and physical sequence without routing grants',()=>{
 const packet=wrapRtcpSenderReport(65535,0xffffffff,report()),v=new DataView(packet.buffer);assert.equal(v.getUint16(2),65535);assert.equal(v.getUint32(4),0xffffffff);assert.equal(isRtcpDatagram(packet),true);assert.deepEqual(decodeRtcpSenderReport(packet.subarray(8)),report());
 for(const bad of [0,-1,65536,1.5])refuses(()=>wrapRtcpSenderReport(bad,0,report()));
 packet[0]=2;assert.equal(isRtcpDatagram(packet),false);packet[0]=1;packet[2]=packet[3]=0;assert.equal(isRtcpDatagram(packet),false);
});
test('RTCP sender compound accepts bounded receive report blocks without interpreting them as timing authority',()=>{
 const source=encodeRtcpSenderReport(report()),packet=new Uint8Array(source.length+24);packet.set(source.subarray(0,28));packet[0]=0x81;new DataView(packet.buffer).setUint16(2,12);packet.set(source.subarray(28),52);assert.deepEqual(decodeRtcpSenderReport(packet),report());
});
test('RTCP rejects truncated, oversized, wrong-version and length-mismatched compounds',()=>{
 const source=encodeRtcpSenderReport(report());for(let n=0;n<source.length;n++)refuses(()=>decodeRtcpSenderReport(source.slice(0,n)));
 for(const [at,value]of [[0,0x40],[1,201],[3,255],[28,0x41],[31,255],[37,255]]){const bad=source.slice();bad[at]=value;refuses(()=>decodeRtcpSenderReport(bad));}
 refuses(()=>decodeRtcpSenderReport(new Uint8Array(1184)));
});
test('RTCP refuses missing, foreign, duplicate or malformed CNAME identities',()=>{
 const source=encodeRtcpSenderReport(report());refuses(()=>decodeRtcpSenderReport(source.subarray(0,28)));
 const foreign=source.slice();new DataView(foreign.buffer).setUint32(32,123);refuses(()=>decodeRtcpSenderReport(foreign));
 const duplicate=new Uint8Array(source.length*2-28);duplicate.set(source);duplicate.set(source.subarray(28),source.length);refuses(()=>decodeRtcpSenderReport(duplicate));
 const badUtf8=source.slice();badUtf8[38]=0xff;refuses(()=>decodeRtcpSenderReport(badUtf8));
 const duplicateSr=new Uint8Array(source.length+28);duplicateSr.set(source);duplicateSr.set(source.subarray(0,28),source.length);refuses(()=>decodeRtcpSenderReport(duplicateSr));
});
test('RTCP validates terminal padding and never accepts a BYE as a live clock sample',()=>{
 const source=encodeRtcpSenderReport(report()),padded=new Uint8Array(source.length+4);padded.set(source);padded[28]|=32;new DataView(padded.buffer).setUint16(30,new DataView(source.buffer).getUint16(30)+1);padded[padded.length-1]=4;assert.deepEqual(decodeRtcpSenderReport(padded),report());
 for(const count of [0,255]){const bad=padded.slice();bad[bad.length-1]=count;refuses(()=>decodeRtcpSenderReport(bad));}
 const first=source.slice();first[0]|=32;refuses(()=>decodeRtcpSenderReport(first));
 const bye=new Uint8Array(source.length+8);bye.set(source);bye.set([0x81,203,0,1,0x10,0x20,0x30,0x40],source.length);refuses(()=>decodeRtcpSenderReport(bye));
});
test('RTCP builders refuse invalid counters, malformed UTF8 strings and excessive names',()=>{
 for(const field of ['ssrc','rtpTimestamp','packets','octets'])for(const value of [-1,2**32,NaN,1.5])refuses(()=>encodeRtcpSenderReport({...report(),[field]:value}));
 for(const cname of ['', '\ud800','a\nb','😀'.repeat(64)])refuses(()=>encodeRtcpSenderReport({...report(),cname}));
 for(const value of [-1n,1n<<64n])refuses(()=>encodeRtcpSenderReport({...report(),ntp:value}));
 assert.equal(decodeRtcpSenderReport(encodeRtcpSenderReport({...report(),cname:'x'.repeat(255)})).cname.length,255);
});
test('RTCP NTP conversion and source interpolation preserve RTP/NTP eras and explicitly missing clocks',()=>{
 assert.equal(unixMicrosecondsToNtp(0n),2208988800n<<32n);assert.equal(unixMicrosecondsToNtp(1500000n),(2208988801n<<32n)+(1n<<31n));
 const r=report();assert.equal(ntpAtRtpTimestamp(r,0,48000),r.ntp+(1n<<32n)/50n);
 const before=ntpAtRtpTimestamp({...r,rtpTimestamp:0},0xfffffc40,48000);assert.equal(before,r.ntp-((1n<<32n)+49n)/50n);
 assert.equal(ntpAtRtpTimestamp({...r,ntp:1n<<32n,rtpTimestamp:8000},0,8000),0n);
 assert.equal(ntpAtRtpTimestamp({...r,ntp:0n},0,48000),undefined);
 assert.equal(ntpAtRtpTimestamp({...r,ntp:(1n<<64n)-1n,rtpTimestamp:0},48000,48000),(1n<<32n)-1n);
 refuses(()=>ntpAtRtpTimestamp({...r,rtpTimestamp:0},0x80000000,48000));refuses(()=>unixMicrosecondsToNtp(-1n));
});
