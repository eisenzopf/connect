/** Fresh independently decoded pixels; intentional exclusions use actual events. */
export function motionObservation(content,mode="clean",transitions=[],bursts=[]) {
    const began=content.window_start_unix_seconds,ended=content.window_end_unix_seconds,times=content.changing_times;
    const valid=["clean","mute","impairment"].includes(mode)&&Number.isFinite(began)&&Number.isFinite(ended)
        &&ended>began&&times.length>0&&times.length<=10000&&times.every(Number.isFinite)
        &&times.slice(1).every((at,index)=>at>times[index]);
    const observed=times.filter(at=>at>=began&&at<=ended);
    const result={status:"failed",method:"successive independently decoded pixel changes on actual receiver observation/emission times; no physical display or native decoder-completion claim",
        freshness_limit_seconds:2,changing_time_bound:10000,changing_observations_in_window:observed.length,
        observed_exclusions:[],active_segments:[],maximum_unexcluded_freshness_gap_seconds:null};
    if(!valid||!observed.length)return result;
    let exclusions=[];
    if(mode==="mute") {
        const events=transitions.filter(row=>row.kind==="video");
        if(events.length!==2||!events[0].muted||events[1].muted)return result;
        exclusions=[[events[0].unix_seconds,events[1].unix_seconds,"actual-camera-mute"]];
    } else if(mode==="impairment") {
        if(!bursts.length)return result;
        exclusions=bursts.map(row=>[row.start_unix_seconds,row.end_unix_seconds,"observed-network-burst"]);
    }
    if(exclusions.some(([start,stop])=>!Number.isFinite(start)||!Number.isFinite(stop)||!(began<start&&start<stop&&stop<ended))
        ||exclusions.slice(1).some((row,index)=>row[0]<=exclusions[index][1]))return result;
    let cursor=began;const segments=[];
    for(const[start,stop,kind]of exclusions) {
        segments.push([cursor,start]);cursor=stop;
        const before=observed.findLast(at=>at<start)??null,resumed=observed.find(at=>at>=stop)??null;
        const passed=before!==null&&start-before<=2&&resumed!==null&&resumed-stop<=2;
        result.observed_exclusions.push({kind,start_unix_seconds:start,end_unix_seconds:stop,
            last_fresh_content_before_unix_seconds:before,first_fresh_content_after_unix_seconds:resumed,status:passed?"passed":"failed"});
    }
    segments.push([cursor,ended]);
    for(const[start,stop]of segments) {
        const boundaries=[start,...observed.filter(at=>at>=start&&at<=stop),stop];
        const gap=Math.max(...boundaries.slice(1).map((at,index)=>at-boundaries[index]));
        result.active_segments.push({start_unix_seconds:start,end_unix_seconds:stop,
            maximum_freshness_gap_seconds:gap,status:gap<=2?"passed":"failed"});
    }
    result.maximum_unexcluded_freshness_gap_seconds=Math.max(...result.active_segments.map(row=>row.maximum_freshness_gap_seconds));
    result.status=[...result.active_segments,...result.observed_exclusions].every(row=>row.status==="passed")?"passed":"failed";
    return result;
}
