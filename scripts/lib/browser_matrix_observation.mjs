import {browserMatrixRtcObservation} from './browser_matrix_rtc_observation.mjs';
import {browserMatrixPipeline} from './browser_matrix_pipeline.mjs';
import {classifySourceFrame,sourceIdentityReport} from '../video/source_identity.mjs';
import {ordinalSync} from '../video/marker_calibration.mjs';
import {AudioEnvelope,combineAudioEnvelopeSync} from '../video/audio_envelope.mjs';
import {pcmIdentity} from './browser_matrix_pcm.mjs';
export {pcmIdentity} from './browser_matrix_pcm.mjs';
import {pcmBoundary,browserMatrixRtcStages} from './browser_matrix_stages.mjs';
const check=(ok,message)=>{if(!ok)throw Error(message);};

export class DecodedRoute {
    constructor(index,expected){this.index=index;this.expected=expected;this.audio=[];this.video=[];this.envelope=new AudioEnvelope();
        this.audioBoundaries=[];this.omittedAudioBoundaries=0;this.invalid=[];this.seen=new Set();this.videoHigh=false;this.lastPixels=null;this.videoEdges=[];this.begin=null;}
    start(originMs){check(this.begin===null&&Number.isFinite(originMs),'One original measurement clock');this.begin=originMs;}
    audioFrame(owner,timestamp,at,samples,codec=null){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.audio.length<16000,'Decoded audio bound');
        const identity=pcmIdentity(samples,this.expected),key=owner+':'+timestamp,duplicate=this.seen.has(key);
        this.seen.add(key);this.audio.push({owner,seconds,timestamp,duplicate,...identity});this.envelope.sample(seconds,identity.rms);
        if(!identity.identified||identity.rms<.03){if(this.audioBoundaries.length<32)this.audioBoundaries.push({owner,original_timestamp_us:timestamp,
            callback_seconds:seconds,identity,boundary:pcmBoundary(samples),codec});else this.omittedAudioBoundaries++;}
    }
    videoFrame(owner,timestamp,at,gray,pixels,width,height){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.video.length<10000,'Decoded Video bound');
        const kind=classifySourceFrame(gray),key=owner+':v:'+timestamp,duplicate=this.seen.has(key);this.seen.add(key);
        const changed=this.lastPixels!==null&&pixels.some((v,i)=>v!==this.lastPixels[i]);this.lastPixels=pixels.slice();
        if(kind==='pulse'&&!this.videoHigh)this.videoEdges.push(seconds);this.videoHigh=kind==='pulse';
        this.video.push({owner,seconds,timestamp,kind,duplicate,changed,dimensions:width===640&&height===360});
    }
    report(){
        const start=1,end=61,seconds=60,inWindow=row=>row.seconds>=start&&row.seconds<end;
        const audio=this.audio.filter(inWindow),video=this.video.filter(inWindow),identified=audio.filter(row=>row.identified&&!row.duplicate);
        const boundaries=[start,...identified.map(row=>row.seconds),end];
        const maximumGap=Math.max(...boundaries.slice(1).map((at,i)=>Math.max(0,at-boundaries[i]-(i===0?0:.02))));
        const counts={};for(const row of video)counts[row.kind]=(counts[row.kind]??0)+1;
        const perSecond=Array(60).fill(0);for(const row of video)if(row.changed&&!row.duplicate&&row.dimensions&&
            [String(this.expected),'pulse'].includes(row.kind))perSecond[Math.floor(row.seconds-start)]++;
        const identity=sourceIdentityReport(counts,this.expected),envelope=this.envelope.report(start);
        const sync=combineAudioEnvelopeSync(ordinalSync(this.videoEdges.filter(at=>at>=start&&at<end),
            envelope.accepted_edge_seconds.filter(at=>at<end),seconds),envelope);
        const audioResult={decoded_seconds:audio.length*.02,identified_seconds:identified.length*.02,
            maximum_gap_ms:maximumGap*1000,bad_identity_frames:audio.filter(row=>!row.identified).length,
            duplicate_frames:audio.filter(row=>row.duplicate).length,
            passed:identified.length*.02>=59.9&&maximumGap<=.2&&audio.every(row=>row.identified&&!row.duplicate)};
        const decodedPerSecond=Array(60).fill(0);for(const row of video)decodedPerSecond[Math.floor(row.seconds-start)]++;
        const callbackTimes=[start,...video.map(row=>row.seconds),end];
        const videoResult={decoded_frames_per_second:decodedPerSecond,
            maximum_decode_callback_gap_ms:Math.max(...callbackTimes.slice(1).map((at,i)=>Math.max(0,at-callbackTimes[i])))*1000,
            decoded_frames:video.length,changing_frames_per_second:perSecond,minimum_fps:Math.min(...perSecond),
            identity,dimension_refusals:video.filter(row=>!row.dimensions).length,duplicate_frames:video.filter(row=>row.duplicate).length,
            passed:Math.min(...perSecond)>=24&&identity.status==='passed'&&video.every(row=>row.dimensions&&!row.duplicate)};
        return {route:this.index,expected_source_id:this.expected,window_start_ms:1000,window_end_ms:61000,
            audio:audioResult,video:videoResult,sync,invalid:this.invalid,
            audio_boundary_diagnostics:{scope:'Actual low-RMS or unidentified PCM; diagnostic only, no quality exception',limit:32,
                omitted:this.omittedAudioBoundaries,events:this.audioBoundaries},
            passed:!this.invalid.length&&audioResult.passed&&videoResult.passed&&sync.status==='passed'};
    }
}

/** Fixture-owned observer of actual borrowed decoder outputs. The public render
 * owner still receives every frame normally. Copies contain only this run's
 * synthetic media; no borrowed frame is retained or closed by the observer. */
export function browserMatrixObservation(Renderer,plan,Peer,Rtc) {
    const routes=plan.routes.map(route=>{
        const peers=plan.routes.filter(other=>other.session===route.session),at=peers.indexOf(route);
        const selected=peers[(at+peers.length-1)%peers.length];
        return {measure:new DecodedRoute(route.index,Number(selected.role.split('-').at(-1))-1),selected,active:null};
    });
    const bindings=new Map(),owners=[],originalAudio=Renderer.prototype.audio,originalVideo=Renderer.prototype.video;
    let closed=false,closing;const rtcStages=Rtc?browserMatrixRtcStages():null;
    const pipeline=Peer?browserMatrixPipeline(Peer,(connection,kind,stream)=>{
        const owner=bindings.get(connection);
        return owner&&routes[owner.index].active===connection&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;
    },(connection,kind,stream)=>{const owner=bindings.get(connection);
        return owner&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;}):null;
    function observe(binding,frame,kind){
        const owner=bindings.get(binding.connectionId);if(!owner||routes[owner.index].active!==binding.connectionId)return;
        if(binding.streamId!==owner[kind])return;
        const route=routes[owner.index].measure,at=performance.now();
        try{
            if(kind==='audio'){
                check(frame.sampleRate===48000&&frame.numberOfChannels===1&&frame.numberOfFrames===960,'Decoded audio format');
                const samples=new Float32Array(960);frame.copyTo(samples,{planeIndex:0,format:'f32-planar'});
                route.audioFrame(binding.connectionId,frame.timestamp,at,samples,pipeline?.snapshot(binding.connectionId)??null);
            }else{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                route.videoFrame(binding.connectionId,frame.timestamp,at,gray,pixels,frame.displayWidth,frame.displayHeight);
            }
        }catch(error){if(route.invalid.length<16)route.invalid.push(String(error.message));}
    }
    const invalidRtc=(connection,message)=>{const owner=bindings.get(connection);if(!owner)return;
        const route=routes[owner.index].measure;if(route.invalid.length<16)route.invalid.push(message);};
    const rtc=Rtc?browserMatrixRtcObservation(Rtc,{
        audio(connection,timestamp,at,samples){
            const owner=bindings.get(connection);if(!owner||routes[owner.index].active!==connection)return;
            try{routes[owner.index].measure.audioFrame(connection,timestamp,at,samples);}
            catch(error){invalidRtc(connection,String(error.message));}
        },
        video(connection,timestamp,at,frame,width,height){
            const owner=bindings.get(connection);if(!owner||routes[owner.index].active!==connection)return;
            try{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                routes[owner.index].measure.videoFrame(connection,timestamp,at,gray,pixels,width,height);
            }catch(error){invalidRtc(connection,String(error.message));}
        },
        invalid:invalidRtc,
    }):null;
    Renderer.prototype.audio=function(binding,frame){observe(binding,frame,'audio');return originalAudio.call(this,binding,frame);};
    Renderer.prototype.video=function(binding,frame){observe(binding,frame,'video');return originalVideo.call(this,binding,frame);};
    return {
        async attach(index,connection,{audio,video,sendVideo,active=false},options){
            check(!closed&&!bindings.has(connection.id)&&owners.length<10,'Bounded original render owners');
            const canvas=document.createElement('canvas');canvas.width=128;canvas.height=72;
            const context=canvas.getContext('2d',{willReadFrequently:true});context.imageSmoothingEnabled=false;
            bindings.set(connection.id,{index,audio,video,sendVideo,context,sourceId:Number(plan.routes[index].role.split('-').at(-1))-1});
            const output=document.createElement('canvas');rtc?.attach(connection.id,output);
            const owner=await connection.render({audio:true,video:[{stream:connection.streams.byId(video),canvas:output}]},options);
            owners.push(owner);if(active)routes[index].active=connection.id;
        },
        async initial(state,options){for(const route of plan.routes){const selected=routes[route.index].selected;
            await this.attach(route.index,state.connections[route.index],{audio:route.audio,video:Rtc?route.video:selected.video,sendVideo:route.video,active:true},options);}},
        start(origin){pipeline?.start(origin.originMs);rtcStages?.start(origin.originMs);for(const route of routes)route.measure.start(origin.originMs);},
        activate(index,connectionId){check(bindings.get(connectionId)?.index===index,'Original prepared observer required');routes[index].active=connectionId;},
        report(){const measured=routes.map(route=>route.measure.report());return {format:'connect/browser-matrix-decoded-media/1',
            method:Rtc?'Independent content measurements on actual WebRTC render-graph PCM and video-frame pixels at local receive callback times; no physical playback claim':
                'Independent content measurements on actual WebCodecs decoded PCM/pixels at local receive callback times; no physical playback claim',
            window_ms:60000,source_counters_used:false,rtc_media_pipeline:rtcStages?.report()??null,
            pipeline:pipeline?.report(routes.map((route,index)=>({index,source:route.selected.index,
                audio:route.measure.envelope.edges,audioEnvelope:route.measure.envelope.report(1),video:route.measure.videoEdges}))),passed:measured.every(route=>route.passed),routes:measured};},
        close(){return closing??=(async()=>{closed=true;pipeline?.close();Renderer.prototype.audio=originalAudio;Renderer.prototype.video=originalVideo;
            // Stop scheduling diagnostics at original close, before joining
            // render owners. A pending original getStats is never a fake join.
            const statisticsWork=rtcStages?.close();
            for(const owner of owners)owner.close();const outcomes=await Promise.allSettled(owners.map(owner=>owner.finished));
            await rtc?.close();const statistics=await statisticsWork,count=owners.length;owners.length=0;bindings.clear();
            return {owners:count,statistics,joined:outcomes.every(value=>value.status==='fulfilled')&&(!rtcStages||statistics?.joined===true)};
        })();},
    };
}
