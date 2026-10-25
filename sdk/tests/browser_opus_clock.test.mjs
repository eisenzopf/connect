import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserOpusCodec} from '../typescript/dist/browser_codecs.js';
function fixture(run){
  const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]),frames=[],errors=[],heard=[];let decoder;
  class Frame {closed=0;constructor(init){Object.assign(this,init);this.samples=new Float32Array(init.data??960);frames.push(this);}copyTo(out){out.set(this.samples);}close(){this.closed++;}}
  class Encoder {state='unconfigured';encodeQueueSize=0;constructor(callbacks){this.callbacks=callbacks;}configure(){this.state='configured';}close(){this.state='closed';}}
  class Decoder extends Encoder {decodeQueueSize=0;calls=[];constructor(callbacks){super(callbacks);decoder=this;}decode(chunk){if(this.throwNext){this.throwNext=false;throw Error('submission refused');}this.calls.push(chunk);}emit(timestamp,extra={}){const frame=new Frame({timestamp,sampleRate:48000,numberOfFrames:960,numberOfChannels:1,data:Float32Array.from({length:960},(_,i)=>i/960),...extra});this.callbacks.output(frame);return frame;}}
  for(const[n,value]of names.map((n,i)=>[n,[Encoder,Decoder,Frame,class{constructor(init){Object.assign(this,init);}}][i]]))Object.defineProperty(globalThis,n,{configurable:true,value});
  let codec;try{codec=new BrowserOpusCodec({encoded(){},decoded(frame){heard.push({frame,timestamp:frame.timestamp,samples:new Float32Array(frame.samples)});},error(e){errors.push(e);}});run({codec,decoder,heard,errors,frames});}
  finally{codec?.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
}
test('Opus decoded output preserves original RTP gaps, late and duplicate timestamps when WebCodecs compresses time',()=>fixture(({codec,decoder,heard,frames,errors})=>{
  const stamps=[0,20000,100000,40000,100000];for(const stamp of stamps)assert.equal(codec.decode(new Uint8Array([1]),stamp),true);
  for(let n=0;n<stamps.length;n++)decoder.emit(n*20000);
  assert.deepEqual(heard.map(x=>x.timestamp),stamps);assert.equal(errors.length,0);assert.ok(frames.every(f=>f.closed===1));
  for(const h of heard)assert.equal(h.samples[720],.75,'timestamp restoration preserves exact decoded PCM');
}));
test('Opus keeps at most ten original timestamps even after native decodeQueueSize reaches zero',()=>fixture(({codec,decoder,heard,errors})=>{
  for(let n=0;n<10;n++)assert.equal(codec.decode(new Uint8Array([1]),n*20000),true);
  assert.equal(codec.decode(new Uint8Array([1]),200000),false);assert.equal(decoder.calls.length,10);
  decoder.emit(0);assert.equal(codec.decode(new Uint8Array([1]),200000),true);assert.equal(heard.length,1);assert.equal(errors.length,0);
  codec.close();const late=decoder.emit(20000);assert.equal(late.closed,1);assert.equal(heard.length,1,'late decoder output cannot revive a retired route');
}));
test('unsolicited Opus output refuses before a borrowed callback and closes its frame',()=>fixture(({decoder,heard,errors})=>{
  const raw=decoder.emit(0);assert.equal(heard.length,0);assert.equal(raw.closed,1);assert.equal(errors[0]?.code,'media');assert.equal(decoder.state,'closed');
}));
test('Opus refuses a non20ms decoder output before assigning the next original timestamp',()=>fixture(({codec,decoder,heard,errors})=>{
  assert.equal(codec.decode(new Uint8Array([1]),0),true);const raw=decoder.emit(0,{numberOfFrames:480});assert.equal(heard.length,0);assert.equal(raw.closed,1);assert.equal(errors[0]?.code,'unsupported-media-profile');
}));
test('a synchronous decode submission refusal does not shift the next original timestamp',()=>fixture(({codec,decoder,heard,errors})=>{
  decoder.throwNext=true;assert.throws(()=>codec.decode(new Uint8Array([1]),0),/submission refused/);
  assert.equal(codec.decode(new Uint8Array([1]),100000),true);decoder.emit(0);assert.equal(heard[0].timestamp,100000);assert.equal(errors.length,0);
}));
