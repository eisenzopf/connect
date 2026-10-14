/** Aggregate activation proof, distinct from per-keyframe arrival timing. */
export function workerPipeline(initial, final, samples, began, ended) {
    const fields=['observedFrames','observedKeyframes','keyframeRequests','keyframeCompletions','keyframeFailures'];
    const valid=initial&&final&&fields.every(key=>Number.isSafeInteger(initial[key])&&initial[key]>=0
        &&Number.isSafeInteger(final[key])&&final[key]>=initial[key]);
    const deltas=Object.fromEntries(fields.map(key=>[key,valid?final[key]-initial[key]:null]));
    const seen=samples.filter(row=>row.unix_seconds>=began&&row.unix_seconds<=ended&&row.outbound?.worker);
    const boundaries=[began,...seen.map(row=>row.unix_seconds),ended];
    const gaps=boundaries.slice(1).map((at,index)=>at-boundaries[index]);
    return {status:valid&&deltas.observedFrames>0&&deltas.observedKeyframes>0&&deltas.keyframeRequests>0
        &&deltas.keyframeCompletions>0&&deltas.keyframeFailures===0?'passed':'failed',
        scope:'aggregate worker passthrough, subsequent keyframe, request and completion during actual measured window; counts do not bound inter-keyframe gaps',
        deltas,snapshot_samples_in_window:seen.length,
        maximum_snapshot_observation_gap_seconds:Math.max(...gaps),
        keyframe_interval_proof:'unmeasured by aggregate counters; independent decoded keyframe journals measure received intervals'};
}
