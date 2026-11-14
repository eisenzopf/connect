const RTC_DECODED_VIDEO_PROFILE='native-decoded-frames/1';
import {rtcAudioBoundaryDiagnostics} from './browser_matrix_rtc_audio_diagnostics.mjs';
// A passive tap on the real RTC render graph. The processor only copies input
// PCM; it never generates a tone, repairs a gap or infers content from counters.
export const rtcDecodedAudioWorklet = `
class MatrixRtcDecodedPcm extends AudioWorkletProcessor {
    constructor() {
        super();this.frame=new Float32Array(960);this.used=0;this.start=0;this.next=null;this.count=0;this.stopped=false;
        this.port.onmessage=event=>{if(event.data?.type==='stop'){this.stopped=true;this.port.postMessage({type:'stopped'});}};
    }
    process(inputs) {
        if(this.stopped)return false;
        const channels=inputs[0];
        if(!channels?.length){this.used=0;this.next=null;return true;}
        if(sampleRate!==48000||channels.length!==1){this.port.postMessage({type:'error',reason:'Decoded RTC PCM format'});return false;}
        const samples=channels[0];
        if(this.next!==null&&this.next!==currentFrame)this.used=0;
        this.next=currentFrame+samples.length;
        for(let i=0;i<samples.length;i++){
            if(!Number.isFinite(samples[i])){this.port.postMessage({type:'error',reason:'Nonfinite decoded RTC PCM'});return false;}
            if(this.used===0)this.start=currentFrame+i;
            this.frame[this.used++]=samples[i];
            if(this.used===960){
                if(++this.count>16000){this.port.postMessage({type:'error',reason:'Decoded RTC PCM bound'});return false;}
                this.port.postMessage({type:'pcm',samples:this.frame,timestamp:Math.round(this.start*1000000/48000)},[this.frame.buffer]);
                this.frame=new Float32Array(960);this.used=0;
            }
        }
        return true;
    }
}
registerProcessor('matrix-rtc-decoded-pcm',MatrixRtcDecodedPcm);
`;

/** Capture the SDK render owner's actual decoded PCM and painted video. An
 * internal preparation/readiness sink is excluded: it is not inside render().
 * The caller still owns render handles and closes/joins their AudioContexts. */
export function browserMatrixRtcObservation(Rtc, {audio, video, invalid, measurementOrigin=()=>null,
    videoObserver='render-callback/1',decodedVideo,decodedVideoOwnerFactory=null}) {
    if(!['render-callback/1',RTC_DECODED_VIDEO_PROFILE].includes(videoObserver))throw Error('Declared RTC video observer required');
    if(videoObserver===RTC_DECODED_VIDEO_PROFILE&&(typeof decodedVideo!=='function'||typeof decodedVideoOwnerFactory!=='function'))throw Error('Original decoded-pixel callback and owner required');
    // Shared callback-only fixtures have no new static module dependency. The
    // matrix's explicit Chrome entry injects the closed owned-reader factory.
    const decoded=videoObserver===RTC_DECODED_VIDEO_PROFILE?decodedVideoOwnerFactory({observe:decodedVideo,invalid}):null;
    // Root may opt into bounded private PCM evidence before this original
    // observer starts. The SDK/browser media profile and graph stay unchanged.
    const audioBoundaries=rtcAudioBoundaryDiagnostics({retainSuspectPcm:globalThis.connectRtcAudioFacts===true,
        contextCloseProbeAvailable:typeof AudioContext==='function'&&typeof AudioContext.prototype?.close==='function'});
    const render=Rtc.prototype.render,capture=Rtc.prototype.capture;
    const sourcePrototype=MediaStreamAudioSourceNode.prototype;
    const sourceConnect=sourcePrototype.connect;
    const sourceConnectDescriptor=Object.getOwnPropertyDescriptor(sourcePrototype,'connect');
    const sourceDisconnect=sourcePrototype.disconnect;
    const sourceDisconnectDescriptor=Object.getOwnPropertyDescriptor(sourcePrototype,'disconnect');
    const contextPrototype=typeof AudioContext==='function'?AudioContext.prototype:null,contextClose=contextPrototype?.close;
    const contextCloseDescriptor=contextPrototype?Object.getOwnPropertyDescriptor(contextPrototype,'close'):undefined;
    const draw=CanvasRenderingContext2D.prototype.drawImage,frame=HTMLVideoElement.prototype.requestVideoFrameCallback;
    const canvases=new Map(),metadata=new WeakMap(),pending=new Set(),taps=new Set(),modules=new WeakMap(),observed=new WeakSet();
    const presentations = new WeakMap();
    let nextPresentationElement = 0;
    let nextAudioClock = 0;
    let rendering=null,closed=false,lifetimeFailures=0;
    const worklet=URL.createObjectURL(new Blob([rtcDecodedAudioWorklet],{type:'text/javascript'}));
    const lifetime=(owned,kind,fields={})=>{
        if(closed||!taps.has(owned))return;
        try{audioBoundaries.lifecycle({owner:owned.clock.owner,clock:owned.clock,kind,
            at:performance.now(),context:owned.context,fields});}catch{lifetimeFailures++;}
    };
    const settlement=(epoch,kind)=>{
        for(const owned of taps)if(owned.clock.epoch_id===epoch)lifetime(owned,kind);
    };
    // These passive continuations never close, await, or own the original work.
    const observeSettlement=(original,epoch,resolved,rejected)=>{
        void Promise.resolve(original).then(()=>settlement(epoch,resolved),()=>settlement(epoch,rejected));
    };
    const observeRender=(result,epochs)=>{
        void Promise.resolve(result).then(owner=>{
            if(closed)return;
            for(const owned of taps)if(epochs.includes(owned.clock.epoch_id)){
                if(owner?.finished&&typeof owner.finished.then==='function'){
                    lifetime(owned,'render-finished-observed');
                    observeSettlement(owner.finished,owned.clock.epoch_id,'render-finished-resolved','render-finished-rejected');
                }
            }
        },()=>{});
    };
    function tap(context,source,connectionId) {
        if (nextAudioClock >= 10) {
            invalid(connectionId, 'Bounded original RTC decoded PCM clocks');
            return;
        }
        const clock = Object.freeze({profile:'connect/rtc-decoded-pcm-clock/1',
            owner:connectionId, epoch_id:++nextAudioClock});
        const owned={context,source,node:null,gain:null,clock,onState:null};taps.add(owned);
        if(typeof context.addEventListener==='function'&&typeof context.removeEventListener==='function'){
            const onState=()=>lifetime(owned,'context-statechange');
            try{context.addEventListener('statechange',onState);owned.onState=onState;}catch{lifetimeFailures++;}
        }
        lifetime(owned,'tap-tracked',{state_listener_installed:owned.onState!==null});
        let loaded=modules.get(context);
        if(!loaded){loaded=context.audioWorklet.addModule(worklet);modules.set(context,loaded);}
        const ready=loaded.then(()=>{
            if(closed||context.state==='closed')return;
            const node=owned.node=new AudioWorkletNode(context,'matrix-rtc-decoded-pcm',{
                numberOfInputs:1,numberOfOutputs:1,outputChannelCount:[1],channelCount:1,
                channelCountMode:'explicit',channelInterpretation:'speakers'});
            const gain=owned.gain=context.createGain();gain.gain.value=0;
            node.port.onmessage=event=>{
                if(closed)return;
                const value=event.data;
                if(value?.type==='pcm'){
                    if(!(value.samples instanceof Float32Array)||value.samples.length!==960||!Number.isFinite(value.timestamp)){
                        invalid(connectionId,'Invalid decoded RTC PCM observation');return;
                    }
                    const at=performance.now();
                    audio(connectionId,value.timestamp,at,value.samples,owned.clock);
                    // The original parent measurement and public-owner selection
                    // decide whether diagnostics may inspect this existing frame.
                    let origin=null;try{origin=measurementOrigin(connectionId);}catch{}
                    audioBoundaries.observe({owner:connectionId,clock:owned.clock,
                        timestamp:value.timestamp,at,origin,samples:value.samples,context:owned.context});
                }else if(value?.type==='error')invalid(connectionId,value.reason);
            };
            node.onprocessorerror=()=>{if(!closed)invalid(connectionId,'Decoded RTC PCM processor failed');};
            source.connect(node);node.connect(gain);gain.connect(context.destination);
        }).catch(error=>{if(!closed&&context.state!=='closed')invalid(connectionId,String(error.message));});
        pending.add(ready);void ready.finally(()=>pending.delete(ready));
    }
    const observedConnect=function(...args){
        const result=sourceConnect.apply(this,args);
        if(rendering&&!closed&&!observed.has(this)){
            observed.add(this);
            tap(this.context,this,rendering);
        }
        return result;
    };
    const observedDisconnect=function(...args){
        const owned=[...taps].filter(tap=>tap.source===this);
        for(const tap of owned)lifetime(tap,'source-disconnect-request',{target:args.length===0?'all-outputs':
            typeof args[0]==='number'?'output-index':args[0]===tap.node?'tap-target':'other-target',
            output_index:typeof args[0]==='number'?args[0]:null});
        try{return sourceDisconnect.apply(this,args);}
        catch(error){for(const tap of owned)lifetime(tap,'source-disconnect-threw');throw error;}
    };
    const observedContextClose=function(...args){
        const owned=[...taps].filter(tap=>tap.context===this);
        for(const tap of owned)lifetime(tap,'context-close-request');
        let original;try{original=contextClose.apply(this,args);}
        catch(error){for(const tap of owned)lifetime(tap,'context-close-threw');throw error;}
        for(const tap of owned){
            if(original&&typeof original.then==='function')observeSettlement(original,tap.clock.epoch_id,'context-close-resolved','context-close-rejected');
            else lifetime(tap,'context-close-promise-unavailable');
        }
        return original;
    };
    sourcePrototype.connect=observedConnect;
    if(typeof sourceDisconnect==='function')sourcePrototype.disconnect=observedDisconnect;
    if(contextPrototype&&typeof contextClose==='function')contextPrototype.close=observedContextClose;
    const observedRender=function(input,...args){
        const previous=rendering;rendering=this.input.connectionId;
        // Label the exact existing native peer; never infer owners from creation order.
        globalThis.connectPeerTag?.(this.peer,this.input.connectionId);
        let entry;
        try{
            const streams=Object.keys(input?.video??{});
            if(streams.length===1)globalThis.connectPeerStreamTag?.(this.peer,this.input.connectionId,streams[0],'receive',this.remote?.get(streams[0]));
            if(decoded&&[...canvases.values()].includes(this.input.connectionId)){
                if(Object.values(input?.video??{}).length!==1
                    ||!Object.values(input.video).every(canvas=>canvases.get(canvas)===this.input.connectionId))
                    throw Error('Exact original matrix RTC render surface required');
                entry=decoded.begin(this,input);
            }
            const firstEpoch=nextAudioClock;
            const result=render.call(this,input,...args);
            observeRender(result,[...taps].filter(tap=>tap.clock.epoch_id>firstEpoch).map(tap=>tap.clock.epoch_id));
            if(entry)decoded.bind(entry,result);
            return result;
        }catch(error){if(entry)void decoded.abandon(entry);throw error;}
        finally{rendering=previous;}
    };
    Rtc.prototype.render=observedRender;
    const observedCapture=function(...args){
        // Private preparation still owns real capture and decoder readiness.
        // Its diagnostic identity does not require starting public playback.
        globalThis.connectPeerTag?.(this.peer,this.input.connectionId);
        const stream=this.input.video??this.input.screen;
        if(stream)globalThis.connectPeerStreamTag?.(this.peer,this.input.connectionId,stream,'send',this.senders?.get(stream));
        return capture.apply(this,args);
    };
    if(capture)Rtc.prototype.capture=observedCapture;
    const observedFrame=function(callback){
        return frame.call(this,(now,detail)=>{
            metadata.set(this,{now,detail});
            try{return callback(now,detail);}finally{metadata.delete(this);}
        });
    };
    HTMLVideoElement.prototype.requestVideoFrameCallback=observedFrame;
    const observedDraw=function(source,...args){
        const result=draw.call(this,source,...args),connectionId=canvases.get(this.canvas),current=metadata.get(source);
        if(!closed&&connectionId&&current){
            let identity = presentations.get(source);
            if (!identity) {
                if (nextPresentationElement >= 10) {
                    invalid(connectionId, 'Bounded original RTC presentation elements');
                    return result;
                }
                identity = {element_id:++nextPresentationElement, owner:connectionId, last:0};
                presentations.set(source, identity);
            }
            const presentation = {profile:'connect/rtc-video-presentation/1',
                element_id:identity.element_id, callbackTime:current.now};
            const descriptor=globalThis.connectPeerStreamDescriptor?.(connectionId,'receive');
            if(descriptor)Object.assign(presentation,descriptor);
            for (const key of ['mediaTime', 'presentedFrames', 'rtpTimestamp', 'presentationTime',
                'expectedDisplayTime', 'captureTime', 'receiveTime', 'processingDuration', 'width', 'height']) {
                if (key in current.detail) presentation[key] = current.detail[key];
            }
            let refusal;
            if (!Number.isFinite(current.detail.mediaTime)) {
                refusal = 'Decoded RTC video media time unavailable';
            } else if (identity.owner !== connectionId) {
                refusal = 'Decoded RTC video presentation element owner changed';
            } else if (!Number.isSafeInteger(presentation.presentedFrames) || presentation.presentedFrames <= 0) {
                refusal = 'Decoded RTC video presentation counter unavailable';
            } else if (presentation.presentedFrames <= identity.last) {
                refusal = 'Decoded RTC video presentation counter repeated or moved backward';
            }
            if (refusal) {
                invalid(connectionId, refusal, presentation);
                return result;
            }
            identity.last = presentation.presentedFrames;
            video(connectionId, Math.round(current.detail.mediaTime * 1000000), performance.now(), source,
                source.videoWidth, source.videoHeight, presentation);
        }
        return result;
    };
    CanvasRenderingContext2D.prototype.drawImage=observedDraw;
    return {
        report(){return audioBoundaries.report();},
        decodedVideoReport(){return decoded?.report()??null;},
        stopDecodedVideo(){return decoded?.close();},
        attach(connectionId,canvas){
            if(closed||canvases.has(canvas)||canvases.size>=10)throw Error('Bounded original RTC render owners');
            canvases.set(canvas,connectionId);
        },
        async close(){
            if(closed)return;closed=true;audioBoundaries.close();
            if(Rtc.prototype.render===observedRender)Rtc.prototype.render=render;
            if(sourcePrototype.connect===observedConnect){
                if(sourceConnectDescriptor)Object.defineProperty(sourcePrototype,'connect',sourceConnectDescriptor);
                else delete sourcePrototype.connect;
            }
            if(sourcePrototype.disconnect===observedDisconnect){
                if(sourceDisconnectDescriptor)Object.defineProperty(sourcePrototype,'disconnect',sourceDisconnectDescriptor);
                else delete sourcePrototype.disconnect;
            }
            if(contextPrototype?.close===observedContextClose){
                if(contextCloseDescriptor)Object.defineProperty(contextPrototype,'close',contextCloseDescriptor);
                else delete contextPrototype.close;
            }
            if(capture&&Rtc.prototype.capture===observedCapture)Rtc.prototype.capture=capture;
            if(CanvasRenderingContext2D.prototype.drawImage===observedDraw)CanvasRenderingContext2D.prototype.drawImage=draw;
            if(HTMLVideoElement.prototype.requestVideoFrameCallback===observedFrame)HTMLVideoElement.prototype.requestVideoFrameCallback=frame;
            let stateListenersUnconfirmed=0;
            for(const tap of taps)if(tap.onState){
                try{tap.context.removeEventListener('statechange',tap.onState);}catch{stateListenersUnconfirmed++;}
                tap.onState=null;
            }
            await Promise.all([...pending]);
            for(const tap of taps){
                if(tap.node){
                    tap.node.port.postMessage({type:'stop'});tap.node.port.onmessage=null;tap.node.port.close();
                    tap.node.onprocessorerror=null;
                    try{tap.source.disconnect(tap.node);}catch{}
                    tap.node.disconnect();
                }
                tap.gain?.disconnect();
            }
            const decodedVideo=await decoded?.close();
            URL.revokeObjectURL(worklet);canvases.clear();taps.clear();
            return {decoded_video:decodedVideo??null,audio_lifetime:{retained_taps:taps.size,
                state_listeners_unconfirmed:stateListenersUnconfirmed,observation_failures:lifetimeFailures,
                render_finished_promises_joined:false,
                hook_values_restored:Object.freeze({rtc_render:Rtc.prototype.render===render,
                    rtc_capture:capture?Rtc.prototype.capture===capture:null,
                    source_connect:sourcePrototype.connect===sourceConnect,
                    source_disconnect:typeof sourceDisconnect==='function'?sourcePrototype.disconnect===sourceDisconnect:null,
                    context_close:typeof contextClose==='function'?contextPrototype?.close===contextClose:null,
                    canvas_draw:CanvasRenderingContext2D.prototype.drawImage===draw,
                    video_frame_callback:HTMLVideoElement.prototype.requestVideoFrameCallback===frame})}};
        },
    };
}
