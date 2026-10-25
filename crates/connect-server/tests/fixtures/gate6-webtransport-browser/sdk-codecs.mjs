// Short primitive checkpoint using production codec/RTP modules. No network/capture claim.
import {BrowserOpusCodec,BrowserVp8Codec,browserCodecSupport,RtpClock} from '/sdk/browser_codecs.js';
import {RtpSender,Vp8Assembler,decodeRtpDatagram} from '/sdk/browser_rtp.js';
const check=(ok,message)=>{if(!ok)throw Error(message);};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const binding=(handle,kind)=>({handle,streamId:'codec-'+handle,connectionId:'codec-connection',sessionId:'codec-session',generation:1,kind,codec:kind==='audio'?'opus':'vp8',payloadType:kind==='audio'?111:96,direction:'sendrecv'});
function tone(frame,hz){
  check(frame.sampleRate===48000&&frame.numberOfChannels===1,'decoded Opus profile');
  const pcm=new Float32Array(frame.numberOfFrames);frame.copyTo(pcm,{planeIndex:0,format:'f32-planar'});
  const power=f=>{let re=0,im=0;for(let n=0;n<pcm.length;n++){const angle=2*Math.PI*f*n/48000;re+=pcm[n]*Math.cos(angle);im+=pcm[n]*Math.sin(angle);}return re*re+im*im;};
  return power(hz)>power(hz===500?250:500)*4&&pcm.some(v=>Math.abs(v)>0.05);
}
export async function run(config={}){
  const wholeFrameLoss=config.whole_frame_loss===true,loss=[undefined,undefined];
  const support=await browserCodecSupport();check(support.audio&&support.video,'frozen WebCodecs profile unavailable: '+support.missing.join(', '));
  const codecs=[],audio=[{decoded:0,identified:0},{decoded:0,identified:0}],video=[{decoded:0,identified:0,motion:new Set()},{decoded:0,identified:0,motion:new Set()}];
  const errors=[],audioCodecs=[],videoCodecs=[],assemblers=[new Vp8Assembler(),new Vp8Assembler()],audioClocks=[new RtpClock(48000),new RtpClock(48000)],videoClocks=[new RtpClock(90000),new RtpClock(90000)];
  const source=[document.createElement('canvas'),document.createElement('canvas')],render=[document.createElement('canvas'),document.createElement('canvas')];
  for(const canvas of [...source,...render]){canvas.width=640;canvas.height=360;}
  let packets=0,maxPacketBytes=0;
  try{
    for(let i=0;i<2;i++){
      const target=1-i,as=new RtpSender(binding(i+1,'audio')),vs=new RtpSender(binding(i+3,'video'));
      const a=new BrowserOpusCodec({encoded:frame=>{
        for(const wire of as.packetize(frame.bytes,Math.round(frame.timestampUs*48000/1000000)>>>0)){
          packets++;maxPacketBytes=Math.max(maxPacketBytes,wire.length);const p=decodeRtpDatagram(wire);audioCodecs[target].decode(p.payload,audioClocks[target].timestamp(p.timestamp));
        }
      },decoded:frame=>{audio[i].decoded++;if(tone(frame,i===0?250:500))audio[i].identified++;},error:e=>errors.push(String(e))});
      const v=new BrowserVp8Codec({encoded:frame=>{
        const wires=vs.packetize(frame.bytes,Math.round(frame.timestampUs*90000/1000000)>>>0);
        if(wholeFrameLoss&&!loss[target]&&frame.timestampUs>=500000){loss[target]={at_ms:performance.now(),timestamp_us:frame.timestampUs,packets:wires.length};return;}
        for(const wire of wires){
          packets++;maxPacketBytes=Math.max(maxPacketBytes,wire.length);const p=decodeRtpDatagram(wire),complete=assemblers[target].push(p);
          if(complete)videoCodecs[target].decode(complete,videoClocks[target].timestamp(p.timestamp));
        }
      },decoded:frame=>{
        const out=render[i].getContext('2d',{willReadFrequently:true});out.drawImage(frame,0,0,640,360);const pixel=out.getImageData(0,0,1,1).data;
        video[i].decoded++;const identified=i===0?pixel[2]>150&&pixel[0]<80:pixel[0]>150&&pixel[2]<80;if(identified)video[i].identified++;
        if(identified&&loss[i]&&loss[i].recovered_ms===undefined&&frame.timestamp>loss[i].timestamp_us){loss[i].recovered_ms=performance.now()-loss[i].at_ms;loss[i].recovered_timestamp_us=frame.timestamp;}
        const row=out.getImageData(0,180,640,1).data;let sum=0,count=0;
        for(let x=0;x<640;x++)if(row[x*4+1]>120&&row[x*4+1]>row[x*4]*1.5&&row[x*4+1]>row[x*4+2]*1.5){sum+=x;count++;}
        if(count>8)video[i].motion.add(Math.round(sum/count));
      },error:e=>errors.push(String(e))});
      audioCodecs.push(a);videoCodecs.push(v);codecs.push(a,v);
    }
    const start=performance.now();
    await Promise.all([
      (async()=>{for(let n=0;n<200;n++){
        for(let i=0;i<2;i++){const samples=Float32Array.from({length:960},(_,j)=>0.25*Math.sin(2*Math.PI*(i===0?500:250)*(n*960+j)/48000));audioCodecs[i].encode(samples,n*20000);}
        await wait(Math.max(0,start+(n+1)*20-performance.now()));
      }})(),
      (async()=>{for(let n=0;n<120;n++){
        for(let i=0;i<2;i++){const c=source[i].getContext('2d');c.fillStyle=i===0?'rgb(220,20,20)':'rgb(20,20,220)';c.fillRect(0,0,640,360);c.fillStyle='rgb(10,230,10)';c.fillRect((n*(i===0?7:11))%608,80,32,200);videoCodecs[i].encode(new VideoFrame(source[i],{timestamp:Math.round(n*1000000/30)}));}
        await wait(Math.max(0,start+(n+1)*1000/30-performance.now()));
      }})(),
    ]);
    check(errors.length===0,'production codec error: '+errors.join('; '));
    for(const a of audio)check(a.identified>=150,'insufficient independently identified decoded Opus frames');
    for(const v of video)check(v.identified>=90&&v.motion.size>=40,'insufficient identified rendered VP8 motion');
    check(maxPacketBytes<=1188,'frozen RTP MTU');
    if(wholeFrameLoss)for(const observed of loss)check(observed&&observed.packets>0&&observed.recovered_ms<=2000&&observed.recovered_timestamp_us>=1000000,'whole missing VP8 frame must wait for an identified keyframe within2seconds: '+JSON.stringify(loss));
    return {passed:true,whole_frame_loss:wholeFrameLoss,recovery:wholeFrameLoss?loss:[],scope:'short synthetic SDK codec/RTP/canvas primitive checkpoint only; no capture, network, native, full 60s operating profile or G6-050 completion claim',elapsed_ms:performance.now()-start,opus:audio,vp8:video.map(v=>({...v,motion:v.motion.size})),packets,max_packet_bytes:maxPacketBytes,errors};
  }finally{for(const c of codecs)c.close();for(const a of assemblers)a.clear();}
}
