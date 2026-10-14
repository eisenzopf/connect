/** Unique ordinal calibration of independently decoded nonperiodic pulses. */
export const markerProgram="connect-av-cadence/1";
const horizon=300,maxMarkers=512,maxStates=1024,tolerance=.15;
export function markerFrames(){
    const digits=Array(19).fill(0),code=[];
    const visit=(at,period)=>{
        if(at>6){if(6%period===0)code.push(...digits.slice(1,period+1));}
        else{digits[at]=digits[at-period];visit(at+1,period);
            for(let value=digits[at-period]+1;value<3;value++){digits[at]=value;visit(at+1,at);}}
    };
    visit(1,1);let at=9;const frames=[];
    for(const value of code){if(at>=horizon*30)break;frames.push(at);at+=[15,27,39][value];}
    if(frames.length>maxMarkers)throw Error("Coded marker generator exceeded its bound");
    return frames;
}
const bound=(array,value,upper)=>{
    let lo=0,hi=array.length;
    while(lo<hi){const mid=Math.floor((lo+hi)/2);
        if(array[mid]<value||(upper&&array[mid]===value))lo=mid+1;else hi=mid;}
    return lo;
};
export function calibrate(edges,schedule=markerFrames().map(frame=>frame/30)){
    if(edges.length<7||edges.length>maxMarkers||!schedule.length||schedule.length>maxMarkers
        ||edges.some(at=>!Number.isFinite(at))||edges.some((at,index)=>index&&at<=edges[index-1]))
        return{status:"missing-proof",reason:"At least seven strictly ordered proven rising edges are required"};
    let states=schedule.map((at,ordinal)=>({ordinals:[ordinal],lo:edges[0]-at-tolerance,hi:edges[0]-at+tolerance}));
    let peak=states.length;
    for(const observed of edges.slice(1)){
        const following=[];
        for(const state of states){
            const first=Math.max(state.ordinals.at(-1)+1,bound(schedule,observed-state.hi-tolerance,false));
            const stop=bound(schedule,observed-state.lo+tolerance,true);
            for(let ordinal=first;ordinal<stop;ordinal++){
                const offset=observed-schedule[ordinal],lo=Math.max(state.lo,offset-tolerance),hi=Math.min(state.hi,offset+tolerance);
                if(lo<=hi)following.push({ordinals:[...state.ordinals,ordinal],lo,hi});
                if(following.length>maxStates)return{status:"ambiguous",reason:"Calibration candidate bound exceeded",candidate_bound:maxStates};
            }
        }
        states=following;peak=Math.max(peak,states.length);
        if(!states.length)return{status:"missing-proof",reason:"Decoded edges do not fit the coded cadence"};
    }
    if(states.length!==1)return{status:"ambiguous",reason:"Multiple ordinal assignments fit decoded edges",candidate_assignments:states.length,candidate_bound:maxStates};
    const state=states[0];
    return{status:"calibrated",ordinals:state.ordinals,edges_seconds:[...edges],source_offset_interval_seconds:[state.lo,state.hi],
        candidate_peak:peak,candidate_bound:maxStates,missing_internal_markers:state.ordinals.at(-1)-state.ordinals[0]+1-state.ordinals.length};
}
export function ordinalSync(videoEdges,audioEdges,seconds){
    const video=calibrate(videoEdges),audio=calibrate(audioEdges);
    const result={status:"failed",program:markerProgram,
        method:"independently decoded low-to-high edges uniquely calibrated against nonperiodic cadence; only equal source ordinals are paired, never nearest periodic edges",
        video_markers:videoEdges.length,audio_markers:audioEdges.length,video_calibration:video,audio_calibration:audio,
        paired:0,marker_pairs:[],median_abs_skew_ms:null,p95_abs_skew_ms:null,calibration_tolerance_seconds:tolerance,
        limits:{median_abs_skew_ms:80,p95_abs_skew_ms:120},quantile_method:"median averages the two middle values; p95 uses nearest rank"};
    if(video.status!=="calibrated"||audio.status!=="calibrated")return result;
    const byAudio=new Map(audio.ordinals.map((ordinal,index)=>[ordinal,audioEdges[index]]));
    const pairs=video.ordinals.flatMap((ordinal,index)=>byAudio.has(ordinal)?[{ordinal,video_emit_seconds:videoEdges[index],
        audio_emit_seconds:byAudio.get(ordinal),signed_video_minus_audio_ms:(videoEdges[index]-byAudio.get(ordinal))*1000}]:[]);
    const values=pairs.map(pair=>Math.abs(pair.signed_video_minus_audio_ms)).sort((a,b)=>a-b);
    Object.assign(result,{marker_pairs:pairs,paired:pairs.length,unmatched_video_markers:videoEdges.length-pairs.length,unmatched_audio_markers:audioEdges.length-pairs.length});
    if(values.length){result.median_abs_skew_ms=(values[Math.floor((values.length-1)/2)]+values[Math.floor(values.length/2)])/2;
        result.p95_abs_skew_ms=values[Math.ceil(values.length*.95)-1];
        result.status=values.length>=seconds*.7&&result.median_abs_skew_ms<=80&&result.p95_abs_skew_ms<=120?"passed":"failed";}
    return result;
}
