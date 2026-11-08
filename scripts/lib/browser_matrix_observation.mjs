import {nativeMembership} from "../../sdk/typescript/dist/browser_native_activation.js";
import {browserMatrixRtcObservation} from './browser_matrix_rtc_observation.mjs';
import {browserMatrixPipeline} from './browser_matrix_pipeline.mjs';
import {classifySourceFrame,sourceIdentityReport} from '../video/source_identity.mjs';
import {ordinalSync} from '../video/marker_calibration.mjs';
import {AudioEnvelope,combineAudioEnvelopeSync} from '../video/audio_envelope.mjs';
import {pcmIdentity} from './browser_matrix_pcm.mjs';
export {pcmIdentity} from './browser_matrix_pcm.mjs';
import {pcmBoundary,browserMatrixRtcStages} from './browser_matrix_stages.mjs';
import {transitionCoverage,cleanBinDurations} from './media_transition_coverage.mjs';
const check=(ok,message)=>{if(!ok)throw Error(message);};

function transitionEnvelope(rows,coverage,since,original) {
    if(!coverage.transitions.length)return original;
    const reports=[];let envelope=new AudioEnvelope(),next=0;
    for(const row of rows){
        if(next<coverage.transitions.length&&row.seconds*1000>=coverage.transitions[next].resumed_sample_ms){
            reports.push(envelope.report(since));envelope=new AudioEnvelope();next++;
        }
        envelope.sample(row.seconds,row.rms);
    }
    reports.push(envelope.report(since));
    return {...original,profile:'connect/transition-segmented-audio-envelope/1',
        status:reports.every(report=>report.status==='observer-valid')?'observer-valid':'observer-invalid',
        invalid:reports.flatMap(report=>report.invalid),
        accepted_edge_seconds:reports.flatMap(report=>report.accepted_edge_seconds),
        segments:reports,original_unsegmented_observer:original,
        onset_state_crossed_transition:false,
        scope:'Original decoded RMS, split only at measured contract-bounded public handoff gaps; onset proof restarts after every gap'};
}

export class DecodedRoute {
    constructor(index,expected){this.index=index;this.expected=expected;this.audio=[];this.video=[];this.envelope=new AudioEnvelope();
        this.audioBoundaries=[];this.omittedAudioBoundaries=0;this.invalid=[];this.seen=new Set();this.videoHigh=false;this.lastPixels=null;this.videoEdges=[];this.begin=null;this.ownerPixels=new Map();
        this.audioCuts=[];this.videoCuts=[];}
    start(originMs){check(this.begin===null&&Number.isFinite(originMs),'One original measurement clock');this.begin=originMs;}
    audioFrame(owner,timestamp,at,samples,codec=null,native=null){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.audio.length<16000,'Decoded audio bound');
        const identity=pcmIdentity(samples,this.expected),key=owner+':'+timestamp,duplicate=this.seen.has(key);
        this.seen.add(key);this.audio.push({owner,seconds,timestamp,duplicate,native,...identity});this.envelope.sample(seconds,identity.rms);
        if(!identity.identified||identity.rms<.03){if(this.audioBoundaries.length<32)this.audioBoundaries.push({owner,original_timestamp_us:timestamp,
            callback_seconds:seconds,identity,boundary:pcmBoundary(samples),codec});else this.omittedAudioBoundaries++;}
    }
    videoFrame(owner,timestamp,at,gray,pixels,width,height,native=null){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.video.length<10000,'Decoded Video bound');
        const kind=classifySourceFrame(gray),key=owner+':v:'+timestamp,duplicate=this.seen.has(key);this.seen.add(key);
        let changed;
        if(native!==null&&native!==undefined){
            // Native private preparation has its own physical history. It must
            // not borrow an old owner's last pixels as decoder-startup evidence.
            const previous=this.ownerPixels.get(owner);check(this.ownerPixels.has(owner)||this.ownerPixels.size<10,"Bounded original pixel histories");
            changed=previous!==undefined&&pixels.some((v,i)=>v!==previous[i]);this.ownerPixels.set(owner,pixels.slice());
        }else{
            // RTC/default observations retain the original logical-route
            // comparison across an already validated owner switch.
            changed=this.lastPixels!==null&&pixels.some((v,i)=>v!==this.lastPixels[i]);this.lastPixels=pixels.slice();
        }
        if(kind==='pulse'&&!this.videoHigh)this.videoEdges.push(seconds);this.videoHigh=kind==='pulse';
        this.video.push({owner,seconds,timestamp,kind,duplicate,changed,native,dimensions:width===640&&height===360});
    }
    workingState(){return {audio_rows:this.audio.length,video_rows:this.video.length,
        duplicate_keys:this.seen.size,audio_boundaries:this.audioBoundaries.length,invalid_entries:this.invalid.length,
        audio_edges:this.envelope.edges.length,audio_transitions:this.envelope.transitions.length,
        audio_oversized:this.envelope.oversized.length,envelope_invalid:this.envelope.invalid.length,
        video_edges:this.videoEdges.length,audio_cuts:this.audioCuts.length,video_cuts:this.videoCuts.length,
        last_pixel_bytes:this.lastPixels?.byteLength??0,
        owner_pixel_bytes:[...this.ownerPixels.values()].reduce((sum,pixels)=>sum+pixels.byteLength,0)};}
    retireWorkingState(){
        // Captured reports alias some diagnostic arrays. Reassign each working
        // field, never truncate or mutate evidence already returned to callers.
        this.audio=[];this.video=[];this.seen=new Set();this.audioBoundaries=[];this.invalid=[];
        this.envelope=new AudioEnvelope();this.videoEdges=[];this.audioCuts=[];this.videoCuts=[];
        this.lastPixels=null;this.ownerPixels=new Map();this.omittedAudioBoundaries=0;
        this.videoHigh=false;this.begin=null;
    }
    report(include=()=>true){
        const originalAudio=this.audio.filter(row=>include(row,'audio')),originalVideo=this.video.filter(row=>include(row,'video'));
        // Rebuild only bounded numeric observations. Original callbacks, times,
        // bytes and decoded media are neither replayed nor altered.
        this.envelope=new AudioEnvelope();for(const row of originalAudio)this.envelope.sample(row.seconds,row.rms);
        this.videoEdges=[];let high=false;for(const row of originalVideo){if(row.kind==='pulse'&&!high)this.videoEdges.push(row.seconds);high=row.kind==='pulse';}
        const start=1,end=61,seconds=60,inWindow=row=>row.seconds>=start&&row.seconds<end;
        const audio=originalAudio.filter(inWindow),video=originalVideo.filter(inWindow),identified=audio.filter(row=>row.identified&&!row.duplicate);
        const boundaries=[start,...identified.map(row=>row.seconds),end];
        const maximumGap=Math.max(...boundaries.slice(1).map((at,i)=>Math.max(0,at-boundaries[i]-(i===0?0:.02))));
        const audioCoverage=transitionCoverage(identified.map(row=>row.seconds*1000),this.audioCuts,
            {startMs:start*1000,endMs:end*1000,frameMs:20,minimumGapMs:50});
        const quantumSeen=new Set(),videoSourceSeen=new Set();let sourceOverlaps=0,videoSourceOverlaps=0,sourceIdentified=0;
        const originNtp=((BigInt(Math.round((performance.timeOrigin+this.begin)*1000))/1000000n+2208988800n)<<32n)+
            ((BigInt(Math.round((performance.timeOrigin+this.begin)*1000))%1000000n)<<32n)/1000000n;
        let sourceUnbound=0;const sourceAnchors=new Map();
        const sourceKey=row=>JSON.stringify([row.owner,row.native.identity.physical_incarnation,row.native.ssrc]);
        for(const row of originalAudio)if(row.native?.sourceClock){const key=sourceKey(row),clock=row.native.sourceClock;
            let anchors=sourceAnchors.get(key);if(!anchors){anchors=new Map();sourceAnchors.set(key,anchors);}
            const id=JSON.stringify(clock);check(anchors.has(id)||anchors.size<512,'Bounded original source clock anchors');anchors.set(id,clock);
        }
        for(const row of audio)if(row.native){
            const anchors=sourceAnchors.get(sourceKey(row))?.values()??[];
            const quanta=new Set();
            for(const clock of anchors){const delta=(row.native.rtpTimestamp-clock.rtpTimestamp)|0;
                if(delta===-2147483648||Math.abs(delta)>3*48000)continue;
                const denominator=960n*(1n<<32n),num=(BigInt(clock.ntp)-originNtp)*48000n+BigInt(delta)*(1n<<32n);
                const rounded=num+denominator/2n;const quantum=Number(rounded>=0n?rounded/denominator:-((-rounded+denominator-1n)/denominator));if(quantum>=0&&quantum<3100)quanta.add(quantum);
            }
            if(quanta.size!==1){sourceUnbound++;continue;}const quantum=[...quanta][0];sourceIdentified++;
            if(quantumSeen.has(quantum))sourceOverlaps++;quantumSeen.add(quantum);
        }
        // The frozen source video clock retains its frame timestamp across
        // physical moves. Quantized callback times are never duplicate keys.
        for(const row of video)if(row.native){const key=row.native.rtpTimestamp;if(videoSourceSeen.has(key))videoSourceOverlaps++;videoSourceSeen.add(key);}
        const counts={};for(const row of video)counts[row.kind]=(counts[row.kind]??0)+1;
        const perSecond=Array(60).fill(0);for(const row of video)if(row.changed&&!row.duplicate&&row.dimensions&&
            [String(this.expected),'pulse'].includes(row.kind))perSecond[Math.floor(row.seconds-start)]++;
        const changingVideo=video.filter(row=>row.changed&&!row.duplicate&&row.dimensions&&[String(this.expected),'pulse'].includes(row.kind));
        const videoCoverage=transitionCoverage(changingVideo.map(row=>row.seconds*1000),this.videoCuts,
            {startMs:start*1000,endMs:end*1000,frameMs:1000/30,minimumGapMs:50});
        const requiredFrames=cleanBinDurations(videoCoverage,start*1000,seconds).map(ms=>Math.ceil(ms*24/1000-1e-9));
        const identity=sourceIdentityReport(counts,this.expected);
        const envelope=transitionEnvelope(originalAudio,audioCoverage,start,this.envelope.report(start));
        if(videoCoverage.transitions.length){
            this.videoEdges=[];let high=null,next=0;
            for(const row of originalVideo){
                if(next<videoCoverage.transitions.length&&row.seconds*1000>=videoCoverage.transitions[next].resumed_sample_ms){high=null;next++;}
                if(row.kind==='pulse'&&high===false)this.videoEdges.push(row.seconds);
                high=row.kind==='pulse';
            }
        }
        const sync=combineAudioEnvelopeSync(ordinalSync(this.videoEdges.filter(at=>at>=start&&at<end),
            envelope.accepted_edge_seconds.filter(at=>at<end),seconds),envelope);
        const audioResult={decoded_seconds:audio.length*.02,identified_seconds:identified.length*.02,
            maximum_gap_ms:maximumGap*1000,bad_identity_frames:audio.filter(row=>!row.identified).length,
            duplicate_frames:audio.filter(row=>row.duplicate).length,original_source_quantum_overlaps:sourceOverlaps,source_quantum_identity_available:sourceIdentified,source_quantum_identity_unbound_or_ambiguous:sourceUnbound,
            transition_coverage:audioCoverage,
            passed:sourceUnbound===0&&sourceOverlaps===0&&identified.length*.02>=59.9-audioCoverage.transition_missing_ms/1000
                &&audioCoverage.maximum_clean_gap_ms<=220&&identified.length>0&&identified[0].seconds-start<=.2
                &&audio.every(row=>row.identified&&!row.duplicate)};
        const decodedPerSecond=Array(60).fill(0);for(const row of video)decodedPerSecond[Math.floor(row.seconds-start)]++;
        const callbackTimes=[start,...video.map(row=>row.seconds),end];
        const videoResult={decoded_frames_per_second:decodedPerSecond,
            maximum_decode_callback_gap_ms:Math.max(...callbackTimes.slice(1).map((at,i)=>Math.max(0,at-callbackTimes[i])))*1000,
            decoded_frames:video.length,changing_frames_per_second:perSecond,minimum_fps:Math.min(...perSecond),
            identity,dimension_refusals:video.filter(row=>!row.dimensions).length,original_source_timestamp_overlaps:videoSourceOverlaps,duplicate_frames:video.filter(row=>row.duplicate).length,
            transition_coverage:videoCoverage,required_changing_frames_per_second:requiredFrames,
            passed:videoSourceOverlaps===0&&perSecond.every((count,index)=>count>=requiredFrames[index])
                &&changingVideo.length>=1440&&identity.status==='passed'&&video.every(row=>row.dimensions&&!row.duplicate)};
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
    const bindings=new Map(),owners=[],surfaces=new Set(),pendingRenders=new Set(),originalAudio=Renderer.prototype.audio,originalVideo=Renderer.prototype.video;
    let closed=false,closing,lastReport,measurementsRetired=false;const rtcStages=Rtc?browserMatrixRtcStages():null;
    const pipeline=Peer?browserMatrixPipeline(Peer,(connection,kind,stream)=>{
        const owner=bindings.get(connection);
        return owner&&routes[owner.index].active===connection&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;
    },(connection,kind,stream)=>{const owner=bindings.get(connection);
        return owner&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;}):null;
    function observe(renderer,binding,frame,kind,native){
        const owner=bindings.get(binding.connectionId);if(closed||!owner)return;
        if(Rtc && routes[owner.index].active!==binding.connectionId)return;
        if(!Rtc && !owner.initial && !owner.activation && !owner.readActivation)owner.readActivation=()=>renderer.nativeActivationBoundary;
        if(binding.streamId!==owner[kind])return;
        const route=routes[owner.index].measure,at=performance.now();
        try{
            if(kind==='audio'){
                check(frame.sampleRate===48000&&frame.numberOfChannels===1&&frame.numberOfFrames===960,'Decoded audio format');
                const samples=new Float32Array(960);frame.copyTo(samples,{planeIndex:0,format:'f32-planar'});
                route.audioFrame(binding.connectionId,frame.timestamp,at,samples,pipeline?.snapshot(binding.connectionId)??null,native);
            }else{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                route.videoFrame(binding.connectionId,frame.timestamp,at,gray,pixels,frame.displayWidth,frame.displayHeight,native);
            }
        }catch(error){if(route.invalid.length<16)route.invalid.push(String(error.message));}
    }
    const invalidRtc=(connection,message)=>{const owner=bindings.get(connection);if(!owner)return;
        const route=routes[owner.index].measure;if(route.invalid.length<16)route.invalid.push(message);};
    const rtc=Rtc?browserMatrixRtcObservation(Rtc,{
        audio(connection,timestamp,at,samples){
            const owner=bindings.get(connection);if(closed||!owner||routes[owner.index].active!==connection)return;
            try{routes[owner.index].measure.audioFrame(connection,timestamp,at,samples);}
            catch(error){invalidRtc(connection,String(error.message));}
        },
        video(connection,timestamp,at,frame,width,height){
            const owner=bindings.get(connection);if(closed||!owner||routes[owner.index].active!==connection)return;
            try{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                routes[owner.index].measure.videoFrame(connection,timestamp,at,gray,pixels,width,height);
            }catch(error){invalidRtc(connection,String(error.message));}
        },
        invalid:invalidRtc,
    }):null;
    Renderer.prototype.audio=function(binding,frame,native){observe(this,binding,frame,'audio',native);return originalAudio.call(this,binding,frame,native);};
    Renderer.prototype.video=function(binding,frame,native){observe(this,binding,frame,'video',native);return originalVideo.call(this,binding,frame,native);};
    function render(binding,connection,options){
        const work=(async()=>{
            const output=binding.surfaces[1];rtc?.attach(connection.id,output);
            const owner=await connection.render({audio:true,video:[{stream:connection.streams.byId(binding.video),canvas:output}]},options);
            owners.push({owner,index:binding.index,connectionId:connection.id,finished:owner.finished,retirement:owner.retirement,
                surfaces:binding.surfaces});
            binding.pendingConnection=undefined;
            if(closed)owner.close();
        })();
        pendingRenders.add(work);
        void work.then(()=>pendingRenders.delete(work),()=>pendingRenders.delete(work));
        return work;
    }
    return {
        async attach(index,connection,{audio,video,sendVideo,active=false},options){
            check(!closed&&!bindings.has(connection.id)&&bindings.size<10,'Bounded original render owners');
            const canvas=document.createElement('canvas');canvas.width=128;canvas.height=72;
            const context=canvas.getContext('2d',{willReadFrequently:true});context.imageSmoothingEnabled=false;
            surfaces.add(canvas);
            const output=document.createElement('canvas');surfaces.add(output);
            const binding={index,audio,video,sendVideo,context,initial:active,surfaces:[canvas,output],
                sourceId:Number(plan.routes[index].role.split('-').at(-1))-1};
            bindings.set(connection.id,binding);
            if(active)routes[index].active=connection.id;
            if(Rtc&&!active){
                // The SDK owns private decoder readiness. The fixture creates
                // public playback only after validating both original receipts.
                binding.pendingConnection=connection;
                return;
            }
            await render(binding,connection,options);
        },
        async initial(state,options){for(const route of plan.routes){const selected=routes[route.index].selected;
            await this.attach(route.index,state.connections[route.index],{audio:route.audio,video:Rtc?route.video:selected.video,sendVideo:route.video,active:true},options);}},
        start(origin){pipeline?.start(origin.originMs);rtcStages?.start(origin.originMs);for(const route of routes)route.measure.start(origin.originMs);},
        activate(index,connectionId,options){const owner=bindings.get(connectionId);check(!closed&&owner?.index===index,'Original prepared observer required');
            const recordCut=()=>{
                const measure=routes[index].measure;if(measure.begin===null)return;
                const cut={id:connectionId,atMs:performance.now()-measure.begin,
                    basis:Rtc?'validated-receipts-local-public-render-cut':'validated-native-public-cut-observed'};
                for(const route of routes){
                    if(plan.routes[route.measure.index].session!==plan.routes[index].session)continue;
                    check(route.measure.audioCuts.length<2,'Bounded original Session handoffs');
                    route.measure.audioCuts.push(cut);
                    if(route.measure.index===index||route.selected.index===index)route.measure.videoCuts.push(cut);
                }
            };
            if(!Rtc){const boundary=owner.readActivation?.();owner.readActivation=undefined;
                check(!owner.activation,'One original public native observer activation');
                check(boundary?.audience?.scope==='public'&&boundary.target_connection_id===connectionId,'Original completed public native cut required');owner.activation=boundary;
                recordCut();routes[index].active=connectionId;return;}
            check(owner.pendingConnection&&!owner.activating,'One original public RTC renderer activation');
            owner.activating=true;
            // This is the known local public-render cut, after signed receipt
            // validation. Select before startup so its first actual output is
            // observed even while render() is still resolving.
            recordCut();routes[index].active=connectionId;
            return render(owner,owner.pendingConnection,options).catch(error=>{
                if(routes[index].measure.invalid.length<16)routes[index].measure.invalid.push('Original public RTC render activation failed');
                throw error;
            });},
        report(){if(measurementsRetired)return lastReport;const measured=routes.map(route=>route.measure.report((row,kind)=>{
                if(Rtc)return true;const owner=bindings.get(row.owner);
                try{check(owner,'Original native output owner missing');check(row.native,'Original native decoded datagram provenance missing');
                    if(owner.initial)return true;
                    check(owner.activation?.audience?.scope==='public','Prepared owner has no completed public cut');
                    const cut=owner.activation.audience.streams.find(c=>c.stream_id===owner[kind]);check(cut,'Original native stream cut missing');
                    return nativeMembership(cut,row.native)==='public';
                }catch(error){if(route.measure.invalid.length<16)route.measure.invalid.push(String(error.message));return false;}
            }));return lastReport=structuredClone({format:Rtc?'connect/browser-matrix-decoded-media/1':'connect/browser-matrix-decoded-media/2',
            native_activation: Rtc?null:[...bindings].filter(([,owner])=>!owner.initial).map(([connection,owner])=>({connection,boundary:owner.activation??null})),
            method:Rtc?'Independent content measurements on actual public WebRTC render-graph PCM and video pixels, started after original completion receipt validation at a known local renderer cut; no server RTP-cut or physical playback claim':
                'Independent content measurements on actual WebCodecs decoded PCM/pixels at local receive callback times; no physical playback claim',
            window_ms:60000,source_counters_used:false,rtc_media_pipeline:rtcStages?.report()??null,
            pipeline:pipeline?.report(routes.map((route,index)=>({index,source:route.selected.index,
                audio:route.measure.envelope.edges,audioEnvelope:route.measure.envelope.report(1),video:route.measure.videoEdges}))),passed:measured.every(route=>route.passed),routes:measured});},
        close(){return closing??=(async()=>{closed=true;pipeline?.close();Renderer.prototype.audio=originalAudio;Renderer.prototype.video=originalVideo;
            // Stop scheduling diagnostics at original close, before joining
            // render owners. A pending original getStats is never a fake join.
            const statisticsWork=rtcStages?.close();
            await Promise.allSettled([...pendingRenders]);
            for(const {owner} of owners)owner.close();
            const finishedWork=Promise.allSettled(owners.map(entry=>entry.finished));
            // Retain each exact original promise at attach. A replacement
            // property or another owner's same-ID receipt proves no cleanup.
            const retirements=await Promise.allSettled(owners.map(async entry=>{
                check(entry.retirement&&typeof entry.retirement.then==='function','Original renderer retirement promise required');
                const receipt=await entry.retirement;
                check(receipt&&Object.isFrozen(receipt)&&receipt.connectionId===entry.connectionId
                    &&receipt.scope==='renderer-local'&&receipt.outputsDetached===true
                    &&['closed','not-created'].includes(receipt.audioGraph),'Original renderer retirement receipt differs');
                return receipt;
            }));
            await rtc?.close();const statistics=await statisticsWork;
            const diagnosticOwnersJoined=!rtcStages||statistics?.joined===true;
            if(diagnosticOwnersJoined)retirements.forEach((result,index)=>{
                if(result.status!=='fulfilled')return;
                for(const canvas of owners[index].surfaces){canvas.width=0;canvas.height=0;surfaces.delete(canvas);}
            });
            // These prepared bindings never created a renderer or a borrower.
            // They cannot supply a fabricated renderer-retirement receipt.
            for(const binding of bindings.values())if(binding.pendingConnection&&!binding.activating){
                binding.pendingConnection=undefined;
                for(const canvas of binding.surfaces){canvas.width=0;canvas.height=0;surfaces.delete(canvas);}
            }
            const outcomes=await finishedWork,bounded=value=>typeof value==='string'?value.slice(0,64):null;
            const details=(outcome,ordinal,stage)=>({owner_index:ordinal,route_index:owners[ordinal].index,
                connection_id:owners[ordinal].connectionId,status:outcome.status,stage,
                ...(outcome.status==='rejected'?{name:bounded(outcome.reason?.name),code:bounded(outcome.reason?.code),kind:bounded(outcome.reason?.kind),
                    certainty:bounded(outcome.reason?.certainty)}:{})});
            const ownerOutcomes=outcomes.map((outcome,index)=>details(outcome,index,'original-render-finished'));
            const retirementOutcomes=retirements.map((outcome,index)=>details(outcome,index,'original-render-retirement'));
            const count=owners.length,workingState=()=>{
                const total={};for(const route of routes)for(const [key,value] of Object.entries(route.measure.workingState()))
                    total[key]=(total[key]??0)+value;
                return total;
            };
            const before=workingState();
            const playbackPassed=outcomes.every(value=>value.status==='fulfilled');
            const joined=retirements.every(value=>value.status==='fulfilled')&&diagnosticOwnersJoined&&surfaces.size===0;
            if(joined){
                // Preserve the most recent complete proof before dropping only
                // no-longer-needed numeric measurement working histories.
                lastReport=this.report();
                for(const route of routes)route.measure.retireWorkingState();
                measurementsRetired=true;
            }
            owners.length=0;bindings.clear();
            return {owners:count,statistics,joined,playback_passed:playbackPassed,
                measurement_working_state:{retired:joined,before,retained:workingState(),
                    scope:'Decoded-route working histories only. Captured report evidence remains unchanged. No garbage collection or RSS return claim.'},
                owner_outcomes:ownerOutcomes,retirement_outcomes:retirementOutcomes,retained_surfaces:surfaces.size};
        })();},
    };
}
