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
