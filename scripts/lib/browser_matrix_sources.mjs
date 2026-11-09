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

// Keep the independently sampled 8x8 identity/pulse centres untouched. The
// bottom macroblock starts below their final y=337.5 centre; its eight visible
// rows remain inside the observer's actual 128x72 change sample.
function paintMatrixVideoFrame(paint, {frame, pulse}, role) {
    paint.fillStyle = `hsl(${frame % 360} 90% 40%)`;
    paint.fillRect(0, 0, 640, 360);
    paint.fillStyle = 'white';
    paint.fillRect((frame * 7) % 580, 70, 60, 150);
    markerCells(role).forEach((value, index) => {
        paint.fillStyle = `rgb(${value},${value},${value})`;
        paint.fillRect(index % 2 * 80, Math.floor(index / 2) * 45, 80, 45);
    });
    if (pulse) {
        paint.fillStyle = 'white';
        paint.fillRect(0, 0, 640, 360);
    }
    // Actual source-frame bits change even while the cadence pulse is white.
    // They grant no identity or frame credit without real decoded pixels.
    for (let bit = 0; bit < 11; bit++) {
        paint.fillStyle = frame & (1 << bit) ? 'white' : 'black';
        paint.fillRect(bit * 32, 352, 32, 8);
    }
}

/** Caller-owned graph/canvas sources survive per-Connection capture retirement.
 * The SDK creates and retires each derived track. The original six sources and
 * fixed clock stay here; generated input never qualifies decoded quality. */
export async function browserMatrixSources() {
    let audio=new AudioContext({sampleRate:48000});
    try{await audio.resume();if(audio.state!=='running')throw Error('Owned real-time audio output did not start');}
    catch(error){await audio.close();throw error;}
    const sources=[];
    const bindings=new Map(),starts=new Set();
    let retiredSnapshot,referenceRetirement;
    let stopped=false,closing,programme,terminalTimer,terminal=false,cleanupFailures=0;
    const retireConnection=connectionId=>{
        const binding=bindings.get(connectionId);if(!binding)return;
        bindings.delete(connectionId);binding.retired=true;
        try{binding.capture?.close();}catch{cleanupFailures++;}
    };
    const stopSources=()=>{
        if(terminal)return;terminal=true;clearTimeout(terminalTimer);
        for(const connectionId of [...bindings.keys()])retireConnection(connectionId);
        for(const source of sources){
            clearInterval(source.timer);source.timer=undefined;
            source.retired=true;
            source.oscillator.stop();source.oscillator.disconnect();source.gain.disconnect();
        }
    };
    const owner={sources,
        start(input) {
            const work=(async({route,connection,options})=>{
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
                const paint=canvas.getContext('2d'),oscillator=audio.createOscillator(),gain=audio.createGain();
                if(!paint)throw Error('Original canvas source context unavailable');
                oscillator.frequency.value=440+220*role;gain.gain.value=matrixAudioProfile.lowPeak;
                oscillator.connect(gain);oscillator.start();
                record={index:route.index,sourceId:role,draws:0,oscillator,gain,canvas,
                    capture:undefined,timer:undefined,retired:false,oscillatorEnded:false,
                    canvasCaptureMode:'sdk-owned-native-canvas-per-connection',
                    attachments:[],programmeDraws:0,lastProgrammeFrame:null,skippedProgrammeFrames:0,
                    maximumDrawGapMs:0,lastDrawMs:null,pulseOrdinals:[],lastPulseOrdinal:null};
                oscillator.onended=()=>{record.oscillatorEnded=true;};
                record.ownerLive=()=>!stopped&&!terminal&&!record.retired&&!record.oscillatorEnded
                    &&audio.state==='running'&&record.timer!==undefined
                    &&record.canvas.width===640&&record.canvas.height===360;
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
                    paintMatrixVideoFrame(paint, coordinate, role);
                    if(pulse&&record.lastPulseOrdinal!==ordinal){record.lastPulseOrdinal=ordinal;record.pulseOrdinals.push(ordinal);}
                    record.draws++;
                };
                record.draw=draw;draw();record.timer=setInterval(draw,1000/30);
            }
            const binding={connectionId:connection.id,capture:undefined,retired:false};
            bindings.set(connection.id,binding);record.attachments.push(binding);
            try{
                binding.capture=await connection.capture({audio:true,audioNode:record.gain,video:'camera',videoCanvas:record.canvas},options);
                if(stopped||terminal||bindings.get(connection.id)!==binding){
                    binding.capture.close();
                    throw Error('Source retired during capture admission');
                }
                record.capture=binding.capture;
            }catch(error){retireConnection(connection.id);throw error;}
            })(input);
            starts.add(work);void work.then(()=>starts.delete(work),()=>starts.delete(work));return work;
        },
        retireConnection,
        beginProgramme() {
            if(stopped||terminal||programme||audio.state!=='running'||sources.length!==6||sources.some(source=>
                !source.ownerLive()||!source.attachments.some(binding=>!binding.retired&&binding.capture&&!binding.capture.closed)))
                throw Error('One running original programme requires six actual original captures');
            // Capture these coordinates once; never rebase after movement or
            // stalled audio. The monotonic terminal remains fixed either way.
            const audioOriginSeconds=audio.currentTime+.1,originMs=performance.now()+100;
            programme=Object.freeze({audioOriginSeconds,originMs,deadlineMs:originMs+durationMs,durationMs,
                markerProgram,sourcePulseMs:100,audioCaptureMode:'caller-owned-audio-node',videoCaptureMode:'caller-owned-canvas',audioPeakLow:matrixAudioProfile.lowPeak,audioPeakHigh:matrixAudioProfile.highPeak});
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
            const stopAtOriginalDeadline=()=>{
                if(stopped||terminal)return;
                const remaining=programme.deadlineMs-performance.now();
                if(remaining>0){terminalTimer=setTimeout(stopAtOriginalDeadline,remaining);return;}
                stopSources();
            };
            terminalTimer=setTimeout(stopAtOriginalDeadline,Math.max(0,programme.deadlineMs-performance.now()));
            return programme;
        },
        close() {
            return closing??=(async()=>{
                stopped=true;stopSources();
                await audio.close();
                if(cleanupFailures)throw Error('Source capture cleanup failed');
                // These are the caller's original canvas surfaces. Closed
                // captures no longer use them; release their backing stores
                // before observing idle resources, without resizing live input.
                for(const source of sources){source.canvas.width=0;source.canvas.height=0;}
            })();
        },
        retireReferences(){return referenceRetirement??=(async()=>{
            await owner.close();await Promise.allSettled([...starts]);
            if(starts.size||bindings.size||sources.some(source=>source.attachments.some(binding=>
                !binding.retired||(binding.capture&&binding.capture.closed!==true))))
                throw Error('Original source capture ownership still pending');
            const snapshot=owner.observation();
            const freeze=value=>{if(value&&typeof value==='object'){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
            retiredSnapshot=freeze(snapshot);
            sources.length=0;audio=undefined;
            return Object.freeze({joined:true,released:true,source_snapshot:retiredSnapshot});
        })();},
        observation:()=>retiredSnapshot??({capture_input_mode:'caller-owned-audio-node+caller-owned-canvas',video_track_ownership:'sdk-per-connection',audio_context_ownership:'original-source-owner',real_time_audio_state:audio.state,programme:programme??null,source_terminal:terminal,cleanup_failures:cleanupFailures,
            source_audio_elapsed_ms:programme?(audio.currentTime-programme.audioOriginSeconds)*1000:null,
            sources:sources.map(source=>({index:source.index,source_id:source.sourceId,draws:source.draws,
                canvas_capture_mode:source.canvasCaptureMode,
                capture_started:Boolean(source.capture),programme_draws:source.programmeDraws,
                skipped_programme_frames:source.skippedProgrammeFrames,maximum_draw_gap_ms:source.maximumDrawGapMs,
                pulse_ordinals:[...source.pulseOrdinals],last_programme_frame:source.lastProgrammeFrame,
                original_source_owner_live:source.ownerLive(),original_source_timer_active:source.timer!==undefined,
                original_source_retired:source.retired,original_oscillator_ended:source.oscillatorEnded,
                original_canvas_backing_released:source.canvas.width===0&&source.canvas.height===0,
                attachments:source.attachments.map(binding=>({connection_id:binding.connectionId,retired:binding.retired,
                    capture_started:Boolean(binding.capture)}))})),
            scope:'Generated source and capture ownership only. No decoded receive, movement or profile-quality pass.'}),
    };
    return owner;
}
