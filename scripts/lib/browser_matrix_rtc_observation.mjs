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
export function browserMatrixRtcObservation(Rtc, {audio, video, invalid}) {
    const render=Rtc.prototype.render,createSource=AudioContext.prototype.createMediaStreamSource;
    const draw=CanvasRenderingContext2D.prototype.drawImage,frame=HTMLVideoElement.prototype.requestVideoFrameCallback;
    const canvases=new Map(),metadata=new WeakMap(),pending=new Set(),taps=new Set(),modules=new WeakMap();
    let rendering=null,closed=false;
    const worklet=URL.createObjectURL(new Blob([rtcDecodedAudioWorklet],{type:'text/javascript'}));
    function tap(context,source,connectionId) {
        const owned={context,source,node:null,gain:null};taps.add(owned);
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
                    audio(connectionId,value.timestamp,performance.now(),value.samples);
                }else if(value?.type==='error')invalid(connectionId,value.reason);
            };
            node.onprocessorerror=()=>{if(!closed)invalid(connectionId,'Decoded RTC PCM processor failed');};
            source.connect(node);node.connect(gain);gain.connect(context.destination);
        }).catch(error=>{if(!closed&&context.state!=='closed')invalid(connectionId,String(error.message));});
        pending.add(ready);void ready.finally(()=>pending.delete(ready));
    }
    AudioContext.prototype.createMediaStreamSource=function(...args){
        const source=createSource.apply(this,args);
        if(rendering&&!closed)tap(this,source,rendering);
        return source;
    };
    Rtc.prototype.render=function(input,...args){
        const previous=rendering;rendering=this.input.connectionId;
        // Label the exact existing native peer; never infer owners from creation order.
        globalThis.connectPeerTag?.(this.peer,this.input.connectionId);
        try{return render.call(this,input,...args);}finally{rendering=previous;}
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
            if(!Number.isFinite(current.detail.mediaTime))invalid(connectionId,'Decoded RTC video media time unavailable');
            else video(connectionId,Math.round(current.detail.mediaTime*1000000),performance.now(),source,
                source.videoWidth,source.videoHeight);
        }
        return result;
    };
    return {
        attach(connectionId,canvas){
            if(closed||canvases.has(canvas)||canvases.size>=10)throw Error('Bounded original RTC render owners');
            canvases.set(canvas,connectionId);
        },
        async close(){
            if(closed)return;closed=true;
            Rtc.prototype.render=render;AudioContext.prototype.createMediaStreamSource=createSource;
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
