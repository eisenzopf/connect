import assert from 'node:assert/strict';
import {test} from 'node:test';
import {RtcpCaptureClock,RtcpReceiveClock,rtpTimestampFromMicroseconds} from '../typescript/dist/browser_rtcp.js';
import {RtpSender} from '../typescript/dist/browser_rtp.js';
const report=(ssrc,ntp,rtpTimestamp,cname='same')=>({ssrc,ntp,rtpTimestamp,cname,packets:1,octets:160});
test('one capture clock preserves intertrack offsets and samples report time rather than adjacent packet time',()=>{
 let now=1000000;const c=new RtcpCaptureClock('same',()=>now,0);assert.equal(c.sample(48000),undefined);
 c.observe(0);now+=200000;c.observe(999999);assert.equal(c.sample(48000).rtpTimestamp,9600);assert.equal(c.sample(90000).rtpTimestamp,18000);assert.equal(c.sample(48000).ntp,c.sample(90000).ntp);
 now+=20000;assert.equal(c.sample(48000).rtpTimestamp,10560);assert.throws(()=>c.observe(-1),{code:'protocol'});
});
test('capture RTP conversion keeps exact microseconds at safe integer bounds and wraps only the wire clock',()=>{
 const us=Number.MAX_SAFE_INTEGER;for(const rate of [8000,48000,90000])assert.equal(rtpTimestampFromMicroseconds(us,rate),Number(BigInt.asUintN(32,(BigInt(us)*BigInt(rate)+500000n)/1000000n)));
 assert.equal(rtpTimestampFromMicroseconds(20000,48000),960);assert.throws(()=>rtpTimestampFromMicroseconds(0,44100),{code:'protocol'});
});
test('receive reports share one clock across audio/video and later per-track arrival variation',()=>{
 let now=1000000;const c=new RtcpReceiveClock(()=>now);c.register(1,48000);c.register(2,90000);
 c.report(1,report(11,100n<<32n,0));const a=c.timestamp(1,11,0);assert.equal(a.synchronized,true);
 now+=40000;c.report(2,report(22,(100n<<32n)+(1n<<32n)/10n,9000));const v=c.timestamp(2,22,0);
 assert.equal(v.synchronized,true);assert.ok(Math.abs(a.timestampUs-v.timestampUs)<=1);
 now+=1000000;c.report(2,report(22,101n<<32n,90000));assert.equal(c.timestamp(2,22,90000).timestampUs,2000000);
});
test('receive clocks retain signed RTP wrap, including a valid computed zero NTP',()=>{
 let now=1000000;const c=new RtcpReceiveClock(()=>now);c.register(1,48000);c.report(1,report(11,1n<<32n,48000));
 assert.deepEqual(c.timestamp(1,11,0),{timestampUs:0,synchronized:true});
 const w=new RtcpReceiveClock(()=>now);w.register(1,48000);w.report(1,report(11,100n<<32n,0xfffffc40));w.timestamp(1,11,0xfffffc40);
 assert.ok(Math.abs(w.timestamp(1,11,0).timestampUs-1020000)<=1);
});
test('receive clock does not reuse a retired handle, prior SSRC or out-of-order sender report',()=>{
 let now=1000000;const c=new RtcpReceiveClock(()=>now);c.register(1,48000);c.report(1,report(11,100n<<32n,0));c.timestamp(1,11,0);
 assert.equal(c.report(1,report(11,100n<<32n,999)),false);assert.equal(c.report(1,report(11,99n<<32n,999)),false);
 assert.equal(c.timestamp(1,12,0).synchronized,false);c.retire(1);assert.equal(c.report(1,report(12,101n<<32n,0)),false);assert.throws(()=>c.timestamp(1,12,0),{code:'protocol'});
});
test('missing, conflicting and expired reports stay explicitly unsynchronized',()=>{
 let now=1000000;const c=new RtcpReceiveClock(()=>now);c.register(1,48000);c.register(2,90000);c.report(1,report(11,100n<<32n,0));assert.equal(c.timestamp(1,11,0).synchronized,true);
 now+=3000001;assert.equal(c.timestamp(1,11,144000).synchronized,false);
 c.report(1,report(11,0n,0));assert.equal(c.timestamp(1,11,144960).synchronized,false);
 c.timestamp(2,22,0);assert.equal(c.report(2,report(22,102n<<32n,0,'foreign')),false);assert.equal(c.timestamp(1,11,145920).synchronized,false);
});
test('clock registries are bounded independently of claimed reports',()=>{
 const c=new RtcpReceiveClock(()=>0);for(let n=1;n<=256;n++)c.register(n,48000);assert.throws(()=>c.register(257,48000),{code:'protocol'});assert.throws(()=>c.register(1,48000),{code:'protocol'});assert.equal(c.report(999,report(1,100n<<32n,0)),false);
});
test('sender statistics count actual supplied successful submissions and exclude packetization',()=>{
 const s=new RtpSender({handle:1,streamId:'audio',connectionId:'connection',sessionId:'session',generation:1,kind:'audio',codec:'opus',payloadType:111,direction:'sendrecv'});
 const packets=s.packetize(new Uint8Array([1,2,3]),960);s.packetize(new Uint8Array([9]),1920);
 const one=s.submitted(packets[0]);assert.equal(one.packets,1);assert.equal(one.octets,3);const two=s.submitted(packets[0]);assert.equal(two.packets,2);assert.equal(two.octets,6);
 const wrong=packets[0].slice();wrong[3]=2;assert.throws(()=>s.submitted(wrong),{code:'protocol'});
});
