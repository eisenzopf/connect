import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserOpusCodec,BrowserVp8Codec} from '../typescript/dist/browser_codecs.js';
function fixture(run){
 const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk','VideoEncoder','VideoDecoder','EncodedVideoChunk'];
 const saved=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
 class Frame{constructor(data){Object.assign(this,data);this.sampleRate=48000;this.numberOfChannels=1;this.numberOfFrames=960;this.samples=new Float32Array(data.data??960);this.closed=0;}
  copyTo(out){out.set(this.samples);}close(){this.closed++;}}
 class Encoder{state='configured';encodeQueueSize=0;decodeQueueSize=0;constructor(callbacks){this.callbacks=callbacks;}configure(){}reset(){}close(){this.state='closed';}
  encode(){if(this.refuse)throw Error('submission refused');}decode(){if(this.refuse)throw Error('submission refused');}
  emit(timestamp,key='key'){this.callbacks.output({timestamp,type:key,byteLength:2,copyTo(out){out.set([1,2]);}});}}
 class Decoder extends Encoder{emit(timestamp){this.callbacks.output(new Frame({timestamp}));}}
 const values=[Encoder,Decoder,Frame,class{constructor(value){Object.assign(this,value);}},Encoder,Decoder,class{constructor(value){Object.assign(this,value);}}];
 for(const [index,name]of names.entries())Object.defineProperty(globalThis,name,{configurable:true,value:values[index]});
 try{run();}finally{for(const[name,value]of saved)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}
test('original Opus sequence, capture gaps and native queues are counted without replacing PCM or times',()=>fixture(()=>{
 const output=[],decoded=[],codec=new BrowserOpusCodec({encoded:frame=>output.push(frame),decoded:frame=>decoded.push(frame.timestamp),error:error=>{throw error;}});
 try{
  codec.encode(new Float32Array(960).fill(.1),100000);codec.encode(new Float32Array(960).fill(.1),160000);
  codec.encoder.emit(0);codec.encoder.emit(20000);assert.deepEqual(output.map(frame=>frame.timestampUs),[100000,160000]);
  codec.encoder.encodeQueueSize=10;assert.equal(codec.encode(new Float32Array(960),180000),false);codec.encoder.encodeQueueSize=0;
  codec.decode(new Uint8Array([1]),100000);codec.decoder.emit(0);
  codec.decoder.refuse=true;assert.throws(()=>codec.decode(new Uint8Array([1]),160000),/submission refused/);codec.decoder.refuse=false;
  const report=codec.diagnostics;assert.equal(report.captureTimelineGaps,1);assert.equal(report.maximumInputStepUs,60000);
  assert.equal(report.encodeAccepted,2);assert.equal(report.encodeQueueRefusals,1);assert.equal(report.encodedFrames,2);assert.equal(report.maxEncoderQueue,10);
  assert.equal(report.decodeAccepted,1);assert.equal(report.decodedFrames,1);assert.equal(report.lastDecodedTimestampUs,100000);assert.deepEqual(decoded,[100000]);
  report.encodedFrames=99;assert.equal(codec.diagnostics.encodedFrames,2,'Snapshots cannot mutate original counters');
 }finally{codec.close();}
}));
test('VP8 diagnostics distinguish rolling-rate, timestamp and queue refusal without relaxing any admission bound',()=>fixture(()=>{
 let closed=0;const codec=new BrowserVp8Codec({encoded(){},decoded(){},error:error=>{throw error;}}),frame=timestamp=>({timestamp,close(){closed++;}});
 try{
  for(let i=0;i<30;i++)assert.equal(codec.encode(frame(i*33333)),true);
  assert.equal(codec.encode(frame(999999)),false);assert.equal(codec.encode(frame(1000000)),true);assert.equal(codec.encode(frame(1000000)),false);
  codec.encoder.encodeQueueSize=2;assert.equal(codec.encode(frame(1033333)),false);codec.encoder.encodeQueueSize=0;
  codec.encoder.emit(1000000);codec.close();assert.equal(codec.encode(frame(2000000)),false);
  const report=codec.encodeDiagnostics;assert.equal(report.acceptedFrames,31);assert.equal(report.rateRefusals,1);assert.equal(report.timestampRefusals,1);
  assert.equal(report.queueRefusals,1);assert.equal(report.stoppedRefusals,1);assert.equal(report.nativeOutputs,1);assert.equal(report.forwardedOutputs,1);assert.equal(closed,35);
 }finally{codec.close();}
}));

test('private readiness observers run only on matched actual decoder output and release on reset/close',()=>fixture(()=>{
 const errors=[],seen=[],codec=new BrowserVp8Codec({encoded(){},decoded(){},error:e=>errors.push(e)});
 const frame=(sequence,key=true)=>({arrived:performance.now(),ssrc:7,timestamp:sequence*3000,key,firstSequence:sequence,lastSequence:sequence,bytes:new Uint8Array([key?0:1,1,2])});
 codec.decode(frame(0),100000,f=>seen.push(f.timestamp));assert.deepEqual(seen,[],'accepted input is not output');
 codec.decoder.emit(100000);assert.deepEqual(seen,[100000]);
 codec.decode(frame(1,false),133333,()=>seen.push('old'));codec.resetReceiveHistory();assert.equal(codec.pendingDecode.length,0);
 codec.decode(frame(2),166667,()=>seen.push('fresh'));codec.decoder.emit(166667);assert.deepEqual(seen,[100000,'fresh']);
 codec.decode(frame(3,false),200000,()=>seen.push('closed'));codec.close();assert.equal(codec.pendingDecode.length,0);assert.equal(errors.length,0);
 const audio=new BrowserOpusCodec({encoded(){},decoded(){},error:e=>errors.push(e)});
 audio.decode(new Uint8Array([1]),100000,f=>seen.push(f.numberOfFrames));assert.equal(seen.length,2);audio.decoder.emit(0);assert.equal(seen.at(-1),960);
 audio.decode(new Uint8Array([1]),120000,()=>seen.push('closed-audio'));audio.close();assert.equal(audio.pendingDecode.length,0);
}));
