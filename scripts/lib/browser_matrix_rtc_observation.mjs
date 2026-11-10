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
export function browserMatrixRtcObservation(Rtc, {audio, video, invalid, measurementOrigin=()=>null}) {
    const audioBoundaries=rtcAudioBoundaryDiagnostics();
    const render=Rtc.prototype.render,capture=Rtc.prototype.capture;
    const sourcePrototype=MediaStreamAudioSourceNode.prototype;
    const sourceConnect=sourcePrototype.connect;
    const sourceConnectDescriptor=Object.getOwnPropertyDescriptor(sourcePrototype,'connect');
    const draw=CanvasRenderingContext2D.prototype.drawImage,frame=HTMLVideoElement.prototype.requestVideoFrameCallback;
    const canvases=new Map(),metadata=new WeakMap(),pending=new Set(),taps=new Set(),modules=new WeakMap(),observed=new WeakSet();
    const presentations = new WeakMap();
    let nextPresentationElement = 0;
    let nextAudioClock = 0;
    let rendering=null,closed=false;
    const worklet=URL.createObjectURL(new Blob([rtcDecodedAudioWorklet],{type:'text/javascript'}));
    function tap(context,source,connectionId) {
        if (nextAudioClock >= 10) {
            invalid(connectionId, 'Bounded original RTC decoded PCM clocks');
            return;
        }
        const clock = Object.freeze({profile:'connect/rtc-decoded-pcm-clock/1',
            owner:connectionId, epoch_id:++nextAudioClock});
        const owned={context,source,node:null,gain:null,clock};taps.add(owned);
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
    MediaStreamAudioSourceNode.prototype.connect=function(...args){
        const result=sourceConnect.apply(this,args);
        if(rendering&&!closed&&!observed.has(this)){
            observed.add(this);
            tap(this.context,this,rendering);
        }
        return result;
    };
    Rtc.prototype.render=function(input,...args){
        const previous=rendering;rendering=this.input.connectionId;
        // Label the exact existing native peer; never infer owners from creation order.
        globalThis.connectPeerTag?.(this.peer,this.input.connectionId);
        try{return render.call(this,input,...args);}finally{rendering=previous;}
    };
    if(capture)Rtc.prototype.capture=function(...args){
        // Private preparation still owns real capture and decoder readiness.
        // Its diagnostic identity does not require starting public playback.
        globalThis.connectPeerTag?.(this.peer,this.input.connectionId);
        return capture.apply(this,args);
    };
    HTMLVideoElement.prototype.requestVideoFrameCallback=function(callback){
        return frame.call(this,(now,detail)=>{
            metadata.set(this,{now,detail});
            try{return callback(now,detail);}finally{metadata.delete(this);}
        });
    };
    CanvasRenderingContext2D.prototype.drawImage=function(source,...args){
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
    return {
        report(){return audioBoundaries.report();},
        attach(connectionId,canvas){
            if(closed||canvases.has(canvas)||canvases.size>=10)throw Error('Bounded original RTC render owners');
            canvases.set(canvas,connectionId);
        },
        async close(){
            if(closed)return;closed=true;audioBoundaries.close();
            Rtc.prototype.render=render;
            if(sourceConnectDescriptor)Object.defineProperty(sourcePrototype,'connect',sourceConnectDescriptor);
            else delete sourcePrototype.connect;
            if(capture)Rtc.prototype.capture=capture;
            CanvasRenderingContext2D.prototype.drawImage=draw;HTMLVideoElement.prototype.requestVideoFrameCallback=frame;
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
            URL.revokeObjectURL(worklet);canvases.clear();taps.clear();
        },
    };
}
