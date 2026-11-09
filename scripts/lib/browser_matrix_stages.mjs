// Diagnostics of original samples. They never substitute for decoded quality.
export function pcmBoundary(samples) {
    if(samples.length!==960||!samples.every(Number.isFinite))throw Error('Actual finite mono20ms PCM required');
    let zero=0,longest=0,near=0,largestStep=0,peak=0,leading=0,trailing=0;
    const rms1ms=[];
    for(let at=0;at<960;at+=48){let energy=0;for(let i=at;i<at+48;i++)energy+=samples[i]**2;rms1ms.push(Math.sqrt(energy/48));}
    const rms2ms=Array.from({length:10},(_,i)=>Math.sqrt((rms1ms[i*2]**2+rms1ms[i*2+1]**2)/2));
    for(let i=0;i<960;i++){
        const value=samples[i];zero=value===0?zero+1:0;longest=Math.max(longest,zero);
        if(Math.abs(value)<=1e-8)near++;peak=Math.max(peak,Math.abs(value));
        if(i)largestStep=Math.max(largestStep,Math.abs(value-samples[i-1]));
    }
    while(leading<960&&samples[leading]===0)leading++;
    while(trailing<960&&samples[959-trailing]===0)trailing++;
    return {sample_count:960,sample_rate:48000,rms_1ms:rms1ms,rms_2ms:rms2ms,
        longest_exact_zero_run_samples:longest,leading_exact_zero_samples:leading,trailing_exact_zero_samples:trailing,
        near_zero_samples:near,near_zero_absolute_limit:1e-8,maximum_absolute_sample_step:largestStep,
        absolute_peak:peak,first_sample:samples[0],last_sample:samples[959]};
}

export class MatrixMediaStages {
    constructor(){this.owners=new Map();this.overflow=false;}
    owner(connection,index){
        let row=this.owners.get(connection);
        if(!row){
            if(this.owners.size>=10){this.overflow=true;return null;}
            row={connection,index,audio_input:0,audio_accepted:0,video_input:0,video_accepted:0,
                last_audio_timestamp_us:null,audio_timestamp_gaps:0,nonforward_audio_timestamps:0,
                maximum_audio_timestamp_step_us:0,last_sample:null,maximum_cross_block_sample_step:0,
                suspect_audio:[],omitted_suspect_audio:0,snapshots:[],last_second:null};
            this.owners.set(connection,row);
        }
        return row;
    }
    snapshot(row, at, peer, refusal = false) {
        const second = Math.floor(at), sameSecond = row.last_second === second;
        if (sameSecond && !refusal) return;
        if (!sameSecond && row.snapshots.length >= 63) { this.overflow = true; return; }
        const routes = peer?.routeDiagnostics?.filter(route => route.connectionId === row.connection).slice(0,16) ?? null;
        const videoEncoders = routes?.filter(route => route.kind === 'video' && route.encoding).map(route => ({
            stream_id:route.streamId,
            encoding:Object.fromEntries(['inputCalls','acceptedFrames','nativeOutputs','forwardedOutputs',
                'stoppedRefusals','queueRefusals','timestampRefusals','rateRefusals','suspendedRefusals',
                'outputRateDrops','outputWaitingForKeyDrops','maxEncoderQueue',
                'lastInputTimestampUs','lastEncodedTimestampUs'].map(key => [key, route.encoding[key]]))
        })) ?? [];
        const snapshot = {callback_seconds:at, peer:peer?.diagnostics ?? null, routes, video_encoders:videoEncoders};
        if (sameSecond) row.snapshots[row.snapshots.length - 1] = snapshot;
        else row.snapshots.push(snapshot);
        row.last_second = second;
    }
    audio(connection,index,timestamp,at,samples,identity,accepted,peer){
        const row=this.owner(connection,index);if(!row)return;row.audio_input++;if(accepted)row.audio_accepted++;
        if(row.last_audio_timestamp_us!==null){
            const delta=timestamp-row.last_audio_timestamp_us;
            if(delta>20000)row.audio_timestamp_gaps++;if(delta<=0)row.nonforward_audio_timestamps++;
            row.maximum_audio_timestamp_step_us=Math.max(row.maximum_audio_timestamp_step_us,delta);
        }
        row.last_audio_timestamp_us=timestamp;
        const cross=row.last_sample===null?null:Math.abs(samples[0]-row.last_sample);
        if(cross!==null)row.maximum_cross_block_sample_step=Math.max(row.maximum_cross_block_sample_step,cross);
        row.last_sample=samples.at(-1);
        if(!identity.identified||identity.rms<.03){
            if(row.suspect_audio.length<32)row.suspect_audio.push({original_timestamp_us:timestamp,callback_seconds:at,
                identity,boundary:pcmBoundary(samples),cross_block_absolute_step:cross,encoder_accepted:accepted,
                codec:peer?.routeDiagnostics?.filter(r=>r.connectionId===connection&&r.kind==='audio').slice(0,4)??null});
            else row.omitted_suspect_audio++;
        }
        this.snapshot(row,at,peer);
    }
    video(connection,index,at,accepted,peer){
        const row=this.owner(connection,index);if(!row)return;row.video_input++;if(accepted)row.video_accepted++;
        this.snapshot(row,at,peer,!accepted);
    }
    report(){return {scope:'Original capture/codec diagnostics only; no media-quality pass',owner_bound:10,
        snapshot_bound_per_owner:63,suspect_bound_per_owner:32,low_rms_diagnostic_limit:.03,overflow:this.overflow,
        owners:[...this.owners.values()].map(({last_sample,last_second,...row})=>row)};}
}

export function browserMatrixRtcStages(read=()=>globalThis.connectPeerMediaDiagnostics?.()) {
    let origin=null,timer=null,stopped=false,pending=null,closing,attempts=0;const rows=[],failures=[];
    const stopTimer=()=>{if(timer!==null){clearInterval(timer);timer=null;}};
    const sample=()=>{
        if(stopped||pending)return;
        if(attempts>=64){stopTimer();return;}
        attempts++;const began=performance.now();
        pending=Promise.resolve().then(read).then(value=>{
            if(value===undefined)throw Error('RTC media statistics unavailable');
            if(rows.length<64)rows.push({requested_seconds:(began-origin)/1000,
                completed_seconds:(performance.now()-origin)/1000,peers:value});
        }).catch(()=>{if(failures.length<16)failures.push({at_seconds:(performance.now()-origin)/1000,
            reason:'original-rtc-statistics-unavailable'});}).finally(()=>{pending=null;});
    };
    return {
        start(value){if(origin!==null||stopped||!Number.isFinite(value))throw Error('One original statistics clock');
            origin=value;sample();timer=setInterval(sample,1000);},
        report(){return {scope:'Read-only original RTC statistics; no decoded-quality or delivery pass',sample_bound:64,attempts,
            samples:structuredClone(rows),failures:structuredClone(failures),pending:pending!==null};},
        close(){return closing??=(async()=>{
            stopped=true;stopTimer();read=null;
            if(!pending)return {joined:true};let wait;
            const original=pending;
            const joined=await Promise.race([original.then(()=>true),new Promise(resolve=>{wait=setTimeout(()=>resolve(false),500);})]);
            clearTimeout(wait);return {joined,outstanding_original_snapshot:!joined};
        })();},
    };
}
