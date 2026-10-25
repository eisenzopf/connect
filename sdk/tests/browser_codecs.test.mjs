import assert from 'node:assert/strict';
import {test} from 'node:test';
import {browserCodecSupport,BrowserOpusCodec,BrowserVp8Codec,RtpClock,OPUS_CONFIG} from '../typescript/dist/browser_codecs.js';
test('feature detection reports absent codecs without constructing capture or pretending media is usable',async()=>{
  assert.deepEqual(await browserCodecSupport(),{audio:false,video:false,missing:['Opus mono 48 kHz WebCodecs','VP8 640x360 WebCodecs']});
  assert.throws(()=>new BrowserOpusCodec({}),{code:'unsupported-media-profile'});
  assert.throws(()=>new BrowserVp8Codec({}),{code:'unsupported-media-profile'});
  assert.throws(()=>{OPUS_CONFIG.opus.frameDuration=10000;},TypeError);
});
test('RTP clock unwraps unsigned timestamps and preserves delayed frame times',()=>{
  const c=new RtpClock(48000);
  assert.equal(c.timestamp(0xfffffc40),0);assert.equal(c.timestamp(0),20000);assert.equal(c.timestamp(960),40000);
  assert.equal(c.timestamp(0),20000);assert.equal(c.timestamp(1920),60000);
});
test('video accepts capture jitter while retaining thirty frames per rolling second and closes every input',()=>{
  const names=['VideoEncoder','VideoDecoder'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);let closed=0,submitted=0;
  class Codec{state='unconfigured';encodeQueueSize=0;configure(){this.state='configured';}encode(){submitted++;}close(){this.state='closed';}}
  for(const n of names)Object.defineProperty(globalThis,n,{configurable:true,value:Codec});
  let codec;try{
    codec=new BrowserVp8Codec({encoded(){},decoded(){},error(e){throw e;}});
    const frame=timestamp=>({timestamp,close(){closed++;}});
    for(let n=0;n<30;n++)assert.equal(codec.encode(frame(n*33333+(n%2?400:0))),true,'capture timestamp '+n);
    assert.equal(submitted,30);assert.equal(codec.encode(frame(999999)),false);assert.equal(codec.encode(frame(1000000)),true);
    assert.equal(codec.encode(frame(1000000)),false);assert.equal(codec.encode(frame(999999)),false);
    codec.close();assert.equal(codec.encode(frame(2000000)),false);assert.equal(closed,35);
  }finally{codec?.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});
test('VP8 refuses deltas after a completely missing RTP frame and resumes only at a current keyframe',async()=>{
  const {RtpSender,Vp8Assembler,decodeRtpDatagram}=await import('../typescript/dist/browser_rtp.js');
  const names=['VideoEncoder','VideoDecoder','EncodedVideoChunk'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]),submitted=[];
  class Codec{state='unconfigured';encodeQueueSize=0;decodeQueueSize=0;configure(){this.state='configured';}reset(){}decode(frame){submitted.push(frame);}close(){this.state='closed';}}
  Object.defineProperty(globalThis,'VideoEncoder',{configurable:true,value:Codec});Object.defineProperty(globalThis,'VideoDecoder',{configurable:true,value:Codec});Object.defineProperty(globalThis,'EncodedVideoChunk',{configurable:true,value:class{constructor(value){Object.assign(this,value);}}});
  let codec;try{
    codec=new BrowserVp8Codec({encoded(){},decoded(){},error(e){throw e;}});
    const sender=new RtpSender({handle:1,streamId:'video',connectionId:'connection',sessionId:'session',generation:1,kind:'video',codec:'vp8',payloadType:96,direction:'sendrecv'}),assembler=new Vp8Assembler();
    const next=(key,n)=>assembler.push(decodeRtpDatagram(sender.packetize(new Uint8Array([key?0:1,2,3]),n*3000)[0]),n);
    const initial=next(true,0);assert.equal(codec.decode(initial,0),true);
    sender.packetize(new Uint8Array([1,2,3]),3000); // The entire middle RTP frame is absent, so no incomplete assembly can expire.
    assert.equal(codec.decode(next(false,2),66667),false,'a complete delta cannot repair a lost reference frame');
    assert.equal(submitted.length,1,'the corrupt reference chain must not reach WebCodecs');
    const recovered=next(true,3);assert.equal(codec.decode(recovered,100000),true);
    assert.equal(codec.decode(next(false,4),133333),true);
    assert.deepEqual(submitted.map(c=>c.type),['key','key','delta']);
    assert.equal(codec.decode(recovered,100000),false,'late keyframes do not rewind a newer reference chain');
    const wrapSender=new RtpSender({handle:1,streamId:'video',connectionId:'connection',sessionId:'session',generation:1,kind:'video',codec:'vp8',payloadType:96,direction:'sendrecv'}),wrapAssembler=new Vp8Assembler();
    const wrapped=(key,sequence,timestamp)=>{const packets=wrapSender.packetize(new Uint8Array([key?0:1,2,3]),timestamp);new DataView(packets[0].buffer).setUint16(10,sequence);new DataView(packets[0].buffer).setUint32(16,(initial.ssrc+1)>>>0);return wrapAssembler.push(decodeRtpDatagram(packets[0]),10+sequence/65536);};
    assert.equal(codec.decode(wrapped(true,65535,0xfffff448),166667),true);
    assert.equal(codec.decode(wrapped(false,0,0),200000),true,'contiguous RTP sequence and timestamp wrap retain a usable reference chain');
    assert.equal(codec.decode(wrapped(false,2,6000),266667),false,'a gap after wrap still requires a keyframe');
  }finally{codec?.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});
