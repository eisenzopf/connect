import {BrowserOpusCodec,OPUS_CONFIG} from '../../sdk/typescript/dist/browser_codecs.js';
import {pcmIdentity} from './browser_matrix_pcm.mjs';
import {matrixAudioProfile} from './browser_matrix_sources.mjs';
const frequencies=[440,660,880,1100],check=(yes,message)=>{if(!yes)throw Error(message);};

/** Diagnostic component only: actual SDK Opus encode/decode and unchanged
 * classifier, with completely known synthetic input. This is not a Connect
 * transport/RTC/PLC/media qualification and grants no quality-frame credit. */
export async function browserOpusIdentityControl({deadlineMs}) {
    check(Number.isFinite(deadlineMs)&&deadlineMs>performance.now(),'Original bounded control deadline required');
    const result={profile:'connect/original-sdk-opus-identity-control/1',config:OPUS_CONFIG,
        source:matrixAudioProfile,source_profile_scope:'Current pinned .05/.2 peaks only; no historical14x source profile attribution',scenarios:[],complete:false,
        scope:'Known single-tone PCM through original SDK BrowserOpusCodec and original pcmIdentity. No actual cell attribution, threshold change or codec concealment claim.'};
    const failure=[];
    // Exactly 32 bounded scenarios, 96 original input frames each. Phase and
    // abrupt pulse edges are known before encode. No competing source exists.
    for(const source of [0,1,2,3])for(const phase of [0,.25,.5,.75])for(const omit of [null,48]) {
        const row={source,phase,omitted_original_packet:omit,inputs:0,encoded:0,decoded:0,
            codec_closed:false,unreported:0,samples:[],ambiguous:0,silent_or_weak:0,
            confidently_wrong:0,minimum_expected_ratio:null,raw_input_unidentified:0};
        let codec,pending=null;
        const finish=error=>{const original=pending;pending=null;if(original){if(error)original.reject(error);else original.resolve();}};
        try {
            codec=new BrowserOpusCodec({
                encoded(frame){
                    row.encoded++;
                    try {
                        check(pending&&frame.timestampUs===pending.timestamp,'Exact original SDK encoded timestamp required');
                        if(pending.index===omit){finish();return;}
                        check(codec.decode(frame.bytes,frame.timestampUs),'Original decoder refused bounded packet');
                    }catch(error){finish(error);}
                },
                decoded(frame){
                    try{
                        check(pending&&frame.timestamp===pending.timestamp&&frame.numberOfFrames===960
                            &&frame.sampleRate===48000&&frame.numberOfChannels===1,'Exact original borrowed decoded frame required');
                        const samples=new Float32Array(960);frame.copyTo(samples,{planeIndex:0,format:'f32-planar'});
                        const actual=pcmIdentity(samples,source),winner=actual.powers.indexOf(Math.max(...actual.powers)),
                            other=Math.max(...actual.powers.filter((_,i)=>i!==source)),ratio=actual.powers[source]/Math.max(other,1e-12),
                            weak=actual.rms*actual.rms<=1e-6,wrong=winner!==source&&pcmIdentity(samples,winner).identified;
                        row.decoded++;row.minimum_expected_ratio=Math.min(row.minimum_expected_ratio??Infinity,ratio);
                        if(!actual.identified){
                            if(weak)row.silent_or_weak++;else if(wrong)row.confidently_wrong++;else row.ambiguous++;
                            const sample={input_index:pending.index,original_timestamp_us:frame.timestamp,rms:actual.rms,
                                powers:actual.powers,expected_ratio:ratio,strongest_source:winner,
                                classification:weak?'silent-or-weak':wrong?'confidently-wrong':'spectrally-ambiguous'};
                            if(row.samples.length<64)row.samples.push(sample);else row.unreported++;
                        }
                        finish();
                    }catch(error){finish(error);}
                    // Original SDK owns and closes this borrowed AudioData.
                },error:finish,
            });
            for(let index=0;index<96;index++) {
                check(performance.now()<deadlineMs,'Original control deadline exhausted');
                // Warmup then a 100ms high pulse crossing a 20ms block at an
                // original sample offset. All samples remain SAME known tone.
                const offset=Math.round(phase*960),rise=40*960+offset,fall=rise+4800;
                const samples=Float32Array.from({length:960},(_,i)=>{
                    const n=index*960+i,peak=n>=rise&&n<fall?matrixAudioProfile.highPeak:matrixAudioProfile.lowPeak;
                    return peak*Math.sin(2*Math.PI*(frequencies[source]*n/48000+phase));
                });
                if(!pcmIdentity(samples,source).identified)row.raw_input_unidentified++;
                const timestamp=index*20000;
                let resolve,reject;const original=new Promise((yes,no)=>{resolve=yes;reject=no;});
                pending={index,timestamp,resolve,reject};
                let timer;
                try{
                    const expired=new Promise((_,no)=>{timer=setTimeout(()=>no(Error('Original codec callback deadline exhausted')),Math.max(0,deadlineMs-performance.now()));});
                    check(codec.encode(samples,timestamp),'Original encoder refused bounded input');row.inputs++;
                    await Promise.race([original,expired]);
                }finally{clearTimeout(timer);}
            }
            check(!pending,'Original codec callback still pending');
            check(row.inputs===96&&row.encoded===96&&row.decoded===96-(omit===null?0:1),'Every submitted original packet accounted');
            check(row.raw_input_unidentified===0,'Generated source itself violated unchanged identity threshold');
            check(row.confidently_wrong===0,'Known original single source decoded as confident different source');
        }catch(error){failure.push(String(error.message).slice(0,160));}
        finally{pending=null;codec?.close();row.codec_closed=Boolean(codec);result.scenarios.push(row);}
        if(failure.length)break;
    }
    result.errors=failure;result.complete=result.scenarios.length===32&&failure.length===0
        &&result.scenarios.every(row=>row.codec_closed&&row.unreported===0);
    result.interpretation='Ambiguous or weak output remains a quality failure under current rules. This control only distinguishes decoder-produced ambiguity from a proven competing source; missing encoded input is explicit and no PLC call is made.';
    return result;
}
