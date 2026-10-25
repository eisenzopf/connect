import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaRenderer} from '../typescript/dist/browser_media_renderer.js';
const binding=(handle,kind)=>({handle,streamId:kind,connectionId:'connection',sessionId:'session',generation:1,kind,codec:kind==='audio'?'opus':'vp8',payloadType:kind==='audio'?111:96,direction:'sendrecv'});
function owner(){const closed=new Set();return{id:'connection',streams:[binding(1,'audio'),binding(2,'video')],closed,onClose(fn){closed.add(fn);return()=>closed.delete(fn);}};}
async function environment(resume,run){
  const contexts=[],names=['AudioContext','document'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
  class Node{connect(){}disconnect(){}gain={value:1};fftSize=0;getFloatTimeDomainData(array){array.fill(.1);}}
  class Context{
    sampleRate=48000;state='suspended';currentTime=0;sources=[];destination={};
    constructor(){contexts.push(this);}
    async resume(){await resume?.();this.state='running';}
    async suspend(){this.state='suspended';}
    async close(){this.state='closed';}
    createGain(){return new Node();}createAnalyser(){return new Node();}createBuffer(){return{copyToChannel(){}};}
    createBufferSource(){const n=new Node();n.stop=()=>{n.stopped=true;};n.start=()=>{};this.sources.push(n);return n;}
  }
  const values=[Context,{visibilityState:'visible',addEventListener(){},removeEventListener(){}}];for(let i=0;i<names.length;i++)Object.defineProperty(globalThis,names[i],{configurable:true,value:values[i]});
  try{await run(contexts);}finally{for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
}
test('renderer startup deadline closes its pending graph and detaches Connection ownership',()=>environment(()=>new Promise(()=>{}),async contexts=>{
  const connection=owner();await assert.rejects(()=>BrowserMediaRenderer.open(connection,{timeoutMs:10}),{code:'timeout'});
  assert.equal(connection.closed.size,0);assert.equal(contexts[0].state,'closed');
}));
test('renderer bounds a burst and Connection closure stops every scheduled buffer without consuming borrowed frames',()=>environment(undefined,async contexts=>{
  const connection=owner(),renderer=await BrowserMediaRenderer.open(connection);let borrowedCloses=0;
  try{
    for(let n=0;n<40;n++)renderer.audio(connection.streams[0],{sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp:n*20000,copyTo(){},close(){borrowedCloses++;}});
    const d=renderer.diagnostics;assert.ok(d.pendingAudio>0&&d.pendingAudio<=10);assert.ok(d.queuedAudioSeconds<=.2);assert.ok(d.droppedAudio>=30);assert.equal(borrowedCloses,0);
    for(const fn of [...connection.closed])fn();await renderer.finished;
    assert.equal(renderer.diagnostics.pendingAudio,0);assert.equal(contexts[0].state,'closed');assert.ok(contexts[0].sources.every(s=>s.stopped));assert.equal(connection.closed.size,0);
  }finally{renderer.close();}
}));
test('renderer refuses stale and foreign bindings and pause clears old playback before fresh resume',()=>environment(undefined,async contexts=>{
  const connection=owner();let drawn=0;const canvas={getContext:()=>({drawImage(){drawn++;}})};
  const renderer=await BrowserMediaRenderer.open(connection,{video:{video:canvas}});
  try{
    assert.equal(renderer.video({...connection.streams[1],generation:2},{}),false);assert.equal(renderer.video({...connection.streams[1],connectionId:'foreign'},{}),false);
    assert.equal(renderer.video({...connection.streams[1],sessionId:'foreign'},{}),false);
    const video=time=>({timestamp:time,clone:()=>({timestamp:time,close(){}})});
    assert.equal(renderer.video(connection.streams[1],video(0)),true);assert.equal(drawn,0);contexts[0].currentTime=.011;await new Promise(r=>setTimeout(r,15));assert.equal(drawn,1);
    const audio=time=>({sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp:time,copyTo(){}});
    assert.equal(renderer.audio(connection.streams[0],audio(0)),true);renderer.pause();assert.equal(renderer.diagnostics.pendingAudio,0);
    assert.equal(renderer.audio(connection.streams[0],audio(20000)),false);assert.equal(renderer.video(connection.streams[1],{}),false);
    await renderer.resume();assert.equal(renderer.audio(connection.streams[0],audio(40000)),true);assert.equal(renderer.video(connection.streams[1],video(40000)),true);assert.equal(contexts[0].state,'running');
  }finally{renderer.close();await renderer.finished;}
}));
test('renderer close failure is observable after all local buffers and ownership observers retire',()=>environment(undefined,async contexts=>{
  const connection=owner(),renderer=await BrowserMediaRenderer.open(connection);
  contexts[0].close=async()=>{throw Error('fixture graph close refused');};renderer.close();
  await assert.rejects(renderer.finished,/graph close refused/);assert.equal(renderer.diagnostics.pendingAudio,0);assert.equal(connection.closed.size,0);assert.match(renderer.diagnostics.error,/graph close refused/);
}));
