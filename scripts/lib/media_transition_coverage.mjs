// Only an observed gap straddling a validated public ownership cut can use
// the media contract's one-second handoff allowance. Preparation time is kept.
export const TRANSITION_PROFILE='connect/media-transition-coverage/1';

export function transitionCoverage(times,cuts,{startMs,endMs,frameMs,minimumGapMs}) {
    if(!Array.isArray(times)||times.length>16000||!Array.isArray(cuts)||cuts.length>2||
        ![startMs,endMs,frameMs,minimumGapMs].every(Number.isFinite)||
        startMs>=endMs||frameMs<=0||minimumGapMs<=frameMs)throw Error('Invalid transition coverage bounds');
    if(times.some((at,i)=>!Number.isFinite(at)||at<startMs||at>=endMs||(i&&at<times[i-1])))
        throw Error('Original media arrivals must be ordered and bounded');
    if(new Set(cuts.map(cut=>cut.id)).size!==cuts.length||cuts.some((cut,i)=>
        typeof cut.id!=='string'||!cut.id||!Number.isFinite(cut.atMs)||
        cut.atMs<=startMs||cut.atMs>=endMs||(i&&cut.atMs<=cuts[i-1].atMs)))
        throw Error('Original public ownership cuts must be unique and ordered');
    const boundaries=[startMs,...times,endMs],used=new Set(),transitions=[];
    let maximumCleanGapMs=0;
    for(let i=1;i<boundaries.length;i++){
        const left=boundaries[i-1],right=boundaries[i],span=right-left;
        const matching=cuts.filter(cut=>left<=cut.atMs&&cut.atMs<right);
        // A boundary at the start/end of the measured window cannot stand in
        // for an identified pre-switch sample and an actual resumed sample.
        if(i>1&&i<boundaries.length-1&&span>minimumGapMs&&matching.length===1&&
            !used.has(matching[0].id)&&span<=1000){
            const cut=matching[0];used.add(cut.id);
            transitions.push({id:cut.id,cut_ms:cut.atMs,last_sample_ms:left,
                resumed_sample_ms:right,interruption_ms:span,
                missing_start_ms:left+frameMs,missing_end_ms:right});
        }else maximumCleanGapMs=Math.max(maximumCleanGapMs,span);
    }
    const missingMs=transitions.reduce((sum,gap)=>sum+gap.missing_end_ms-gap.missing_start_ms,0);
    return {profile:TRANSITION_PROFILE,maximum_interruption_ms:1000,
        original_public_cuts:cuts.map(cut=>({...cut})),transitions,
        transition_missing_ms:missingMs,maximum_clean_gap_ms:maximumCleanGapMs};
}

export function cleanBinDurations(coverage,startMs,seconds) {
    return Array.from({length:seconds},(_,i)=>{
        const start=startMs+i*1000,end=start+1000;
        return 1000-coverage.transitions.reduce((sum,gap)=>sum+
            Math.max(0,Math.min(end,gap.missing_end_ms)-Math.max(start,gap.missing_start_ms)),0);
    });
}
