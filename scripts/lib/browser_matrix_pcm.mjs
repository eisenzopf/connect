const frequencies=[440,660,880,1100];
const check=(ok,message)=>{if(!ok)throw Error(message);};

// Actual decoded PCM, independent of RTP/SDK/sender counters. No expected-tone
// oscillator or generated samples enter this observer.
export function pcmIdentity(samples,expected) {
    check(samples.length===960&&Number.isInteger(expected)&&expected>=0&&expected<4,'Mono 20ms source identity');
    let energy=0;for(const value of samples){check(Number.isFinite(value),'Finite decoded PCM');energy+=value*value;}
    // Project the original 48kHz block directly. Unfiltered 8:1 sampling
    // folds other decoded frequencies into the four authored source tones.
    const powers=frequencies.map(hz=>{
        const coefficient=2*Math.cos(2*Math.PI*hz/48000);let a=0,b=0;
        for(let i=0;i<samples.length;i++){const value=samples[i]+coefficient*a-b;b=a;a=value;}
        return (a*a+b*b-coefficient*a*b)/960**2;
    });
    const rms=Math.sqrt(energy/samples.length),wrong=Math.max(...powers.filter((_,i)=>i!==expected));
    // Source recognition and usable-volume availability are independent.
    // Keep the original dominance/floor exactly; literal zero and unrecognized
    // PCM cannot identify a source. Quiet recognition earns no usable time.
    const source_identified=powers[expected]>8*Math.max(wrong,1e-12);
    return {rms,powers,source_identified,identified:energy/samples.length>1e-6&&source_identified};
}

// Prospective version2: original full rows and version1 remain available beside
// every decision. No inferred source clock, repaired PCM, or expected-tone
// samples enter this observer. A short observed tone can be nonorthogonal to
// the other authored tones: solve those ORIGINAL samples jointly, fail closed
// on rank/conditioning or residual energy, retain the original8x/floor rule.
export const STRICT_PCM_PROFILE='connect/decoded-tone-recognition/2';
export const PCM_MEASUREMENT_METHOD='connect/finite-support-decoded-tone-and-sample-coverage/1';
export const PCM_SAMPLE_PROFILE='connect/original-decoded-pcm-sample-coverage/1';
const minimumFitEdgeZeroRun=128;
const zeroRuns=samples=>{
    const runs=[];
    for(let i=0;i<samples.length;){if(samples[i]!==0){i++;continue;}
        const start=i;while(i<samples.length&&samples[i]===0)i++;
        runs.push({start_sample:start,end_sample_exclusive:i,
            before_sign:start>0?Math.sign(samples[start-1]):null,after_sign:i<samples.length?Math.sign(samples[i]):null});}
    return runs;
};
const componentPower=(samples,hz)=>{
    const coefficient=2*Math.cos(2*Math.PI*hz/48000);let a=0,b=0;
    for(const value of samples){const next=value+coefficient*a-b;b=a;a=next;}
    return (a*a+b*b-coefficient*a*b)/samples.length**2;
};
// Pivoted modified Gram-Schmidt, reorthogonalized twice. Full residual is
// recomputed from ORIGINAL samples. A small pivot or nonfinite result refuses;
// there is no least-squares/rank fallback or diagonal regularization.
function finiteToneFit(samples,start,end,bank){
    const n=end-start,columns=bank.flatMap(hz=>[
        Array.from({length:n},(_,i)=>Math.sin(2*Math.PI*hz*i/48000)),
        Array.from({length:n},(_,i)=>Math.cos(2*Math.PI*hz*i/48000)),
    ]),count=columns.length;
    if(n<64||n<2*count)return null;
    const working=columns.map(column=>column.slice()),order=Array.from({length:count},(_,i)=>i),
        triangular=Array.from({length:count},()=>Array(count).fill(0)),projected=[],q=[];
    let largest=0,smallest=Infinity;
    for(let k=0;k<count;k++){
        let pivot=k;for(let j=k+1;j<count;j++)if(working[j].reduce((s,x)=>s+x*x,0)>working[pivot].reduce((s,x)=>s+x*x,0))pivot=j;
        [working[k],working[pivot]]=[working[pivot],working[k]];[order[k],order[pivot]]=[order[pivot],order[k]];
        for(let i=0;i<k;i++)[triangular[i][k],triangular[i][pivot]]=[triangular[i][pivot],triangular[i][k]];
        const norm=Math.sqrt(working[k].reduce((s,x)=>s+x*x,0));largest=Math.max(largest,norm);smallest=Math.min(smallest,norm);
        if(!Number.isFinite(norm)||norm===0||smallest/largest<1e-4)return null;
        triangular[k][k]=norm;q[k]=working[k].map(x=>x/norm);
        projected[k]=q[k].reduce((sum,x,i)=>sum+x*samples[start+i],0);
        for(let j=k+1;j<count;j++)for(let pass=0;pass<2;pass++){
            const dot=q[k].reduce((sum,x,i)=>sum+x*working[j][i],0);triangular[k][j]+=dot;
            working[j]=working[j].map((x,i)=>x-dot*q[k][i]);
        }
    }
    const permuted=Array(count).fill(0),coefficients=Array(count).fill(0);
    for(let k=count-1;k>=0;k--)permuted[k]=(projected[k]-triangular[k].reduce((sum,x,j)=>sum+(j>k?x*permuted[j]:0),0))/triangular[k][k];
    for(let k=0;k<count;k++)coefficients[order[k]]=permuted[k];
    if(!coefficients.every(Number.isFinite))return null;
    const powers=bank.map((_,i)=>(coefficients[2*i]**2+coefficients[2*i+1]**2)/4*n/samples.length),componentEnergy=bank.map(()=>0);
    let residual=0;
    for(let i=0;i<samples.length;i++){
        let modeled=0;
        if(i>=start&&i<end)for(let j=0;j<bank.length;j++){
            const value=columns[2*j][i-start]*coefficients[2*j]+columns[2*j+1][i-start]*coefficients[2*j+1];
            modeled+=value;componentEnergy[j]+=value*value;
        }
        residual+=(samples[i]-modeled)**2;
    }
    const residualPower=residual/(2*samples.length);
    if(!powers.every(Number.isFinite)||!Number.isFinite(residualPower))return null;
    return {powers,residual_power:residualPower,component_energy:componentEnergy,
        minimum_relative_pivot:smallest/largest,support_samples:n};
}
export function strictPcmIdentity(samples,expected,{competitors=[]}={}){
    const legacy=pcmIdentity(samples,expected);
    check(Array.isArray(competitors)&&competitors.length<=2&&competitors.every(hz=>Number.isFinite(hz)&&hz>0&&hz<24000),'Bounded extra tone competitors');
    const bank=[...frequencies,...competitors.filter(hz=>!frequencies.includes(hz))];
    check(new Set(bank).size===bank.length,'Unique extra tone competitors');
    const runs=zeroRuns(samples),prefix=runs[0]?.start_sample===0?runs[0].end_sample_exclusive:0,
        tail=runs.at(-1),suffix=tail?.end_sample_exclusive===960?960-tail.start_sample:0;
    const start=prefix>=minimumFitEdgeZeroRun?prefix:0,end=Math.max(start,suffix>=minimumFitEdgeZeroRun?960-suffix:960);
    let fit=null,powers=[...legacy.powers,...bank.slice(4).map(hz=>componentPower(samples,hz))];
    // Existing full-window recognized rows retain their exact powers/verdict.
    // Only long ORIGINAL edge zeros permit a finite-support correction.
    if(!legacy.source_identified&&(start>0||end<960))fit=finiteToneFit(samples,start,end,bank);
    if(fit)powers=fit.powers;
    const wrong=Math.max(...powers.filter((_,i)=>i!==expected),fit?.residual_power??0,1e-12),
        source=fit?powers[expected]>8*wrong:legacy.source_identified,
        energy=legacy.rms**2*960,
        explained=energy>0?Math.min(2*powers[expected]/legacy.rms**2,fit?fit.component_energy[expected]/energy:Infinity):0;
    return {...legacy,profile:STRICT_PCM_PROFILE,measurement_method_profile:PCM_MEASUREMENT_METHOD,legacy,powers:powers.slice(0,4),
        source_identified:source,identified:fit?source&&energy/960>1e-6:legacy.identified,
        extra_powers:bank.slice(4).map((hz,i)=>({hz,power:powers[4+i]})),
        expected_explained_energy_fraction:explained,
        finite_support:fit?{used:true,start_sample:start,end_sample_exclusive:end,
            support_samples:fit.support_samples,residual_power:fit.residual_power,minimum_relative_pivot:fit.minimum_relative_pivot}: {used:false},
        original_samples:960,first_sample_sign:Math.sign(samples[0]),last_sample_sign:Math.sign(samples[959]),exact_zero_samples:runs.reduce((n,run)=>n+run.end_sample_exclusive-run.start_sample,0),exact_zero_runs:runs};
}

// Same original public interval, owner and decoded clock ONLY. Debit EVERY
// exactzero by default. Exempt only ONE zero bracketed by observed nonzero
// samples of opposite sign. A missing neighbor never supplies a guessed sign.
// Two or more zeros, same-sign one-sample holes and unknown edges debit ALL.
// Row arrival is the SAME start anchor transitionCoverage already uses in
// missing_start_ms=left+frameMs; decoded positions prove only contiguity.
export class PcmSampleLedger {
    constructor(){this.rows=[];this.clocks=new Map();this.currentClocks=new Map();this.ownerSamples=new Map();}
    push({owner,clock,timestamp,at,identity,position=timestamp,rate=1000000,duplicate=false,publicInterval=0,usable=identity.identified}){
        check(typeof owner==='string'&&owner.length>0&&clock&&clock.owner===owner,'Original decoded PCM owner/clock');
        check(identity?.profile===STRICT_PCM_PROFILE&&Number.isSafeInteger(timestamp)&&Number.isSafeInteger(position)&&Number.isFinite(at)&&[48000,1000000].includes(rate),'Original decoded PCM sample row');
        check(this.rows.length<16000,'Bounded original PCM sample rows');
        const key=JSON.stringify([owner,clock,publicInterval]),previous=this.clocks.get(key);
        check(previous||this.clocks.size<30,'Bounded original public PCM clocks');
        check(!previous||this.currentClocks.get(owner)===key,'Retired original PCM clock/public interval');
        const delta=previous?position-previous.row.position:null;
        check(!previous||delta===0||delta*48000>=960*rate,'Overlapping or backward original PCM position');
        duplicate ||= delta===0;
        const row={owner,clock,key,timestamp,at,position,rate,publicInterval,identity,duplicate,
            usable,available_samples:usable&&!duplicate?960:0,unavailable_runs:[],row_index:this.rows.length};
        this.rows.push(row);this.ownerSamples.set(owner,(this.ownerSamples.get(owner)??0)+row.available_samples);
        const state={row,pending:null},contiguous=previous&&rate===previous.row.rate&&delta*48000===960*rate&&!duplicate&&!previous.row.duplicate;
        const charge=(part)=>{
            const r=part.row,range={start_sample:part.start,end_sample_exclusive:part.end};r.unavailable_runs.push(range);part.range=range;
            if(r.usable&&!r.duplicate){const n=part.end-part.start;r.available_samples-=n;this.ownerSamples.set(r.owner,this.ownerSamples.get(r.owner)-n);}
        };
        const refund=(part)=>{
            const r=part.row,index=r.unavailable_runs.indexOf(part.range);check(index>=0,'Original charged zero required');r.unavailable_runs.splice(index,1);
            if(r.usable&&!r.duplicate){const n=part.end-part.start;r.available_samples+=n;this.ownerSamples.set(r.owner,this.ownerSamples.get(r.owner)+n);}
        };
        // A trailing singlezero was debited while its right neighbor was
        // unknown. ONLY this exact next ORIGINAL contiguous nonzero sample may
        // prove an actual sign crossing and refund that original one sample.
        if(contiguous&&previous.pending?.samples===1&&identity.first_sample_sign!==0
            &&previous.pending.before_sign!==null&&previous.pending.before_sign*identity.first_sample_sign<0){
            refund(previous.pending.part);
        }
        for(const run of identity.exact_zero_runs){
            const continuation=run.start_sample===0&&contiguous?previous.pending:null,
                count=run.end_sample_exclusive-run.start_sample+(continuation?.samples??0),
                before=continuation?continuation.before_sign:run.before_sign??(run.start_sample===0&&contiguous?previous.row.identity.last_sample_sign:null),
                after=run.after_sign,part={row,start:run.start_sample,end:run.end_sample_exclusive};
            const observedCrossing=count===1&&before!==null&&after!==null&&before!==0&&after!==0&&before*after<0;
            if(!observedCrossing)charge(part);
            if(run.end_sample_exclusive===960)state.pending={samples:count,before_sign:before,part};
        }
        this.clocks.set(key,state);this.currentClocks.set(owner,key);
        return row;
    }
    availableSamples(owner){return this.ownerSamples.get(owner)??0;}
    report(rows=this.rows){return {profile:PCM_SAMPLE_PROFILE,measurement_method_profile:PCM_MEASUREMENT_METHOD,
        zero_policy:'Every exactzero debited except isolated one-sample crossing with actual opposite-sign nonzero neighbors; unknown neighbors debit',
        finite_fit_edge_zero_threshold_samples:minimumFitEdgeZeroRun,
        anchor:'Original callback start plus ORIGINAL sample offset/48000; same transitionCoverage anchor, not capture-time inference',
        physical_audibility_proved:false,original_rows:rows.length,
        available_samples:rows.reduce((sum,row)=>sum+row.available_samples,0),
        unavailable_samples:rows.reduce((sum,row)=>sum+960-row.available_samples,0),
        exact_zero_samples:rows.reduce((sum,row)=>sum+row.identity.exact_zero_samples,0),
        bad_identity_rows:rows.filter(row=>!row.identity.source_identified).length,
        duplicate_rows:rows.filter(row=>row.duplicate).length,
        legacy_bad_identity_rows:rows.filter(row=>!row.identity.legacy.source_identified).length,
        legacy_identified_seconds:rows.filter(row=>row.identity.legacy.identified&&!row.duplicate).length*.02,
        identified_seconds:rows.reduce((sum,row)=>sum+row.available_samples,0)/48000,
        missing_zero_runs:rows.flatMap(row=>row.unavailable_runs.map(run=>({row_index:row.row_index,owner:row.owner,clock:row.clock,...run}))).slice(0,64),
        missing_zero_run_limit:64,missing_zero_runs_omitted:Math.max(0,rows.reduce((sum,row)=>sum+row.unavailable_runs.length,0)-64)};}
}
const unionIntervals=values=>{
    const result=[];for(const [start,end]of values.filter(([a,b])=>b>a).sort((a,b)=>a[0]-b[0])){
        const last=result.at(-1);if(last&&start<=last[1])last[1]=Math.max(last[1],end);else result.push([start,end]);}
    return result;
};
function withoutIntervals(values,excluded){
    let result=values;for(const [start,end]of excluded)result=result.flatMap(([a,b])=>b<=start||a>=end?[[a,b]]:
        [...(a<start?[[a,start]]:[]),...(b>end?[[end,b]]:[])]);return result;
}
// Coverage ALWAYS uses the caller's original admitted handoff holes. It never
// grants a cut, extends a hole, moves a row, or changes a numeric allowance.
export function pcmSampleCoverage(rows,transition,{startMs,endMs}){
    const usable=rows.filter(row=>row.available_samples>0&&!row.duplicate),times=usable.map(row=>row.at),
        boundaries=[startMs,...times,endMs],holes=transition.transitions.map(gap=>[gap.missing_start_ms,gap.missing_end_ms]);
    const base=[];
    for(let i=1;i<boundaries.length;i++){
        const left=boundaries[i-1],right=boundaries[i];
        if(!transition.transitions.some(gap=>gap.last_sample_ms===left&&gap.resumed_sample_ms===right))
            base.push([i===1?left:left+20,right]);
    }
    const missing=rows.flatMap(row=>row.unavailable_runs.map(run=>[row.at+run.start_sample/48,row.at+run.end_sample_exclusive/48]));
    const clean=unionIntervals(withoutIntervals([...base,...missing].map(([a,b])=>[Math.max(startMs,a),Math.min(endMs,b)]),holes));
    // A callback burst never compresses the length of an ORIGINAL decoded
    // zero run. These durations use only its own clock/sample coordinates.
    const decodedRuns=new Map();let maximumDecodedZeroRunSamples=0;
    for(const row of rows)for(const run of row.unavailable_runs){
        const firstPosition=row.position*48000+run.start_sample*row.rate,
            lastPosition=row.position*48000+run.end_sample_exclusive*row.rate,
            previous=decodedRuns.get(row.key),length=run.end_sample_exclusive-run.start_sample;
        const total=previous&&previous.end===firstPosition?previous.samples+length:length;
        decodedRuns.set(row.key,{end:lastPosition,samples:total});maximumDecodedZeroRunSamples=Math.max(maximumDecodedZeroRunSamples,total);
    }
    const first=usable[0],firstMissing=first?.unavailable_runs.find(run=>run.start_sample===0),
        firstAvailableAt=first?first.at+(firstMissing?.end_sample_exclusive??0)/48:null;
    return {profile:PCM_SAMPLE_PROFILE,measurement_method_profile:PCM_MEASUREMENT_METHOD,identified_seconds:rows.reduce((sum,row)=>sum+row.available_samples,0)/48000,
        unavailable_zero_samples:rows.reduce((sum,row)=>sum+row.unavailable_runs.reduce((n,run)=>n+run.end_sample_exclusive-run.start_sample,0),0),
        maximum_clean_gap_ms:Math.max(transition.maximum_clean_gap_ms,maximumDecodedZeroRunSamples?maximumDecodedZeroRunSamples/48+20:0,0,...clean.map(([a,b])=>b-a+20)),
        maximum_decoded_zero_run_samples:maximumDecodedZeroRunSamples,first_available_ms:firstAvailableAt,transition_missing_ms:transition.transition_missing_ms,
        original_transition_coverage:transition,clean_unavailable_intervals_ms:clean.slice(0,64),clean_interval_limit:64,clean_intervals_omitted:Math.max(0,clean.length-64),
        original_callback_interval_anchoring:true,new_handoff_allowance_ms:0};
}
