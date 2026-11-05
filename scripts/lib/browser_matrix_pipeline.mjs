import {MatrixMediaStages} from './browser_matrix_stages.mjs';
import {pcmIdentity} from './browser_matrix_pcm.mjs';
import {AudioEnvelope} from '../video/audio_envelope.mjs';
import {calibrate} from '../video/marker_calibration.mjs';
import {classifySourceFrame} from '../video/source_identity.mjs';
import {MatrixAudioInput} from './browser_matrix_audio_input.mjs';
// Match content ordinals, never nearest arrivals or expected timing offsets.
export function pipelineLatency(sent,received){
    const a=calibrate(sent),b=calibrate(received),result={status:'missing-proof',paired:0,signed_median_ms:null,p95_ms:null};
    if(a.status!=='calibrated'||b.status!=='calibrated')return result;
    const source=new Map(a.ordinals.map((ordinal,i)=>[ordinal,sent[i]]));
    const values=b.ordinals.flatMap((ordinal,i)=>source.has(ordinal)?[(received[i]-source.get(ordinal))*1000]:[]).sort((x,y)=>x-y);
    if(!values.length)return result;
    return {...result,status:values.length>=42?'measured':'insufficient-pairs',paired:values.length,
        signed_median_ms:(values[Math.floor((values.length-1)/2)]+values[Math.floor(values.length/2)])/2,
        p95_ms:values[Math.ceil(values.length*.95)-1],minimum_ms:values[0],maximum_ms:values.at(-1)};
}
// Numeric marker matches remain diagnostic when either envelope is invalid.
export function pipelineEnvelopeLatency(sent,received,...envelopes){
    const result=pipelineLatency(sent,received);
    return envelopes.every(envelope=>envelope?.status==='observer-valid')?result:
        {...result,status:'observer-invalid',reason:'Audio envelope validity is required for delay attribution'};
}
/** Passive fixture diagnostics at the real SDK encoder input. No clock/PCM/
 * pixel alteration, retries or new media. These counters cannot qualify quality. */
export function browserMatrixPipeline(Peer,lookup,lookupAll=null){
    const audio=Peer.prototype.encodeAudio,video=Peer.prototype.encodeVideo;
    const routes=Array.from({length:6},()=>({audio:new AudioEnvelope(),input:new MatrixAudioInput(),videoEdges:[],high:false,
        audioFrames:0,videoFrames:0,audioRefused:0,videoRefused:0,maxPeerDroppedPackets:0,errors:[]}));
    let origin=null,closed=false;const stages=new MatrixMediaStages();
    const canvas=document.createElement('canvas');canvas.width=8;canvas.height=8;
    const context=canvas.getContext('2d',{willReadFrequently:true});context.imageSmoothingEnabled=false;
    function target(connection,kind,stream){const owner=lookup(connection,kind,stream);
        if(closed||origin===null||!owner)return null;const at=(performance.now()-origin)/1000;
        return at>=0&&at<62?{route:routes[owner.index],at}:null;}
    function diagnostic(connection,kind,stream){if(!lookupAll)return null;const owner=lookupAll(connection,kind,stream);if(closed||origin===null||!owner)return null;
        const at=(performance.now()-origin)/1000;return at>=0&&at<62?{owner,at}:null;}
    const failed=(route,error)=>{if(route.errors.length<16)route.errors.push(String(error.message));};
    Peer.prototype.encodeAudio=function(connection,stream,samples,timestamp){
        const observed=target(connection,'audio',stream),stage=diagnostic(connection,'audio',stream),accepted=audio.call(this,connection,stream,samples,timestamp);
        if(stage)try{stages.audio(connection,stage.owner.index,timestamp,stage.at,samples,
            pcmIdentity(samples,stage.owner.sourceId),accepted,this);}catch(error){if(observed)failed(observed.route,error);}
        if(observed){const {route,at}=observed;route.audioFrames++;if(!accepted)route.audioRefused++;
            try{let energy=0;for(const value of samples)energy+=value*value;
                const rms=Math.sqrt(energy/samples.length);
                route.audio.sample(at,rms);
                route.input.sample(connection,timestamp,at,rms);
                route.maxPeerDroppedPackets=Math.max(route.maxPeerDroppedPackets,this.diagnostics.droppedPackets);
            }catch(error){failed(route,error);}}
        return accepted;
    };
    Peer.prototype.encodeVideo=function(connection,stream,frame){
        const observed=target(connection,'video',stream),stage=diagnostic(connection,'video',stream);let pulse=false;
        if(observed)try{
            context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
            pulse=classifySourceFrame(Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]))==='pulse';
        }catch(error){failed(observed.route,error);}
        const accepted=video.call(this,connection,stream,frame);
        if(stage)try{stages.video(connection,stage.owner.index,stage.at,accepted,this);}catch(error){if(observed)failed(observed.route,error);}
        if(observed){const {route,at}=observed;route.videoFrames++;if(!accepted)route.videoRefused++;
            if(pulse&&!route.high){if(route.videoEdges.length<512)route.videoEdges.push(at);else failed(route,Error('marker bound'));}route.high=pulse;}
        return accepted;
    };
    return {start(value){if(origin!==null)throw Error('One original pipeline clock');origin=value;},
        report(receivers){const edges=values=>values.filter(at=>at>=1&&at<61);
            return {format:'connect/browser-capture-pipeline/1',scope:'Original encoder-input content versus actual decoded output; diagnostic only, no transport pass',
                stages:stages.report(),
                sources:routes.map((route,index)=>({index,audio_frames:route.audioFrames,video_frames:route.videoFrames,
                    audio_encoder_refusals:route.audioRefused,video_encoder_refusals:route.videoRefused,
                    maximum_peer_dropped_packets:route.maxPeerDroppedPackets,errors:route.errors,
                    audio_envelope:route.audio.report(1),original_audio_input:route.input.report(),video_edges:edges(route.videoEdges),
                    original_capture_audio_minus_video:pipelineEnvelopeLatency(edges(route.videoEdges),edges(route.input.edges),route.input.report()),
                    capture_audio_minus_video:pipelineEnvelopeLatency(edges(route.videoEdges),edges(route.audio.edges),route.audio.report(1))})),
                receive_paths:receivers.map(({index,source,audio,video,audioEnvelope})=>({index,source,
                    audio_input_to_decode:pipelineEnvelopeLatency(edges(routes[source].audio.edges),edges(audio),routes[source].audio.report(1),audioEnvelope),
                    original_audio_input_to_decode:pipelineEnvelopeLatency(edges(routes[source].input.edges),edges(audio),routes[source].input.report(),audioEnvelope),
                    video_input_to_decode:pipelineLatency(edges(routes[source].videoEdges),edges(video))}))};},
        snapshot(connection){return stages.owners.get(connection)?.snapshots.at(-1)??null;},
        close(){if(closed)return;closed=true;Peer.prototype.encodeAudio=audio;Peer.prototype.encodeVideo=video;}};
}
