/** Received decoded-audio identity at every observed point, with exact faults. */
export function audioObservation(content,expectedHz,mode="clean",transitions=[],bursts=[]){
    const began=content.window_start_unix_seconds,ended=content.window_end_unix_seconds;
    const samples=content.samples.filter(row=>row.unix_seconds>=began&&row.unix_seconds<=ended);
    const identified=row=>Number.isFinite(row.rms)&&Number.isFinite(row.frequency_hz)&&row.rms>.001&&Math.abs(row.frequency_hz-expectedHz)<35;
    const boundaries=[began,...samples.map(row=>row.unix_seconds),ended];
    const gaps=boundaries.slice(1).map((at,index)=>at-boundaries[index]);
    const covered=ended>began&&samples.every((row,index)=>!index||row.unix_seconds>samples[index-1].unix_seconds)
        &&samples.length<=2000&&samples.length>=Math.floor((ended-began)/.1)*.9&&Math.max(...gaps)<=.2;
    const result={status:"failed",method:"throughout-window independent received AudioContext RMS/frequency samples; no physical speaker claim",
        expected_frequency_hz:expectedHz,samples:samples.length,coverage_complete:covered,maximum_sampling_gap_seconds:Math.max(...gaps),
        configured_interval_seconds:.1,coverage_bounds:{maximum_samples:2000,minimum_expected_sample_fraction:.9,maximum_gap_seconds:.2},
        observed_exceptions:[],bad_unexcepted_samples:0,limits:{identified_rms_above:.001,frequency_error_below_hz:35,recovery_seconds:2,
            mute_silence_rms_at_most:.003,mute_observation_seconds:.5}};
    if(!["clean","mute","impairment"].includes(mode)||!covered)return result;
    const exceptions=[],checks=[];
    if(mode==="mute"){
        const events=transitions.filter(event=>event.kind==="audio");
        if(events.length!==2||!events[0].muted||events[1].muted)return result;
        const on=events[0].unix_seconds,off=events[1].unix_seconds;
        if(!(began<=on&&on<off&&off<ended))return result;
        const silence=samples.find(row=>row.unix_seconds>=on&&row.rms<=.003)?.unix_seconds;
        const resumed=samples.find(row=>row.unix_seconds>=off&&identified(row))?.unix_seconds;
        let valid=silence!==undefined&&resumed!==undefined&&silence<=on+.5&&silence<off&&resumed-off<=2;
        if(valid){const muted=samples.filter(row=>row.unix_seconds>=silence&&row.unix_seconds<off);
            valid=Boolean(muted.length)&&muted.every(row=>row.rms<=.003);exceptions.push([silence,resumed]);}
        checks.push({kind:"actual-microphone-mute",mute_unix_seconds:on,unmute_unix_seconds:off,
            observed_silence_start_unix_seconds:silence??null,observed_identified_recovery_unix_seconds:resumed??null,status:valid?"passed":"failed"});
    }else if(mode==="impairment"){
      let previousDeadline=-Infinity;
      for(const burst of bursts){
        const start=burst.start_unix_seconds,stop=burst.end_unix_seconds,deadline=stop+2;
        const before=samples.filter(row=>row.unix_seconds<start).at(-1);
        const aftermath=samples.filter(row=>row.unix_seconds>=start&&row.unix_seconds<=deadline);
        const lows=aftermath.filter(attenuated),lastLow=lows.at(-1)?.unix_seconds;
        const first=aftermath.find(row=>row.unix_seconds>=stop&&identified(row))?.unix_seconds;
        const resumed=aftermath.find(row=>row.unix_seconds>=stop&&(lastLow===undefined||row.unix_seconds>lastLow)&&identified(row))?.unix_seconds;
        const witness=samples.find(row=>resumed!==undefined&&row.unix_seconds>=resumed+.2&&identified(row))?.unix_seconds;
        const valid=began<=start&&start<stop&&deadline<=ended&&start>previousDeadline
            &&before&&identified(before)&&start-before.unix_seconds<=.2
            &&resumed!==undefined&&resumed<=deadline&&witness!==undefined
            &&aftermath.every(row=>identified(row)||attenuated(row))
            &&samples.filter(row=>row.unix_seconds>=resumed&&row.unix_seconds<=witness).every(identified);
        previousDeadline=deadline;
        if(valid)exceptions.push([start,resumed]);
        checks.push({kind:"observed-network-burst",start_unix_seconds:start,end_unix_seconds:stop,
            recovery_deadline_unix_seconds:deadline,first_identified_after_burst_unix_seconds:first??null,
            last_attenuation_unix_seconds:lastLow??null,attenuated_samples:lows,
            observed_identified_recovery_unix_seconds:resumed??null,
            identified_continuation_witness_unix_seconds:witness??null,status:valid?"passed":"failed"});
      }
    }
    function attenuated(row){return Number.isFinite(row.rms)&&row.rms>=0&&row.rms<=.001&&Number.isFinite(row.frequency_hz);}
    const excepted=row=>exceptions.some(([start,stop])=>row.unix_seconds>=start&&row.unix_seconds<stop)&&(mode!=="impairment"||attenuated(row));
    const bad=samples.filter(row=>!excepted(row)&&!identified(row));
    if(mode==="impairment")Object.assign(result,{observed_bad_samples:samples.filter(row=>!identified(row)).length,
        excepted_attenuation_samples:samples.filter(excepted).length,
        recovery_method:"last finite near-silence in the observed two-second aftermath, then identified return and covered continuation; wrong tone is never excepted"});
    Object.assign(result,{observed_exceptions:checks,bad_unexcepted_samples:bad.length,
        status:!bad.length&&checks.every(row=>row.status==="passed")?"passed":"failed"});
    return result;
}
