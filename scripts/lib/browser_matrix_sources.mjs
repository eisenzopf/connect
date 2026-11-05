import {markerFrames,markerProgram} from '../video/marker_calibration.mjs';
import {markerCells} from '../video/source_identity.mjs';
const durationMs=62000;
// A14x pulse amplitude step can make an otherwise clean20ms tone block
// spectrally ambiguous. Keep every block identifiable while preserving the
// fixed100ms pulse schedule and unchanged decoded RMS/identity thresholds.
export const matrixAudioProfile=Object.freeze({lowPeak:.05,highPeak:.2});
const pulses=Object.freeze(markerFrames().filter(frame=>frame<1860));
// Source coordinates only: late callbacks draw the current frame once, never
// catch-up frames or fabricated observer samples.
export function matrixSourceFrame(elapsedMs) {
    if(!Number.isFinite(elapsedMs))throw Error('Finite original source clock required');
    if(elapsedMs<0||elapsedMs>=durationMs)return null;
    const frame=Math.floor(elapsedMs*30/1000);
    let low=0,high=pulses.length;
    while(low<high){const mid=Math.floor((low+high)/2);if(pulses[mid]<=frame)low=mid+1;else high=mid;}
    const ordinal=low-1;
    return {frame,pulse:ordinal>=0&&frame<pulses[ordinal]+3,ordinal:ordinal>=0?ordinal:null};
}

function originalCanvasCapture(canvas) {
    const requested=canvas.captureStream(0),video=requested.getVideoTracks()[0];
    if(typeof video?.requestFrame==='function')return {video,
        requestFrame:()=>video.requestFrame(),mode:'manual-frame-request'};
    // A zero-rate track cannot deliver later draws without requestFrame().
    // Retire that probe before creating the bounded automatic capture. The
    // source clock still draws only the current 30fps coordinate once.
    requested.getTracks().forEach(track=>track.stop());
    const automatic=canvas.captureStream(30),automaticVideo=automatic.getVideoTracks()[0];
    if(!automaticVideo||automaticVideo.readyState!=='live'){
        automatic.getTracks().forEach(track=>track.stop());
        throw Error('Owned live canvas video track unavailable');
    }
    return {video:automaticVideo,requestFrame:()=>{},mode:'automatic-canvas-capture-30fps'};
}

/** Original source masters survive capture retirement during device roundtrips.
 * Each Connection owns track clones; the six masters and fixed clock stay here.
 * Generated input observations never qualify decoded receive quality. */
export async function browserMatrixSources() {
    const audio=new AudioContext({sampleRate:48000});
    try{await audio.resume();if(audio.state!=='running')throw Error('Owned real-time audio output did not start');}
    catch(error){await audio.close();throw error;}
    const sources=[];
    const bindings=new Map();
    let stopped=false,closing,programme,terminalTimer,terminal=false,cleanupFailures=0;
    const retireConnection=connectionId=>{
        const binding=bindings.get(connectionId);if(!binding)return;
        bindings.delete(connectionId);binding.retired=true;
        try{binding.capture?.close();}catch{cleanupFailures++;}
        finally{binding.stream.getTracks().forEach(track=>track.stop());}
    };
    const stopSources=()=>{
        if(terminal)return;terminal=true;clearTimeout(terminalTimer);
        for(const connectionId of [...bindings.keys()])retireConnection(connectionId);
        for(const source of sources){
            clearInterval(source.timer);source.stream.getTracks().forEach(track=>track.stop());
            source.oscillator.stop();source.oscillator.disconnect();source.gain.disconnect();
        }
    };
    return {sources,
        async start({route,connection,options}) {
            if(stopped||terminal)throw Error('Original media source owner retired');
            if(!Number.isInteger(route.index)||route.index<0||route.index>=6||typeof connection.id!=='string'||bindings.has(connection.id))
                throw Error('Six original routes and distinct physical captures required');
            const role=Number(route.role.split('-').at(-1))-1;
            markerCells(role);
            let record=sources.find(source=>source.index===route.index);
            if(record&&record.sourceId!==role)throw Error('Replacement must retain original source identity');
            if(record&&record.attachments.length>=3)throw Error('Original source permits one switch and one return');
            if(!record){
                if(programme)throw Error('All original sources must exist before the programme');
                const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
                const canvasCapture=originalCanvasCapture(canvas);
                const paint=canvas.getContext('2d'),oscillator=audio.createOscillator(),gain=audio.createGain();
                const destination=audio.createMediaStreamDestination();destination.channelCount=1;
                oscillator.frequency.value=440+220*role;gain.gain.value=matrixAudioProfile.lowPeak;
                oscillator.connect(gain);gain.connect(destination);oscillator.start();
                record={index:route.index,sourceId:role,draws:0,oscillator,gain,destination,canvas,
                    stream:new MediaStream([...destination.stream.getAudioTracks(),canvasCapture.video]),capture:undefined,timer:undefined,
                    canvasCaptureMode:canvasCapture.mode,
                    attachments:[],programmeDraws:0,lastProgrammeFrame:null,skippedProgrammeFrames:0,
                    maximumDrawGapMs:0,lastDrawMs:null,pulseOrdinals:[],lastPulseOrdinal:null};
                sources.push(record);
                const draw=()=>{
                    if(stopped||terminal)return;
                    const now=performance.now();
                    if(programme&&now>=programme.deadlineMs){stopSources();return;}
                    const elapsed=programme?(audio.currentTime-programme.audioOriginSeconds)*1000:null;
                    const coordinate=elapsed===null||elapsed<0?{frame:record.draws,pulse:false,ordinal:null}:matrixSourceFrame(elapsed);
                    if(!coordinate)return;
                    if(elapsed!==null&&elapsed>=0){
                        if(coordinate.frame===record.lastProgrammeFrame)return;
                        record.skippedProgrammeFrames+=record.lastProgrammeFrame===null?coordinate.frame:
                            Math.max(0,coordinate.frame-record.lastProgrammeFrame-1);
                        record.lastProgrammeFrame=coordinate.frame;record.programmeDraws++;
                    }
                    const {frame,pulse,ordinal}=coordinate;
                    record.maximumDrawGapMs=Math.max(record.maximumDrawGapMs,record.lastDrawMs===null?0:now-record.lastDrawMs);
                    record.lastDrawMs=now;
                    paint.fillStyle=`hsl(${frame%360} 90% 40%)`;paint.fillRect(0,0,640,360);
                    paint.fillStyle='white';paint.fillRect((frame*7)%580,70,60,150);
                    markerCells(role).forEach((value,index)=>{
                        paint.fillStyle=`rgb(${value},${value},${value})`;paint.fillRect(index%2*80,Math.floor(index/2)*45,80,45);
                    });
                    if(pulse){paint.fillStyle='white';paint.fillRect(0,0,640,360);}
                    if(pulse&&record.lastPulseOrdinal!==ordinal){record.lastPulseOrdinal=ordinal;record.pulseOrdinals.push(ordinal);}
                    record.draws++;canvasCapture.requestFrame();
                };
                record.draw=draw;draw();record.timer=setInterval(draw,1000/30);
            }
            const stream=new MediaStream(record.stream.getTracks().map(track=>track.clone()));
            const binding={connectionId:connection.id,stream,capture:undefined,retired:false};
            bindings.set(connection.id,binding);record.attachments.push(binding);
            try{
                binding.capture=await connection.capture({audio:true,video:'camera',stream},options);
                if(stopped||terminal||bindings.get(connection.id)!==binding){
                    binding.capture.close();stream.getTracks().forEach(track=>track.stop());
                    throw Error('Source retired during capture admission');
                }
                record.capture=binding.capture;
            }catch(error){retireConnection(connection.id);throw error;}
        },
        retireConnection,
        beginProgramme() {
            if(stopped||terminal||programme||audio.state!=='running'||sources.length!==6||sources.some(source=>
                !source.attachments.some(binding=>!binding.retired&&binding.capture&&!binding.capture.closed)))
                throw Error('One running original programme requires six actual original captures');
            // Capture these coordinates once; never rebase after movement or
            // stalled audio. The monotonic terminal remains fixed either way.
            const audioOriginSeconds=audio.currentTime+.1,originMs=performance.now()+100;
            programme=Object.freeze({audioOriginSeconds,originMs,deadlineMs:originMs+durationMs,durationMs,
                markerProgram,sourcePulseMs:100,audioPeakLow:matrixAudioProfile.lowPeak,audioPeakHigh:matrixAudioProfile.highPeak});
            for(const source of sources){
                // AudioContext advances in quanta. Poll the unchanged 30fps
                // source coordinate frequently enough to observe each frame;
                // draw still emits only the current frame once, never catch-up.
                clearInterval(source.timer);source.timer=setInterval(source.draw,5);
                source.gain.gain.setValueAtTime(matrixAudioProfile.lowPeak,audioOriginSeconds);
                for(const frame of pulses){
                    source.gain.gain.setValueAtTime(matrixAudioProfile.highPeak,audioOriginSeconds+frame/30);
                    source.gain.gain.setValueAtTime(matrixAudioProfile.lowPeak,audioOriginSeconds+frame/30+.1);
                }
            }
            terminalTimer=setTimeout(stopSources,Math.max(0,programme.deadlineMs-performance.now()));
            return programme;
        },
        close() {
            return closing??=(async()=>{
                stopped=true;stopSources();
                await audio.close();
                if(cleanupFailures)throw Error('Source capture cleanup failed');
            })();
        },
        observation:()=>({real_time_audio_state:audio.state,programme:programme??null,source_terminal:terminal,cleanup_failures:cleanupFailures,
            source_audio_elapsed_ms:programme?(audio.currentTime-programme.audioOriginSeconds)*1000:null,
            sources:sources.map(source=>({index:source.index,source_id:source.sourceId,draws:source.draws,
                canvas_capture_mode:source.canvasCaptureMode,
                capture_started:Boolean(source.capture),programme_draws:source.programmeDraws,
                skipped_programme_frames:source.skippedProgrammeFrames,maximum_draw_gap_ms:source.maximumDrawGapMs,
                pulse_ordinals:[...source.pulseOrdinals],last_programme_frame:source.lastProgrammeFrame,
                original_master_tracks_live:source.stream.getTracks().every(track=>track.readyState==='live'),
                attachments:source.attachments.map(binding=>({connection_id:binding.connectionId,retired:binding.retired,
                    capture_started:Boolean(binding.capture)}))})),
            scope:'Generated source and capture ownership only. No decoded receive, movement or profile-quality pass.'}),
    };
}
