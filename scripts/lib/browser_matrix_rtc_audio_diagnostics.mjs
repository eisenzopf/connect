// Diagnostic metadata from the existing public render tap only. No audio graph,
// payload retention, sample generation, playout changes, or quality exceptions.
const number=read=>{try{const value=read();return Number.isFinite(value)?value:null;}catch{return null;}};
export function rtcAudioBoundaryDiagnostics() {
    const limit=32,minimum=128,events=[];let omitted=0,closed=false;
    return {
        observe({owner,clock,timestamp,at,origin,samples,context}) {
            if(closed||!Number.isFinite(origin)||!Number.isFinite(at)||at<origin||at>=origin+62000)return;
            let run=0,longest=0,start=null;
            for(let i=0;i<samples.length;i++){
                run=samples[i]===0?run+1:0;
                if(run>longest){longest=run;start=i-run+1;}
            }
            if(longest<minimum)return;
            if(events.length>=limit){omitted++;return;}
            let output=null;
            try{
                const value=context?.getOutputTimestamp?.();
                if(value)output=Object.freeze({context_time_seconds:number(()=>value.contextTime),
                    performance_time_ms:number(()=>value.performanceTime)});
            }catch{}
            events.push(Object.freeze({owner,clock:Object.freeze({profile:clock.profile,owner:clock.owner,epoch_id:clock.epoch_id}),
                original_worklet_timestamp_us:timestamp,callback_performance_ms:at,callback_seconds:(at-origin)/1000,
                sample_count:samples.length,exact_zero_run_samples:longest,exact_zero_run_start_sample:start,
                exact_zero_run_end_sample_exclusive:start+longest,
                exact_zero_start_worklet_us:timestamp+start*1000000/48000,
                exact_zero_end_worklet_us:timestamp+(start+longest)*1000000/48000,
                context:Object.freeze({current_time_seconds:number(()=>context.currentTime),
                    sample_rate:number(()=>context.sampleRate),base_latency_seconds:number(()=>context.baseLatency),
                    output_latency_seconds:number(()=>context.outputLatency),output_timestamp:output})}));
        },
        report(){return Object.freeze({profile:'connect/rtc-render-audio-boundary/1',
            scope:'Exact zeros copied from original public WebAudio input; context values sampled at main-thread callback, not proof of pre-WebAudio content or physical playout',
            minimum_exact_zero_samples:minimum,limit,omitted,events:Object.freeze(events.slice())});},
        close(){closed=true;},
    };
}
