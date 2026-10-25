// Real browser capture/worklet/codecs/playback; mock Connection, no network claim.
import {BrowserMediaCapture} from '/sdk/browser_media_capture.js';
import {BrowserMediaRenderer} from '/sdk/browser_media_renderer.js';
import {BrowserOpusCodec,BrowserVp8Codec,browserCodecSupport} from '/sdk/browser_codecs.js';
const check=(ok,message)=>{if(!ok)throw Error(message);};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const binding=(handle,kind)=>({handle,streamId:kind,connectionId:'capture-connection',sessionId:'capture-session',generation:1,kind,codec:kind==='audio'?'opus':'vp8',payloadType:kind==='audio'?111:96,direction:'sendrecv'});
function tone(frame){
  const pcm=new Float32Array(frame.numberOfFrames);frame.copyTo(pcm,{planeIndex:0,format:'f32-planar'});
  const power=f=>{let re=0,im=0;for(let n=0;n<pcm.length;n++){const a=2*Math.PI*f*n/48000;re+=pcm[n]*Math.cos(a);im+=pcm[n]*Math.sin(a);}return re*re+im*im;};
  return power(500)>power(250)*4&&pcm.some(v=>Math.abs(v)>.04);
}
export async function run({mode}){
  const support=await browserCodecSupport();check(support.audio&&support.video,'frozen codec support');
  const contexts=[],grantedTracks=[],OriginalAudioContext=globalThis.AudioContext,devices=navigator.mediaDevices,originalGetUserMedia=devices.getUserMedia;
  devices.getUserMedia=async function(options){const stream=await originalGetUserMedia.call(this,options);grantedTracks.push(...stream.getTracks());return stream;};
  globalThis.AudioContext=class extends OriginalAudioContext{constructor(options){super(options);contexts.push(this);}};
  const owners=new Set(),bindings=[binding(1,'audio'),binding(2,'video')];
  let capture,renderer,source,oscillator,paint,opus,vp8,stream;
  const errors=[],states=[],audio={decoded:0,identified:0},video={decoded:0,identified:0,motion:new Set()},encoded={audio:0,video:0};
  const connection={id:'capture-connection',streams:bindings,onClose(fn){owners.add(fn);return()=>owners.delete(fn);},
    encodeAudio(id,pcm,time){check(id==='audio','exact audio source');encoded.audio++;return opus.encode(pcm,time);},
    encodeVideo(id,frame){check(id==='video','exact video source');encoded.video++;return vp8.encode(frame);}};
  const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
  const output=document.createElement('canvas');
  const mark=n=>{const out=canvas.getContext('2d');out.fillStyle='rgb(220,20,20)';out.fillRect(0,0,640,360);out.fillStyle='rgb(10,230,10)';out.fillRect(n*7%608,80,32,200);};
  try{
    if(mode==='denied'){
      let denied;try{await BrowserMediaCapture.open(connection,{audioStreamId:'audio',videoStreamId:'video',timeoutMs:3000});}catch(e){denied=e.code;}
      check(denied==='media-denied'&&owners.size===0&&contexts.length===0,'actual permission denial before graph');
      return{passed:true,mode,permission_denied:true,graph_count:contexts.length,scope:'owned Chrome permission denial with fake device endpoint; no network/native qualification'};
    }
    renderer=await BrowserMediaRenderer.open(connection,{video:{video:output},volume:0,timeoutMs:3000});
    opus=new BrowserOpusCodec({encoded:f=>opus.decode(f.bytes,f.timestampUs),decoded:f=>{audio.decoded++;if(tone(f))audio.identified++;renderer.audio(bindings[0],f);},error:e=>errors.push(String(e))});
    vp8=new BrowserVp8Codec({encoded:f=>vp8.decode({...f,timestamp:0,ssrc:1},f.timestampUs),decoded:f=>{
      renderer.video(bindings[1],f);const out=output.getContext('2d',{willReadFrequently:true}),pixel=out.getImageData(0,0,1,1).data;
      video.decoded++;if(pixel[0]>150&&pixel[2]<80)video.identified++;
      const row=out.getImageData(0,180,640,1).data;let sum=0,count=0;for(let x=0;x<640;x++)if(row[x*4+1]>120&&row[x*4+1]>row[x*4]*1.5&&row[x*4+1]>row[x*4+2]*1.5){sum+=x;count++;}if(count>8)video.motion.add(Math.round(sum/count));
    },error:e=>errors.push(String(e))});
    if(mode==='provided'){
      source=new AudioContext({sampleRate:48000});oscillator=source.createOscillator();oscillator.frequency.value=500;
      const gain=source.createGain();gain.gain.value=.25;const dest=source.createMediaStreamDestination();oscillator.connect(gain);gain.connect(dest);oscillator.start();await source.resume();
      mark(0);stream=new MediaStream([...dest.stream.getAudioTracks(),...canvas.captureStream(30).getVideoTracks()]);let n=0;paint=setInterval(()=>mark(++n),1000/30);
    }
    capture=await BrowserMediaCapture.open(connection,{audioStreamId:'audio',videoStreamId:'video',...(stream?{stream}:{}),timeoutMs:3000,state:(state,error)=>{states.push(state);if(error)errors.push(String(error));}});
    const start=performance.now();let maxPending=0,maxQueuedSeconds=0,power=0;
    while(performance.now()-start<4000){const d=renderer.diagnostics;power=Math.max(power,d.audioRms);maxPending=Math.max(maxPending,d.pendingAudio);maxQueuedSeconds=Math.max(maxQueuedSeconds,d.queuedAudioSeconds);await wait(20);}
    const measured={duration_ms:performance.now()-start,opus:{...audio},vp8:{...video,motion:video.motion.size}};
    check(errors.length===0,'actual capture/codec/playback errors '+errors.join('; '));
    if(mode==='provided'){
      check(audio.identified>=150,'identified captured audio '+JSON.stringify({audio,video:{...video,motion:video.motion.size},encoded,states}));
      check(video.identified>=90&&video.motion.size>=40,'identified captured video '+JSON.stringify({audio,video:{...video,motion:video.motion.size},encoded,states}));
    }else check(audio.decoded>=150&&video.decoded>=90,'real fake-device capture flow '+JSON.stringify({audio,video:{...video,motion:video.motion.size},encoded,states}));
    check(power>.02&&renderer.diagnostics.endedAudio>=140,'actual audio render graph '+JSON.stringify(renderer.diagnostics));
    check(maxPending<=10&&maxQueuedSeconds<=.2,'bounded audio playback');
    globalThis.captureVisibilityPhase='hide';
    const visibility=async expected=>{const until=performance.now()+5000;while((document.visibilityState==='visible')!==expected){check(performance.now()<until,'actual visibility change deadline');await wait(20);}};
    await visibility(false);await wait(100);check(renderer.diagnostics.paused&&states.at(-1)==='paused','actual hidden-page pause');
    const before={...encoded};await wait(250);check(encoded.audio===before.audio&&encoded.video===before.video,'hidden-page capture quiescence');
    check([...grantedTracks,...(stream?.getTracks()??[])].every(t=>!t.enabled),'hidden capture tracks disabled');
    globalThis.captureVisibilityPhase='hidden';await visibility(true);
    await renderer.resume();await capture.resume();await wait(400);check(encoded.audio>before.audio+10&&encoded.video>before.video+5,'local resume restores fresh capture');
    const retirement=performance.now();for(const fn of [...owners])fn();await renderer.finished;
    while(contexts.some(c=>c!==source&&c.state!=='closed')){check(performance.now()-retirement<5000,'owned audio graph close deadline');await wait(10);}
    check(capture.closed&&renderer.diagnostics.closed&&renderer.diagnostics.pendingAudio===0&&owners.size===0,'Connection-owned resources retired');
    check([...grantedTracks,...(stream?.getTracks()??[])].every(t=>t.readyState==='ended'),'all actual owned capture tracks stopped');
    const terminal={...encoded};await wait(150);check(encoded.audio===terminal.audio&&encoded.video===terminal.video,'no capture after owner closure');
    return{passed:true,mode,measured,opus:audio,vp8:{...video,motion:video.motion.size},encoded,renderer:renderer.diagnostics,max_pending_audio:maxPending,max_queued_audio_seconds:maxQueuedSeconds,peak_render_rms:power,states,actual_hidden_pause:true,explicit_visible_resume:true,owned_cleanup_ms:performance.now()-retirement,owned_context_states:contexts.filter(c=>c!==source).map(c=>c.state),owned_track_states:[...grantedTracks,...(stream?.getTracks()??[])].map(t=>({kind:t.kind,state:t.readyState})),scope:'short actual capture/worklet/WebCodecs/canvas/audio-graph ownership checkpoint with mock Connection; no WT/native/full60s/sync/impairment/screen permission qualification'};
  }finally{
    clearInterval(paint);capture?.close();renderer?.close();opus?.close();vp8?.close();oscillator?.stop();for(const t of stream?.getTracks()??[])t.stop();
    await Promise.all(contexts.filter(c=>c.state!=='closed').map(c=>c.close().catch(()=>{})));globalThis.AudioContext=OriginalAudioContext;devices.getUserMedia=originalGetUserMedia;
  }
}
