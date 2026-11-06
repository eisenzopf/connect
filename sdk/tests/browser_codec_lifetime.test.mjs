// Controlled WebCodecs owner/error lifetime checks, not real decoder/OS evidence.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserOpusCodec,BrowserVp8Codec} from '../typescript/dist/browser_codecs.js';

function fixture(run, {kind='video', constructorFailure, closeFailure}={}) {
 const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk','VideoEncoder','VideoDecoder','EncodedVideoChunk'];
 const saved=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]),encoders=[],decoders=[],frames=[];
 class Frame {
  closed=0;
  constructor(value){Object.assign(this,value);this.sampleRate??=48000;this.numberOfChannels??=1;this.numberOfFrames??=960;this.samples=new Float32Array(value.data??960);frames.push(this);}
  copyTo(out){out.set(this.samples);}
  close(){this.closed++;}
 }
 class Encoder {
  state='unconfigured';encodeQueueSize=0;decodeQueueSize=0;closes=0;
  constructor(callbacks){this.callbacks=callbacks;encoders.push(this);}
  configure(){this.state='configured';}reset(){}encode(){}decode(){}
  close(){this.closes++;if(closeFailure)throw closeFailure;this.state='closed';}
 }
 class Decoder extends Encoder {
  constructor(callbacks){if(constructorFailure)throw constructorFailure;super(callbacks);encoders.pop();decoders.push(this);}
  emit(timestamp,extra={}){const frame=new Frame({timestamp,...extra});this.callbacks.output(frame);return frame;}
 }
 class Chunk {constructor(value){Object.assign(this,value);}}
 for(const [name,value] of Object.entries({AudioEncoder:Encoder,AudioDecoder:Decoder,AudioData:Frame,EncodedAudioChunk:Chunk,VideoEncoder:Encoder,VideoDecoder:Decoder,EncodedVideoChunk:Chunk}))Object.defineProperty(globalThis,name,{configurable:true,value});
 const Codec=kind==='audio'?BrowserOpusCodec:BrowserVp8Codec;
 try{return run({Codec,encoders,decoders,frames});}
 finally{for(const[name,value]of saved)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}
for(const kind of ['audio','video']) {
 test(`${kind} second codec constructor failure closes its first original encoder and preserves exact error`,()=>{
  const original=Error('original second codec allocation refused');
  fixture(({Codec,encoders,decoders})=>{
   assert.throws(()=>new Codec({encoded(){},decoded(){},error(){assert.fail('unreturned codec issued another failure');}}),error=>error===original);
   assert.equal(encoders.length,1);assert.equal(encoders[0].closes,1);assert.equal(encoders[0].state,'closed');assert.equal(decoders.length,0);
  },{kind,constructorFailure:original});
 });
 test(`${kind} failed partial encoder cleanup cannot replace the original constructor error`,()=>{
  const original=Error('original second codec allocation refused'),cleanup=Error('original first codec close refused');
  fixture(({Codec,encoders})=>{
   assert.throws(()=>new Codec({encoded(){},decoded(){},error(){}}),error=>error===original);
   assert.equal(encoders[0].closes,1,'original cleanup must be attempted exactly once');
   // A throwing close is NOT a successful cleanup/retirement receipt.
   assert.equal(encoders[0].state,'unconfigured');
  },{kind,constructorFailure:original,closeFailure:cleanup});
 });
}
const video=(sequence,key=true)=>({arrived:performance.now(),ssrc:7,timestamp:sequence*3000,key,firstSequence:sequence,lastSequence:sequence,bytes:new Uint8Array([key?0:1,1,2])});
test('VP8 stale reset output and every queued output after close dispose original frames without borrowed callbacks',()=>fixture(({Codec,encoders,decoders,frames})=>{
 const errors=[],seen=[],codec=new Codec({encoded(){},decoded:frame=>seen.push(frame),error:error=>errors.push(error)});
 codec.decode(video(0),100000);codec.resetReceiveHistory();codec.decode(video(1),133333);
 decoders[0].emit(100000);assert.equal(errors.length,1);assert.equal(errors[0].code,'media');assert.equal(frames[0].closed,1);assert.equal(seen.length,0);
 decoders[0].emit(133333);assert.equal(frames[1].closed,1);assert.equal(seen.length,0);
 codec.close();codec.close();assert.equal(encoders[0].closes,1);assert.equal(decoders[0].closes,1);
}));
test('VP8 observer and application callback failures close output even when error callback throws',()=>{
 for(const which of ['observer','decoded'])fixture(({Codec,encoders,decoders,frames})=>{
  const original=Error(which+' refused'),errorCallback=Error('error callback refused');
  const codec=new Codec({encoded(){},decoded(){if(which==='decoded')throw original;},error(error){assert.equal(error,original);throw errorCallback;}});
  codec.decode(video(0),100000,()=>{if(which==='observer')throw original;});
  assert.throws(()=>decoders[0].emit(100000),error=>error===errorCallback);assert.equal(frames[0].closed,1);
  codec.close();assert.equal(encoders[0].closes,1);assert.equal(decoders[0].closes,1);
 });
});
test('Opus restored timestamp output closes both original and restoration after callback failure and later retirement output',()=>fixture(({Codec,encoders,decoders,frames})=>{
 const original=Error('borrowed observer refused'),seen=[],errors=[],codec=new Codec({encoded(){},decoded:f=>seen.push(f),error:e=>errors.push(e)});
 codec.decode(new Uint8Array([1]),100000,()=>{throw original;});decoders[0].emit(0);
 assert.equal(errors[0],original);assert.equal(seen.length,0);assert.equal(frames.length,2);assert.ok(frames.every(frame=>frame.closed===1));
 decoders[0].emit(20000);assert.equal(frames.at(-1).closed,1);assert.equal(seen.length,0);
 codec.close();codec.close();assert.equal(encoders[0].closes,1);assert.equal(decoders[0].closes,1);
},{kind:'audio'}));
