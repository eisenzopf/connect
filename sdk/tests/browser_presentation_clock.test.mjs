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
   advance(time,milliseconds=Math.round((time-contexts[0].currentTime)*1000)){contexts[0].currentTime=time;wall+=milliseconds;t.mock.timers.tick(milliseconds);}});
 }finally{renderer?.close();await renderer?.finished.catch(()=>{});t.mock.timers.reset();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
}
test('presentation preserves a same-tick audio capture gap and schedules video at that same source time',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;assert.equal(r.audio(c.streams[0],e.audio(0)),true);assert.equal(r.audio(c.streams[0],e.audio(100000)),true);
 assert.ok(Math.abs(e.contexts[0].sources[1].startedAt-e.contexts[0].sources[0].startedAt-.1)<1e-9,'audio capture gap collapsed into contiguous playback');
 const original=e.frame(100000);assert.equal(r.video(c.streams[1],original),true);original.close();assert.equal(e.drawn.length,0);assert.equal(e.clones[0].closed,0);
 e.advance(.109);assert.equal(e.drawn.length,0);e.advance(.111);assert.equal(e.drawn.length,1);assert.ok(Math.abs(e.drawn[0].at-e.contexts[0].sources[1].startedAt)<.002);
 assert.equal(e.clones[0].closed,1);assert.equal(original.closed,1);assert.equal(r.diagnostics.pendingVideo,0);
}));
test('late audio moves the common presentation origin and retimes already queued video',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;r.audio(c.streams[0],e.audio(0));r.video(c.streams[1],e.frame(20000));e.advance(.025);assert.equal(e.drawn.length,0);
 r.audio(c.streams[0],e.audio(20000));assert.ok(Math.abs(e.contexts[0].sources[1].startedAt-.035)<1e-9);
 e.advance(.031);assert.equal(e.drawn.length,0);e.advance(.036);assert.equal(e.drawn.length,1);assert.ok(Math.abs(e.drawn[0].at-.035)<.002);
}));
test('presentation bounds owned video and retires queued audio/video after the negotiated binding disappears',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;r.audio(c.streams[0],e.audio(0));for(let n=0;n<40;n++)r.video(c.streams[1],e.frame(n*1000));
 assert.equal(r.diagnostics.pendingVideo,8);assert.equal(e.clones.filter(c=>c.closed===1).length,32);assert.equal(e.drawn.length,0);assert.ok(e.originals.every(f=>f.closed===0));
 c.streams=[];e.advance(.1);assert.equal(r.diagnostics.pendingAudio,0);assert.equal(r.diagnostics.pendingVideo,0);assert.equal(e.drawn.length,0);assert.ok(e.contexts[0].sources.every(s=>s.stopped));assert.ok(e.clones.every(c=>c.closed===1));
}));
test('pause and close release every owned video clone and never consume the decoder original',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;r.audio(c.streams[0],e.audio(0));r.video(c.streams[1],e.frame(0));assert.equal(e.clones.length,1,'renderer retained no owned video clone');assert.equal(e.clones[0].closed,0);
 r.pause();assert.equal(e.clones[0].closed,1);assert.equal(r.diagnostics.pendingVideo,0);e.advance(1);assert.equal(e.drawn.length,0);
 await r.resume();r.video(c.streams[1],e.frame(1000000));assert.equal(e.clones[1].closed,0);r.close();await r.finished;
 e.advance(2);assert.equal(e.drawn.length,0);assert.ok(e.clones.every(c=>c.closed===1));assert.ok(e.originals.every(f=>f.closed===0));
}));
test('a stalled playback clock expires original queued work then presents fresh media after recovery',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;r.audio(c.streams[0],e.audio(0));r.video(c.streams[1],e.frame(0));e.advance(0,201);
 assert.equal(r.diagnostics.pendingAudio,0);assert.equal(r.diagnostics.pendingVideo,0);assert.equal(e.clones[0].closed,1);assert.equal(e.drawn.length,0);
 e.advance(.22,19);assert.equal(r.audio(c.streams[0],e.audio(220000)),true);assert.equal(r.video(c.streams[1],e.frame(220000)),true);e.advance(.231,11);
 assert.equal(e.drawn.length,1);assert.equal(e.drawn[0].timestamp,220000);assert.ok(e.contexts[0].sources[0].stopped);assert.ok(e.clones.every(c=>c.closed===1));
}));
test('a delayed canvas failure closes the graph and all remaining owned frames',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;const draw=e.canvas.getContext();draw.drawImage=()=>{throw Error('fixture canvas refused');};
 r.audio(c.streams[0],e.audio(0));r.video(c.streams[1],e.frame(0));r.video(c.streams[1],e.frame(20000));e.advance(.011);
 await assert.rejects(r.finished,/fixture canvas refused/);assert.equal(r.diagnostics.pendingVideo,0);assert.equal(r.diagnostics.pendingAudio,0);assert.ok(e.clones.every(c=>c.closed===1));assert.ok(e.originals.every(f=>f.closed===0));
}));
test('an invalid owned clone is closed before refusal and cannot consume the next valid source frame',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;let closed=0;assert.throws(()=>r.video(c.streams[1],{timestamp:0,clone:()=>({timestamp:1,close(){closed++;}})}),{code:'media-playback'});assert.equal(closed,1);assert.equal(r.diagnostics.pendingVideo,0);
 assert.equal(r.video(c.streams[1],e.frame(0)),true);e.advance(.011);assert.equal(e.drawn.length,1);assert.equal(e.clones[0].closed,1);
}));

test('a late presentation callback displays only the newest due frame instead of replaying a burst',t=>environment(t,async e=>{
 const {renderer:r,connection:c}=e;for(let n=0;n<8;n++)r.video(c.streams[1],e.frame(n*1000));assert.equal(e.drawn.length,0);
 e.advance(.1);assert.equal(e.drawn.length,1);assert.equal(e.drawn[0].timestamp,7000);assert.equal(r.diagnostics.video,1);assert.equal(r.diagnostics.droppedVideo,7);assert.ok(e.clones.every(c=>c.closed===1));
}));
