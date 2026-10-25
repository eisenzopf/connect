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
