// Production SDK on the actual signed listener. Witnesses inspect decoded PCM/pixels.
import {BrowserClient} from '/sdk/browser_client.js';
import {BrowserMediaPeer} from '/sdk/browser_media.js';
import {BrowserMediaCapture} from '/sdk/browser_media_capture.js';
import {BrowserMediaRenderer} from '/sdk/browser_media_renderer.js';
import {unbase64url} from '/sdk/uctp_signing.js';
const check=(ok,message)=>{if(!ok)throw Error(message);};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
export async function options(actor){
  const a=actor.algorithm==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
  const s=actor.server_algorithm==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
  return {url:actor.url,credential:actor.credential,identityId:actor.identity_id,device:actor.device,
    signing:{key:await crypto.subtle.importKey('jwk',actor.private_jwk,a,false,['sign']),keyId:actor.keyid,algorithm:actor.algorithm},
    server:{key:await crypto.subtle.importKey('raw',unbase64url(actor.server_public_raw),s,false,['verify']),keyId:actor.server_keyid,algorithm:actor.server_algorithm,notBefore:actor.server_not_before,expiresAt:actor.server_expires_at},
    serverCertificateHashes:[{algorithm:'sha-256',value:unbase64url(actor.certificate_sha256)}]};
}
export function tone(frame,hz){
  const pcm=new Float32Array(frame.numberOfFrames);frame.copyTo(pcm,{planeIndex:0,format:'f32-planar'});
  const power=f=>{let re=0,im=0;for(let n=0;n<pcm.length;n++){const angle=2*Math.PI*f*n/48000;re+=pcm[n]*Math.cos(angle);im+=pcm[n]*Math.sin(angle);}return re*re+im*im;};
  return frame.sampleRate===48000&&frame.numberOfChannels===1&&power(hz)>power(hz===500?250:500)*4&&pcm.some(v=>Math.abs(v)>0.04);
}
// Explicit connected failure profiles. Normal media/clean60 sources and limits
// remain below; every boundary also completes that original short journey.
async function capturePreflightBoundary(config,clients,peers,connections,contexts){
  const mode=config.captureBoundary;
  if(mode===undefined)return;
  check(!config.clean60&&config.capture&&!config.native,'short connected capture boundary only');
  const result={mode,passed:false},connection=connections[0],capture={audioStreamId:'browser-audio-0',videoStreamId:'browser-video-0',timeoutMs:3000};
  if(mode==='permission-denied'){
    const before=contexts.length;let refusal;
    try{const unexpected=await BrowserMediaCapture.open(connection,capture);unexpected.close();}catch(error){refusal=error.code;}
    check(refusal==='media-denied'&&contexts.length===before,'connected actual browser permission denial before graph');
    result.permission_denied=true;result.graphs_created_on_denial=contexts.length-before;
  }else if(mode==='unsupported-api-codec'){
    const descriptor=Object.getOwnPropertyDescriptor(globalThis,'AudioWorkletNode'),devices=navigator.mediaDevices,originalGetUserMedia=devices.getUserMedia;
    let requests=0,apiRefusal;
    try{
      Object.defineProperty(globalThis,'AudioWorkletNode',{configurable:true,value:undefined});
      devices.getUserMedia=async()=>{requests++;throw Error('unexpected physical device request');};
      try{const unexpected=await BrowserMediaCapture.open(connection,capture);unexpected.close();}catch(error){apiRefusal=error.code;}
    }finally{if(descriptor)Object.defineProperty(globalThis,'AudioWorkletNode',descriptor);else delete globalThis.AudioWorkletNode;devices.getUserMedia=originalGetUserMedia;}
    check(apiRefusal==='unsupported-media-profile'&&requests===0&&contexts.length===0,'connected missing capture API refuses before device/graph');
    const originalSupport=AudioEncoder.isConfigSupported,request=clients[0].request;let controlRequests=0,codecRefusal;
    try{
      AudioEncoder.isConfigSupported=async config=>({supported:false,config});
      clients[0].request=function(...args){controlRequests++;return request.apply(this,args);};
      try{await peers[0].offerConnection({conversationId:config.media_cid,sessionId:'http3-media-session',connectionId:'browser-unsupported-codec',operationId:'browser-unsupported-codec',audio:'unsupported-audio'});}catch(error){codecRefusal=error.code;}
    }finally{AudioEncoder.isConfigSupported=originalSupport;clients[0].request=request;}
    check(codecRefusal==='unsupported-media-profile'&&controlRequests===0&&!clients[0].closed,'connected unsupported Opus refuses before control mutation');
    result.injected_missing_api_refused=true;result.injected_unsupported_codec_refused=true;result.device_requests=requests;result.control_requests=controlRequests;
  }else check(mode==='background-resume','known connected capture boundary');
  return result;
}
async function captureVisibilityBoundary(result,captures,renderers,streams,peers){
  if(result?.mode!=='background-resume')return;
  const visibility=async visible=>{const until=performance.now()+5000;while((document.visibilityState==='visible')!==visible){check(performance.now()<until,'connected actual visibility deadline');await wait(20);}};
  globalThis.captureVisibilityPhase='hide';await visibility(false);await wait(100);
  check(renderers.every(r=>r.diagnostics.paused&&r.diagnostics.pendingAudio===0&&r.diagnostics.pendingVideo===0),'connected hidden page drains owned presentation');
  check(streams.every(s=>s.getTracks().every(t=>!t.enabled)),'connected hidden page disables owned tracks');
  const encoded=()=>peers.map(p=>p.routeDiagnostics.reduce((sum,r)=>sum+r.counters.encoded,0));
  const before=encoded();await wait(250);check(JSON.stringify(before)===JSON.stringify(encoded()),'connected hidden capture quiescence');
  result.actual_hidden_pause=true;result.hidden_encoded_before=before;result.hidden_encoded_after=encoded();
  globalThis.captureVisibilityPhase='hidden';await visibility(true);
  await Promise.all(renderers.map(r=>r.resume()));await Promise.all(captures.map(c=>c.resume()));
  const resumed=performance.now(),counts=encoded(),until=resumed+2000;
  while(encoded().some((n,i)=>n<=counts[i]+10)){check(performance.now()<until,'connected fresh capture resume deadline');await wait(20);}
  check(streams.every(s=>s.getTracks().every(t=>t.enabled))&&renderers.every(r=>!r.diagnostics.paused),'connected explicit visible resume restores tracks and graphs');
  result.explicit_visible_resume=true;result.fresh_capture_recovery_ms=performance.now()-resumed;
}
export async function run(config){
  const clean60=config.clean60===true;check(!clean60||config.capture===true,'clean60 requires actual capture/render');
  let captureBoundary;
  const audioRefusals=[{},{}];
  const presentAudio=(i,binding,frame)=>{
    const r=renderers[i];if(!r)return;if(!config.captureBoundary){r.audio(binding,frame);return;}
    // Bounded numeric pre-call classification for explicit failure profiles.
    // Frames remain borrowed; the original renderer still makes every decision.
    const clock=r.clock,now=r.now(),due=clock?clock.local+(frame.timestamp-clock.source)/1000000:undefined;
    const reason=!r.current(binding)?'inactive':r.sources.size>=10?'full':frame.timestamp<=r.lastAudio?'nonmonotonic':due!==undefined&&due+1/48000<r.nextAudio?'overlap':due!==undefined&&due+1/48000<now?'late':due!==undefined&&Math.max(now,r.nextAudio,due)+.02-now>.2?'future':'other';
    if(!r.audio(binding,frame))audioRefusals[i][reason]=(audioRefusals[i][reason]??0)+1;
  };
  const clients=[],peers=[],connections=[],errors=[],audio=[{decoded:0,identified:0},{decoded:0,identified:0}],video=[{decoded:0,identified:0,motion:new Set()},{decoded:0,identified:0,motion:new Set()}],data=[[],[]];
  const source=[document.createElement('canvas'),document.createElement('canvas')],render=[document.createElement('canvas'),document.createElement('canvas')];
  for(const c of [...source,...render]){c.width=640;c.height=360;}
  let markerStart,minimumMarker=1,measuring=false;const markerHz=id=>1200+id*80;
  const markerOscillators=[],analysers=[],sync=[{audio:new Map(),video:new Map(),candidate:0,stable:0},{audio:new Map(),video:new Map(),candidate:0,stable:0}];
  const paint=(i,n)=>{
    const c=source[i].getContext('2d');c.fillStyle=i===0?'rgb(220,20,20)':'rgb(20,20,220)';c.fillRect(0,0,640,360);c.fillStyle='rgb(10,230,10)';c.fillRect((n*(i===0?7:11))%608,80,32,200);
    if(clean60){const id=markerStart===undefined?1:Math.floor((performance.now()-markerStart)/1000)+1;for(let bit=0;bit<8;bit++){c.fillStyle=id&(1<<bit)?'rgb(245,245,245)':'rgb(12,12,12)';c.fillRect(20+bit*8,20,8,8);}markerOscillators[i]?.frequency.setValueAtTime(markerHz(id),markerOscillators[i].context.currentTime);}
  };
  const captures=[],renderers=[],streams=[],oscillators=[],contexts=[],sourceContexts=new Set(),OriginalAudioContext=globalThis.AudioContext;
  if(config.capture)globalThis.AudioContext=class extends OriginalAudioContext{constructor(options){super(options);contexts.push(this);}createAnalyser(){const node=super.createAnalyser();if(clean60)node.smoothingTimeConstant=0;analysers.push(node);return node;}};
  const observeAudioMarkers=()=>{
    for(let i=0;i<analysers.length;i++){
      const node=analysers[i],bins=new Float32Array(node.frequencyBinCount);node.getFloatFrequencyData(bins);let best=-Infinity,second=-Infinity,id=0;
      for(let candidate=1;candidate<=80;candidate++){const bin=Math.round(markerHz(candidate)*node.fftSize/node.context.sampleRate);const db=Math.max(bins[bin-1]??-Infinity,bins[bin]??-Infinity,bins[bin+1]??-Infinity);if(db>best){second=best;best=db;id=candidate;}else second=Math.max(second,db);}
      const observed=best>-38&&best-second>5?id:0,state=sync[i];state.stable=observed===state.candidate?state.stable+1:1;state.candidate=observed;
      if(observed>=minimumMarker&&state.stable>=2&&!state.audio.has(observed))state.audio.set(observed,performance.now());
    }
  };
  let created;
  try{
    for(let i=0;i<2;i++){
      const client=await BrowserClient.open(await options(config.actors[i]));clients.push(client);
      if(i===0)created=await client.createConversation('browser-create',{participants:[config.actors[1].identity_id]});
      const peer=new BrowserMediaPeer(client,{
        audio:(binding,frame)=>{audio[i].decoded++;if(tone(frame,i===0?250:500))audio[i].identified++;presentAudio(i,binding,frame);},
        video:(binding,frame)=>{
          video[i].decoded++;const out=render[i].getContext('2d',{willReadFrequently:true});if(config.capture){if(!renderers[i]?.video(binding,frame))return;}else out.drawImage(frame,0,0,640,360);const pixel=out.getImageData(0,0,1,1).data;
          if(i===0?pixel[2]>150&&pixel[0]<80:pixel[0]>150&&pixel[2]<80)video[i].identified++;
          const row=out.getImageData(0,180,640,1).data;let sum=0,count=0;
          for(let x=0;x<640;x++)if(row[x*4+1]>120&&row[x*4+1]>row[x*4]*1.5&&row[x*4+1]>row[x*4+2]*1.5){sum+=x;count++;}
          if(count>8)video[i].motion.add(Math.round(sum/count));
          if(clean60&&measuring){let id=0;for(let bit=0;bit<8;bit++){const pixel=out.getImageData(24+bit*8,24,1,1).data;if(pixel[0]+pixel[1]+pixel[2]>500)id|=1<<bit;}if(id>=minimumMarker&&id<=80&&!sync[i].video.has(id))sync[i].video.set(id,performance.now());}
        },data:message=>data[i].push(message),error:e=>errors.push(String(e)),
      });peers.push(peer);
      connections.push(await peer.offerConnection({conversationId:config.media_cid,sessionId:'http3-media-session',connectionId:'browser-connection-'+i,operationId:'browser-offer-'+i,audio:'browser-audio-'+i,video:'browser-video-'+i,
        data:['text','json','binary'].map(protocol=>({id:'browser-'+protocol+'-'+i,protocol}))}));
    }
    for(let i=0;i<2;i++)await connections[i].subscribe('browser-subscribe-'+i,config.participants[1-i],['audio','video','data']);
    if(config.captureBoundary)captureBoundary=await capturePreflightBoundary(config,clients,peers,connections,contexts);
    if(config.capture){
      for(let i=0;i<2;i++)renderers.push(await BrowserMediaRenderer.open(connections[i],{video:{['browser-video-'+(1-i)]:render[i]},volume:0,timeoutMs:3000}));
      for(let i=0;i<2;i++){
        const context=new AudioContext({sampleRate:48000});sourceContexts.add(context);const oscillator=context.createOscillator();oscillator.frequency.value=i===0?500:250;oscillators.push(oscillator);
        const gain=context.createGain();gain.gain.value=.25;const destination=context.createMediaStreamDestination();oscillator.connect(gain);gain.connect(destination);oscillator.start();await context.resume();
        if(clean60){const marker=context.createOscillator(),level=context.createGain();marker.frequency.value=markerHz(1);level.gain.value=.12;marker.connect(level);level.connect(destination);marker.start();markerOscillators[i]=marker;oscillators.push(marker);}
        paint(i,0);const stream=new MediaStream([...destination.stream.getAudioTracks(),...source[i].captureStream(30).getVideoTracks()]);streams.push(stream);
        captures.push(await BrowserMediaCapture.open(connections[i],{audioStreamId:'browser-audio-'+i,videoStreamId:'browser-video-'+i,stream,timeoutMs:3000,state:(_state,error)=>{if(error)errors.push(String(error));}}));
      }
    }
    const start=performance.now(),duration=clean60?60000:4000;markerStart=start;let stop=false;
    const accepted={audio:[0,0],video:[0,0]};
    const pumping=Promise.all([
      (async()=>{if(config.capture)return;for(let n=0;!stop;n++){
        for(let i=0;i<2;i++)if(connections[i].encodeAudio('browser-audio-'+i,Float32Array.from({length:960},(_,j)=>0.25*Math.sin(2*Math.PI*(i===0?500:250)*(n*960+j)/48000)),n*20000))accepted.audio[i]++;
        await wait(Math.max(0,start+(n+1)*20-performance.now()));
      }})(),
      (async()=>{for(let n=0;!stop;n++){
        for(let i=0;i<2;i++){paint(i,n);if(!config.capture&&connections[i].encodeVideo('browser-video-'+i,new VideoFrame(source[i],{timestamp:Math.round(n*1000000/30)})))accepted.video[i]++;}
        await wait(Math.max(0,start+(n+1)*1000/30-performance.now()));
      }})(),
    ]);void pumping.catch(()=>{});
    let readiness_ms,measurement;const playback=renderers.map(()=>({peakRms:0,maxPending:0,maxQueuedSeconds:0,queueSamples:0,queueSecondsTotal:0,meanQueuedAudioSeconds:0}));
    try{
      const readyUntil=performance.now()+5000;
      while(!audio.every(a=>a.identified>=5)||!video.every(v=>v.identified>=3)){
        check(performance.now()<readyUntil,'decoded readiness deadline '+JSON.stringify({audio,video:video.map(v=>({...v,motion:v.motion.size})),metrics:peers.map(p=>({aggregate:p.diagnostics,routes:p.routeDiagnostics})),accepted}));await wait(20);
      }
      readiness_ms=performance.now()-start;
    for(let i=0;i<2;i++){
      const bodies=[{type:'text',value:'identified sender '+i+' 😀'},{type:'json',value:{source:i,marker:'ordered-json'}},{type:'binary',value:new Uint8Array([0,255,17,i])}];
      for(const body of bodies)await connections[i].sendData('browser-data-'+i+'-'+body.type,'browser-message-'+i+'-'+body.type,'browser-'+body.type+'-'+i,body);
    }

      if(captureBoundary?.mode==='background-resume')await captureVisibilityBoundary(captureBoundary,captures,renderers,streams,peers);
      for(const a of audio){a.decoded=0;a.identified=0;}for(const v of video){v.decoded=0;v.identified=0;v.motion.clear();}
      const before=renderers.map(r=>r.diagnostics);
      const refusalBefore=audioRefusals.map(r=>({...r}));
      const measured=performance.now();minimumMarker=Math.floor((measured-markerStart)/1000)+2;measuring=true;
      while(performance.now()-measured<duration){for(let i=0;i<renderers.length;i++){const d=renderers[i].diagnostics,p=playback[i];p.peakRms=Math.max(p.peakRms,d.audioRms);p.maxPending=Math.max(p.maxPending,d.pendingAudio);p.maxQueuedSeconds=Math.max(p.maxQueuedSeconds,d.queuedAudioSeconds);p.queueSamples++;p.queueSecondsTotal+=d.queuedAudioSeconds;}if(clean60)observeAudioMarkers();await wait(clean60?5:20);}
      measuring=false;measurement={elapsed_ms:performance.now()-measured,opus:audio.map(a=>({...a})),vp8:video.map(v=>({...v,motion:v.motion.size}))};
      for(let i=0;i<renderers.length;i++){const after=renderers[i].diagnostics;playback[i].meanQueuedAudioSeconds=playback[i].queueSamples?playback[i].queueSecondsTotal/playback[i].queueSamples:0;playback[i].audio=after.audio-before[i].audio;playback[i].video=after.video-before[i].video;if(config.captureBoundary)playback[i].audio_refusal_observations=Object.fromEntries(Object.entries(audioRefusals[i]).map(([reason,n])=>[reason,n-(refusalBefore[i][reason]??0)]));}
    }finally{stop=true;await pumping;}
    check(errors.length===0,'actual SDK media failure: '+errors.join('; '));
    const observedAudio=measurement.opus,observedVideo=measurement.vp8,minimumAudio=clean60?Math.ceil(measurement.elapsed_ms*.0375):150,minimumVideo=clean60?Math.ceil(measurement.elapsed_ms*.024):90;
    for(let i=0;i<2;i++){
      check(observedAudio[i].identified>=minimumAudio,'actual decoded Opus insufficient '+JSON.stringify({audio:observedAudio,video:observedVideo,metrics:peers.map(p=>({aggregate:p.diagnostics,routes:p.routeDiagnostics})),accepted,readiness_ms,playback,measured_ms:measurement.elapsed_ms}));
      check(observedVideo[i].identified>=minimumVideo&&observedVideo[i].motion>=40,'actual rendered VP8 motion insufficient '+JSON.stringify({video:observedVideo,metrics:peers.map(p=>({aggregate:p.diagnostics,routes:p.routeDiagnostics})),readiness_ms}));
      check(data[i].length===3,'actual reliable data count');
      const other=1-i;for(const message of data[i]){
        const kind=message.body.type;check(message.from===config.participants[other]&&message.messageId==='browser-message-'+other+'-'+kind&&message.operationId==='browser-data-'+other+'-'+kind,'reliable original attribution/IDs');
        if(kind==='text')check(message.body.value==='identified sender '+other+' 😀','reliable UTF-8 text');
        if(kind==='json')check(message.body.value.source===other&&message.body.value.marker==='ordered-json','reliable JSON');
        if(kind==='binary')check(message.body.value.join(',')==='0,255,17,'+other,'reliable binary');
      }
    }
    const metrics=peers.map(p=>p.diagnostics);
    if(config.capture)for(let i=0;i<2;i++){
      check(playback[i].audio>=minimumAudio&&playback[i].video>=minimumVideo&&playback[i].peakRms>.02&&playback[i].maxPending<=10&&playback[i].maxQueuedSeconds<=.2,'actual bounded connected audio/video playback '+JSON.stringify(playback));
      for(const kind of ['audio','video'])accepted[kind][i]=peers[i].routeDiagnostics.find(r=>r.streamId==='browser-'+kind+'-'+i).counters.encoded;
    }
    const alignment=sync.map(state=>{
      const pairs=[...state.video].filter(([id])=>state.audio.has(id)).map(([id,at])=>({id,skew_ms:state.audio.get(id)-at})),absolute=pairs.map(p=>Math.abs(p.skew_ms)).sort((a,b)=>a-b);
      const middle=Math.floor(absolute.length*.5);return{pairs,paired_markers:pairs.length,audio_markers:state.audio.size,video_markers:state.video.size,median_abs_skew_ms:absolute.length?(absolute.length%2?absolute[middle]:(absolute[middle-1]+absolute[middle])/2):null,p95_abs_skew_ms:absolute.length?absolute[Math.ceil(absolute.length*.95)-1]:null};
    });
    if(clean60)for(const result of alignment)check(result.paired_markers>=42&&result.median_abs_skew_ms<=80&&result.p95_abs_skew_ms<=120,'frozen clean A/V marker alignment '+JSON.stringify(alignment)+' media_context='+JSON.stringify({opus:observedAudio,vp8:observedVideo,playback,metrics,readiness_ms,measured_ms:measurement.elapsed_ms}));
    const retiring=performance.now();
    for(let i=0;i<2;i++){
      const operation='browser-end-'+i,revision=connections[i].revision;
      try{await connections[i].end(operation);}catch(error){
        let status;try{status=await clients[i].operationStatus(config.media_cid,operation);}catch(e){status=String(e);}
        throw Error('original media end refused '+JSON.stringify({index:i,revision,operation,error:String(error),status,opus:audio,vp8:video.map(v=>({...v,motion:v.motion.size}))}));
      }
    }
    if(config.capture){
      await Promise.all(renderers.map(r=>r.finished));
      while(contexts.some(c=>!sourceContexts.has(c)&&c.state!=='closed')){check(performance.now()-retiring<5000,'actual connected graph teardown deadline');await wait(10);}
      check(captures.every(c=>c.closed)&&streams.every(s=>s.getTracks().every(t=>t.readyState==='ended'))&&renderers.every(r=>r.diagnostics.closed&&r.diagnostics.pendingAudio===0),'actual connected capture/playback ownership');
    }
    for(const peer of peers)peer.close();
    check(clients.every(c=>c.closed)&&peers.every(p=>p.diagnostics.activeBindings===0&&p.diagnostics.queuedBytes===0),'owned media cleanup');
    if(config.capture)check(performance.now()-retiring<=5000,'frozen terminal intent through confirmed ends/local owned teardown exceeds5seconds');
    if(captureBoundary)captureBoundary.passed=true;
    return{passed:true,production_sdk_media:true,capture_boundary:captureBoundary,actual_capture_render:config.capture===true,clean60,alignment:clean60?alignment:[],algorithm:config.actors[0].algorithm,created_cid:created.id,decoded_forward:true,decoded_reverse:true,reliable_data_duplex:true,owned_cleanup:true,opus:observedAudio,vp8:observedVideo,decoded_fps:observedVideo.map(v=>v.identified*1000/measurement.elapsed_ms),metrics,accepted,playback,owned_graph_states:contexts.filter(c=>!sourceContexts.has(c)).map(c=>c.state),owned_track_states:streams.flatMap(s=>s.getTracks().map(t=>({kind:t.kind,state:t.readyState}))),teardown_ms:performance.now()-retiring,readiness_ms,duration_ms:duration,measured_ms:measurement.elapsed_ms,scope:clean60?'60second actual supplied-stream capture/worklet/SDK/WT/Store/Core/decoded/rendered browser-browser Opus/VP8/data and uniquely paired graph/canvas markers; native/impairment/other browser profiles remain open':config.capture?'short actual supplied-stream capture/worklet/SDK/WebTransport/Store/Core/decoded-rendered Opus/VP8/data; native/full60s/sync/impairment remain open':'short synthetic source over actual SDK/control/WebTransport/Store/Core media and data; capture/playback/native/full60s/impairment remain open'};
  }finally{for(const c of captures)c.close();for(const r of renderers)r.close();for(const o of oscillators)o.stop();for(const stream of streams)for(const t of stream.getTracks())t.stop();for(const peer of peers)peer.close();for(const client of clients)client.close();await Promise.all(contexts.filter(c=>c.state!=='closed').map(c=>c.close().catch(()=>{})));globalThis.AudioContext=OriginalAudioContext;}
}
