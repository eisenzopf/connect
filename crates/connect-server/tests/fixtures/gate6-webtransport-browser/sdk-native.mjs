// Actual browser/public-native-client Opus/VP8/data functional profile.
import {BrowserClient} from '/sdk/browser_client.js';
import {BrowserMediaPeer} from '/sdk/browser_media.js';
import {BrowserMediaCapture} from '/sdk/browser_media_capture.js';
import {BrowserMediaRenderer} from '/sdk/browser_media_renderer.js';
import {options,tone} from '/sdk-media.mjs';
const check=(ok,message)=>{if(!ok)throw Error(message);};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
export async function run(config){
  check(config.native===true&&!config.clean60,'explicit short native functional profile');
  const state=async flag=>{
    const response=await fetch('/native-state'+(flag?'?set='+encodeURIComponent(flag):''),{signal:AbortSignal.timeout(2000)});
    check(response.ok,'native fixture coordination HTTP status');return response.json();
  };
  const until=async flag=>{const deadline=performance.now()+15000;while(!(await state())[flag]){check(performance.now()<deadline,'native coordination deadline '+flag);await wait(20);}};
  let client,peer,connection,capture,renderer,context,oscillator,stream,stop=false,pumping;
  const errors=[],audio={decoded:0,identified:0},video={decoded:0,identified:0,motion:new Set()},data=[];
  const source=document.createElement('canvas'),output=document.createElement('canvas');
  source.width=output.width=640;source.height=output.height=360;
  const paint=n=>{const c=source.getContext('2d');c.fillStyle='rgb(220,20,20)';c.fillRect(0,0,640,360);c.fillStyle='rgb(10,230,10)';c.fillRect((n*7)%608,80,32,200);};
  try{
    client=await BrowserClient.open(await options(config.actors[0]));
    const created=await client.createConversation('browser-create',{participants:[config.actors[1].identity_id]});
    peer=new BrowserMediaPeer(client,{
      audio:(binding,frame)=>{audio.decoded++;if(tone(frame,250))audio.identified++;renderer?.audio(binding,frame);},
      video:(binding,frame)=>{
        video.decoded++;const out=output.getContext('2d',{willReadFrequently:true});
        if(config.capture){if(!renderer?.video(binding,frame))return;}else out.drawImage(frame,0,0,640,360);
        const pixel=out.getImageData(0,0,1,1).data;if(pixel[2]>150&&pixel[0]<80)video.identified++;
        const row=out.getImageData(0,180,640,1).data;let sum=0,count=0;
        for(let x=0;x<640;x++)if(row[x*4+1]>120&&row[x*4+1]>row[x*4]*1.5&&row[x*4+1]>row[x*4+2]*1.5){sum+=x;count++;}
        if(count>8)video.motion.add(Math.round(sum/count));
      },data:message=>data.push(message),error:error=>errors.push(String(error)),
    });
    connection=await peer.offerConnection({conversationId:config.media_cid,sessionId:'http3-media-session',connectionId:'browser-connection-0',operationId:'browser-offer-0',audio:'browser-audio-0',video:'browser-video-0',data:['text','json','binary'].map(protocol=>({id:'browser-'+protocol+'-0',protocol}))});
    await state('browser-offered');await until('native-offered');
    await connection.subscribe('browser-subscribe-0',config.participants[1],['audio','video','data']);
    await state('browser-subscribed');await until('native-subscribed');
    if(config.capture){
      renderer=await BrowserMediaRenderer.open(connection,{video:{'browser-video-1':output},volume:0,timeoutMs:3000});
      context=new AudioContext({sampleRate:48000});oscillator=context.createOscillator();oscillator.frequency.value=500;
      const gain=context.createGain(),destination=context.createMediaStreamDestination();gain.gain.value=.25;oscillator.connect(gain);gain.connect(destination);oscillator.start();await context.resume();
      paint(0);stream=new MediaStream([...destination.stream.getAudioTracks(),...source.captureStream(30).getVideoTracks()]);
      capture=await BrowserMediaCapture.open(connection,{audioStreamId:'browser-audio-0',videoStreamId:'browser-video-0',stream,timeoutMs:3000,state:(_state,error)=>{if(error)errors.push(String(error));}});
    }
    const start=performance.now();const accepted={audio:0,video:0};
    pumping=Promise.all([
      (async()=>{if(config.capture)return;for(let n=0;!stop;n++){
        if(connection.encodeAudio('browser-audio-0',Float32Array.from({length:960},(_,j)=>.25*Math.sin(2*Math.PI*500*(n*960+j)/48000)),n*20000))accepted.audio++;
        await wait(Math.max(0,start+(n+1)*20-performance.now()));
      }})(),
      (async()=>{for(let n=0;!stop;n++){
        paint(n);if(!config.capture&&connection.encodeVideo('browser-video-0',new VideoFrame(source,{timestamp:Math.round(n*1000000/30)})))accepted.video++;
        await wait(Math.max(0,start+(n+1)*1000/30-performance.now()));
      }})(),
    ]);void pumping.catch(()=>{});
    const readyDeadline=performance.now()+5000;
    while(audio.identified<5||video.identified<3){check(performance.now()<readyDeadline,'browser decoded native readiness '+JSON.stringify({audio,video:{...video,motion:video.motion.size},routes:peer.routeDiagnostics}));await wait(20);}
    await state('data-go');
    for(const body of [{type:'text',value:'identified sender 0 😀'},{type:'json',value:{source:0,marker:'ordered-json'}},{type:'binary',value:new Uint8Array([0,255,17,0])}])await connection.sendData('browser-data-0-'+body.type,'browser-message-0-'+body.type,'browser-'+body.type+'-0',body);
    await until('native-data-sent');
    const dataDeadline=performance.now()+3000;while(data.length<3){check(performance.now()<dataDeadline,'native reliable data delivery deadline');await wait(20);}
    await state('measure-start');
    audio.decoded=audio.identified=video.decoded=video.identified=0;video.motion.clear();
    const before=renderer?.diagnostics;let peakRms=0,maxPending=0,maxQueuedSeconds=0;
    const measured=performance.now();
    while(performance.now()-measured<4000){if(renderer){const d=renderer.diagnostics;peakRms=Math.max(peakRms,d.audioRms);maxPending=Math.max(maxPending,d.pendingAudio);maxQueuedSeconds=Math.max(maxQueuedSeconds,d.queuedAudioSeconds);}await wait(10);}
    const elapsed=performance.now()-measured,observedAudio={...audio},observedVideo={...video,motion:video.motion.size},after=renderer?.diagnostics;
    await state('measure-end');
    stop=true;await pumping;
    check(errors.length===0,'browser/native media errors '+errors.join('; '));
    check(observedAudio.identified>=150&&observedVideo.identified>=90&&observedVideo.motion>=40,'browser independently decoded native source '+JSON.stringify({audio:observedAudio,video:observedVideo,routes:peer.routeDiagnostics}));
    check(data.length===3,'exact native reliable messages');
    for(const message of data){const kind=message.body.type;check(message.from===config.participants[1]&&message.messageId==='browser-message-1-'+kind&&message.operationId==='browser-data-1-'+kind,'native original reliable attribution');if(kind==='text')check(message.body.value==='identified sender 1 😀','native UTF-8 body');if(kind==='json')check(message.body.value.source===1&&message.body.value.marker==='ordered-json','native JSON body');if(kind==='binary')check(message.body.value.join(',')==='0,255,17,1','native binary body');}
    let playback;
    if(renderer){playback={audio:after.audio-before.audio,video:after.video-before.video,peakRms,maxPending,maxQueuedSeconds};check(playback.audio>=150&&playback.video>=90&&peakRms>.02&&maxPending<=10&&maxQueuedSeconds<=.2,'bounded browser playback of native source '+JSON.stringify(playback));}
    const terminal=performance.now();await connection.end('browser-end-0');await state('native-end');await until('native-ended');
    if(renderer){await renderer.finished;check(capture.closed&&stream.getTracks().every(t=>t.readyState==='ended')&&renderer.diagnostics.closed&&renderer.diagnostics.pendingAudio===0,'browser owned capture/render cleanup');}
    const diagnostics=peer.diagnostics;peer.close();check(client.closed&&peer.diagnostics.activeBindings===0&&peer.diagnostics.queuedBytes===0,'browser physical media cleanup');
    check(performance.now()-terminal<=5000,'browser/native terminal teardown exceeds5seconds');
    return {passed:true,production_sdk_media:true,canonical_public_native_client:true,native_transport:'quic',actual_capture_render:config.capture===true,algorithm:config.actors[0].algorithm,created_cid:created.id,decoded_forward:true,reliable_data_duplex:true,owned_cleanup:true,opus:observedAudio,vp8:observedVideo,measured_ms:elapsed,metrics:diagnostics,playback,teardown_ms:performance.now()-terminal,scope:'Short browser/public CanonicalClient QUIC Opus/VP8/data functional profile. Native received packets require separate independent decoder evidence. Native live encoding/playout, full60second/sync/impairment/operating qualification remain open.'};
  }finally{stop=true;if(pumping)await pumping.catch(()=>{});capture?.close();renderer?.close();oscillator?.stop();if(stream)for(const track of stream.getTracks())track.stop();peer?.close();client?.close();if(context&&context.state!=='closed')await context.close().catch(()=>{});}
}
