import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaRenderer} from '../typescript/dist/browser_media_renderer.js';
const binding=(handle,kind)=>({handle,streamId:kind,connectionId:'connection',sessionId:'session',generation:1,kind,codec:kind==='audio'?'opus':'vp8',payloadType:kind==='audio'?111:96,direction:'sendrecv'});
async function environment(t,run){
 const names=['AudioContext','document','performance'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]),contexts=[];let wall=0;
 class Node{connect(){}disconnect(){this.disconnected=true;}gain={value:1};fftSize=0;getFloatTimeDomainData(a){a.fill(.1);}}
 class Context{
  sampleRate=48000;state='suspended';currentTime=0;sources=[];destination={};
  constructor(){contexts.push(this);}async resume(){this.state='running';}async suspend(){this.state='suspended';}async close(){this.state='closed';}
  createGain(){return new Node();}createAnalyser(){return new Node();}createBuffer(){return{copyToChannel(){}};}
  createBufferSource(){const s=new Node();s.start=time=>{s.startedAt=time;};s.stop=()=>{s.stopped=true;};this.sources.push(s);return s;}
 }
 const values=[Context,{visibilityState:'visible',addEventListener(){},removeEventListener(){}},{now:()=>wall}];
 for(let i=0;i<names.length;i++)Object.defineProperty(globalThis,names[i],{configurable:true,value:values[i]});
 t.mock.timers.enable({apis:['setTimeout']});
 const connection={id:'connection',streams:[binding(1,'audio'),binding(2,'video')],onClose(){return()=>{};}};
 let renderer;const drawn=[],clones=[],originals=[];
 const draw={drawImage(frame){drawn.push({timestamp:frame.timestamp,at:contexts[0].currentTime});}};const canvas={getContext:()=>draw};
 const frame=timestamp=>{const source={timestamp,closed:0,close(){this.closed++;},clone(){const c={timestamp,closed:0,close(){this.closed++;}};clones.push(c);return c;}};originals.push(source);return source;};
 const audio=timestamp=>({timestamp,sampleRate:48000,numberOfChannels:1,numberOfFrames:960,copyTo(){},close(){throw Error('borrowed audio consumed');}});
 try{
  renderer=await BrowserMediaRenderer.open(connection,{video:{video:canvas}});
  await run({renderer,connection,contexts,drawn,clones,originals,frame,audio,canvas,
   advance(time,milliseconds=Math.round((time-contexts[0].currentTime)*1000)){contexts[0].currentTime=time;for(const source of contexts[0].sources)if(source.startedAt+.02<=time)source.onended?.();wall+=milliseconds;t.mock.timers.tick(milliseconds);}});
 }finally{renderer?.close();await renderer?.finished.catch(()=>{});t.mock.timers.reset();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
}
test('repeated late originals never move the shared deadline and fresh audio/video remain synchronized',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;assert.equal(r.audio(c.streams[0],e.audio(0)),true);
 const lead=e.contexts[0].sources[0].startedAt;
 for(let second=0;second<30;second++){
  const base=second;
  e.advance(base+.15);
  assert.equal(r.audio(c.streams[0],e.audio(second*1000000+20000)),false,'late original moved the shared epoch');
  const pts=second*1000000+160000;e.advance(base+.16);
  assert.equal(r.audio(c.streams[0],e.audio(pts)),true,'fresh output inherited an old late original');
  assert.ok(Math.abs(e.contexts[0].sources.at(-1).startedAt-(base+.16+lead))<1e-9);
  assert.equal(r.video(c.streams[1],e.frame(pts)),true);
  e.advance(base+.16+lead+.001);
  assert.equal(e.drawn.at(-1).timestamp,pts);assert.ok(Math.abs(e.drawn.at(-1).at-e.contexts[0].sources.at(-1).startedAt)<.002);
  assert.ok(r.diagnostics.pendingAudio<=2&&r.diagnostics.pendingVideo===0);
 }
 assert.equal(r.diagnostics.droppedAudio,30);
}));
test('a250ms playback stall expires originals and resumes fresh output without changing the shared epoch',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;assert.equal(r.audio(c.streams[0],e.audio(0)),true);const lead=e.contexts[0].sources[0].startedAt;
 assert.equal(r.video(c.streams[1],e.frame(0)),true);e.advance(0,251);
 assert.equal(r.diagnostics.pendingAudio,0);assert.equal(r.diagnostics.pendingVideo,0);assert.equal(e.clones[0].closed,1);assert.equal(e.drawn.length,0);
 e.advance(.25,0);assert.equal(r.audio(c.streams[0],e.audio(120000)),false,'expired source deadline was moved to now');
 assert.equal(r.audio(c.streams[0],e.audio(250000)),true);assert.equal(r.video(c.streams[1],e.frame(250000)),true);
 assert.ok(Math.abs(e.contexts[0].sources.at(-1).startedAt-(.25+lead))<1e-9);e.advance(.25+lead+.001);
 assert.equal(e.drawn.length,1);assert.equal(e.drawn[0].timestamp,250000);assert.ok(e.contexts[0].sources[0].stopped);assert.ok(e.clones.every(f=>f.closed===1));
}));

test('100ms arrival variation retains fixed common audio/video deadlines inside the original queue budget',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;assert.equal(r.audio(c.streams[0],e.audio(0)),true);
 const lead=e.contexts[0].sources[0].startedAt;
 assert.ok(lead>0&&lead+.02<=.2);
 for(let step=1;step<=50;step++){
  const source=step*.02,pts=step*20000;e.advance(source+.1);
  assert.equal(r.audio(c.streams[0],e.audio(pts)),true,'bounded delay variation missed the fixed audio deadline');
  if(step%2===0)assert.equal(r.video(c.streams[1],e.frame(pts)),true);
  assert.ok(Math.abs(e.contexts[0].sources.at(-1).startedAt-(source+lead))<1e-9,'arrival jitter moved the shared origin');
  assert.ok(r.diagnostics.pendingAudio<=3&&r.diagnostics.pendingVideo<=2);
  assert.ok(r.diagnostics.queuedAudioSeconds<=.2);
 }
 e.advance(1+lead+.001);
 assert.equal(r.diagnostics.audio,51);assert.equal(r.diagnostics.droppedAudio,0);
 assert.equal(e.drawn.length,25);assert.equal(e.drawn.at(-1).timestamp,1000000);
 assert.ok(Math.abs(e.drawn.at(-1).at-e.contexts[0].sources.at(-1).startedAt)<.002);
 assert.ok(e.clones.every(f=>f.closed===1));assert.ok(e.originals.every(f=>f.closed===0));
}));
