import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaCapture,browserCaptureSupport} from '../typescript/dist/browser_media_capture.js';
function owner(){
  const closed=new Set();return{closed,streams:[{streamId:'audio',kind:'audio',codec:'opus',direction:'sendrecv'}],onClose(fn){closed.add(fn);return()=>closed.delete(fn);},encodeAudio(){return true;},encodeVideo(frame){frame.close();return true;}};
}
async function environment(getUserMedia,run){
  const names=['navigator','AudioContext','AudioWorkletNode','document'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
  const values=[{mediaDevices:{getUserMedia}},class{},class{},{visibilityState:'visible',addEventListener(){},removeEventListener(){}}];
  for(let i=0;i<names.length;i++)Object.defineProperty(globalThis,names[i],{configurable:true,value:values[i]});
  try{await run();}finally{for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
}
test('canonical capture refuses unsupported primitives before device permission',async()=>{
  assert.equal(browserCaptureSupport().audioWorklet,false);
  await assert.rejects(()=>BrowserMediaCapture.open(owner(),{audioStreamId:'audio'}),{code:'unsupported-media-profile'});
});
test('canonical capture reports denial and cleans its Connection lifetime observer',()=>environment(async()=>{throw new DOMException('fixture denial','NotAllowedError');},async()=>{
  const c=owner();await assert.rejects(()=>BrowserMediaCapture.open(c,{audioStreamId:'audio'}),{code:'media-denied'});assert.equal(c.closed.size,0);
}));
test('late permission after capture deadline stops owned tracks without starting a graph',async()=>{
  let finish;await environment(()=>new Promise(resolve=>{finish=resolve;}),async()=>{
    const c=owner();await assert.rejects(()=>BrowserMediaCapture.open(c,{audioStreamId:'audio',timeoutMs:10}),{code:'timeout'});
    let stopped=0;finish({getTracks:()=>[{stop(){stopped++;}}]});await Promise.resolve();await Promise.resolve();
    assert.equal(stopped,1);assert.equal(c.closed.size,0);
  });
});
test('Connection closure cancels pending capture and discards a later device grant',async()=>{
  let finish;await environment(()=>new Promise(resolve=>{finish=resolve;}),async()=>{
    const c=owner();const pending=BrowserMediaCapture.open(c,{audioStreamId:'audio'});
    for(const fn of c.closed)fn();await assert.rejects(()=>pending,{code:'closed'});
    let stopped=0;finish({getTracks:()=>[{stop(){stopped++;}}]});await Promise.resolve();await Promise.resolve();assert.equal(stopped,1);
  });
});
test('capture worklet bounds outstanding PCM and old epochs cannot lend new capture credits',async()=>{
  const names=['AudioWorkletProcessor','registerProcessor','sampleRate','currentFrame'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);let Processor;const output=[];
  class Base{port={postMessage:message=>output.push(message),onmessage:undefined};}
  const values=[Base,(_name,type)=>{Processor=type;},48000,0];for(let i=0;i<names.length;i++)Object.defineProperty(globalThis,names[i],{configurable:true,writable:true,value:values[i]});
  try{
    await import('../typescript/dist/browser_audio_worklet.js');const p=new Processor();
    const message=data=>p.port.onmessage({data});const frame=value=>{p.process([[new Float32Array(960).fill(value)]]);globalThis.currentFrame+=960;};
    message({type:'reset',epoch:1,originUs:1000000});for(let n=0;n<30;n++)frame(.1);assert.equal(output.length,10);
    for(let n=0;n<10;n++)message({type:'credit',epoch:0});frame(.1);assert.equal(output.length,10);
    message({type:'credit',epoch:1});frame(.2);assert.equal(output.length,11);
    p.process([[new Float32Array(480).fill(.3)]]);message({type:'reset',epoch:2,originUs:2000000});
    p.process([[new Float32Array(480).fill(.4)]]);assert.equal(output.length,11);p.process([[new Float32Array(480).fill(.4)]]);
    assert.equal(output.length,12);assert.equal(output.at(-1).epoch,2);assert.ok(output.at(-1).samples.every(v=>Math.abs(v-.4)<1e-6));
    message({type:'stop'});assert.equal(p.process([[new Float32Array(128)]]),false);
  }finally{for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});
