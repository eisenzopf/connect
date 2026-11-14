// Diagnostic metadata from the existing public render tap only. No audio graph,
// payload retention, sample generation, playout changes, or quality exceptions.
const number=read=>{try{const value=read();return Number.isFinite(value)?value:null;}catch{return null;}};
export function rtcAudioBoundaryDiagnostics({retainSuspectPcm=false,contextCloseProbeAvailable=null}={}) {
    const limit=32,minimum=128,events=[];let omitted=0,closed=false;
    const lifetimeLimit=128,lifetimeEvents=[];let lifetimeOmitted=0;
    const contextState=context=>{try{const state=context?.state;return ['running','suspended','closed','interrupted'].includes(state)?state:null;}catch{return null;}};
    const pcmLimit=8,pcmOwners=10,history=new Map(),pcmEvents=[];
    let pcmOmitted=0,pcmUnknown=0;
    const copy=frame=>Object.freeze({original_worklet_timestamp_us:frame.timestamp,
        callback_performance_ms:frame.at,original_worklet_end_timestamp_us:frame.timestamp+20000,
        context_state_at_delivery:frame.contextState,context_time_seconds_at_delivery:frame.contextTime,
        samples:Object.freeze(Array.from(frame.samples))});
    function retain({owner,clock,timestamp,at,samples,contextState,contextTime},suspect) {
        if(!retainSuspectPcm)return;
        if(!(samples instanceof Float32Array)||samples.length!==960||!samples.every(Number.isFinite)
            ||clock?.owner!==owner||!Number.isSafeInteger(clock?.epoch_id)||clock.epoch_id<1
            ||typeof owner!=='string'||owner.length>256||!Number.isFinite(timestamp)) {pcmUnknown++;return;}
        let prior=history.get(clock.epoch_id);
        if(prior&&prior.owner!==owner) {pcmUnknown++;return;}
        if(!prior&&history.size>=pcmOwners) {pcmUnknown++;return;}
        const contiguous=prior&&timestamp-prior.timestamp===20000;
        if(prior?.pending!==null&&prior?.pending!==undefined) {
            pcmEvents[prior.pending].after=contiguous?copy({timestamp,at,samples,contextState,contextTime}):null;
            pcmEvents[prior.pending].after_status=contiguous?'captured':'discontinuous';
        }
        let pending=null;
        if(suspect) {
            if(pcmEvents.length<pcmLimit) {
                pending=pcmEvents.length;
                pcmEvents.push({owner,clock_epoch_id:clock.epoch_id,
                    before:contiguous?copy(prior):null,current:copy({timestamp,at,samples,contextState,contextTime}),
                    after:null,after_status:'not-yet-observed'});
            }else pcmOmitted++;
        }
        // The callback borrows samples. Own only one prior960 block per clock;
        // no track, AudioContext, callback or renderer owner is retained here.
        history.set(clock.epoch_id,{owner,timestamp,at,samples:samples.slice(),pending,contextState,contextTime});
    }
    return {
        lifecycle({owner,clock,kind,at,context,fields={}}) {
            if(closed)return;
            if(lifetimeEvents.length>=lifetimeLimit){lifetimeOmitted++;return;}
            lifetimeEvents.push(Object.freeze({owner,clock_epoch_id:clock.epoch_id,kind,
                callback_performance_ms:at,context_state:contextState(context),
                context_time_seconds:number(()=>context.currentTime),...fields}));
        },
        observe({owner,clock,timestamp,at,origin,samples,context}) {
            if(closed||!Number.isFinite(origin)||!Number.isFinite(at)||at<origin||at>=origin+62000)return;
            let run=0,longest=0,start=null;
            for(let i=0;i<samples.length;i++){
                run=samples[i]===0?run+1:0;
                if(run>longest){longest=run;start=i-run+1;}
            }
            if(retainSuspectPcm)retain({owner,clock,timestamp,at,samples,contextState:contextState(context),
                contextTime:number(()=>context.currentTime)},longest>=minimum);
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
                context:Object.freeze({state:contextState(context),current_time_seconds:number(()=>context.currentTime),
                    sample_rate:number(()=>context.sampleRate),base_latency_seconds:number(()=>context.baseLatency),
                    output_latency_seconds:number(()=>context.outputLatency),output_timestamp:output})}));
        },
        report(){return Object.freeze({profile:'connect/rtc-render-audio-boundary/1',
            scope:'Exact zeros copied from original public WebAudio input; context values sampled at main-thread callback, not proof of pre-WebAudio content or physical playout',
            minimum_exact_zero_samples:minimum,limit,omitted,events:Object.freeze(events.slice()),
            lifetime:Object.freeze({profile:'connect/original-rtc-audio-lifetime/1',limit:lifetimeLimit,omitted:lifetimeOmitted,
                context_close_probe_available:contextCloseProbeAvailable,
                scope:'Requests and promise settlements on original public graph only; disconnect request is not proof of effective stop or physical playout. PCM is never filtered by lifetime.',
                events:Object.freeze(lifetimeEvents.slice())}),
            suspect_pcm:Object.freeze({enabled:retainSuspectPcm,profile:'connect/original-rtc-suspect-pcm/1',
                event_limit:pcmLimit,owner_limit:pcmOwners,omitted:pcmOmitted,owner_unknown:pcmUnknown,
                scope:'Original borrowed public PCM only; no packet-to-PCM identity or quality credit',
                complete:retainSuspectPcm&&pcmOmitted===0&&pcmUnknown===0
                    &&pcmEvents.every(row=>row.before&&row.after_status==='captured'),
                events:Object.freeze(pcmEvents.map(row=>Object.freeze({...row})))})});},
        close(){closed=true;history.clear();},
    };
}
