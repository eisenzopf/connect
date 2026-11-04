import {AudioEnvelope} from '../video/audio_envelope.mjs';

/** Passive encoder-input attribution. Sample continuity uses the original
 * capture clock; marker arrival uses the unmodified local observation time.
 * Equal-resolution callback times are not duplicate PCM. No timestamp, PCM,
 * decoded observer or qualification result is changed here. */
export class MatrixAudioInput {
    constructor(){this.owners=new Map();this.edges=[];this.invalid=[];this.lastArrival=null;this.tiedArrivals=0;}
    fail(reason){if(this.invalid.length<16)this.invalid.push(reason);}
    sample(owner,timestampUs,at,rms){
        if(typeof owner!=='string'||!owner||!Number.isSafeInteger(timestampUs)||timestampUs<0||
            !Number.isFinite(at)||at<0||!Number.isFinite(rms)||rms<0){this.fail('Invalid original capture observation');return;}
        if(this.lastArrival!==null&&at<this.lastArrival){this.fail('Arrival clock moved backwards');return;}
        if(at===this.lastArrival)this.tiedArrivals++;
        this.lastArrival=at;
        let record=this.owners.get(owner);
        if(!record){
            if(this.owners.size>=3){this.fail('Original route exceeded three physical capture owners');return;}
            record={envelope:new AudioEnvelope(),first:timestampUs,last:null,frames:0,gaps:0,missingUs:0};
            this.owners.set(owner,record);
        }
        // Each worklet frame contains exactly 960 samples at 48 kHz. Preserve
        // real discontinuities; never fill a gap or carry low/rearm state into
        // another physical capture owner after replacement.
        if(record.last!==null){
            const delta=timestampUs-record.last;
            if(delta<20000){this.fail('Duplicate, overlapping or backwards capture PCM');return;}
            if(delta>20000){record.gaps++;record.missingUs+=delta-20000;}
        }
        record.last=timestampUs;record.frames++;
        if(record.envelope.sample((timestampUs-record.first)/1e6,rms)){
            if(this.edges.length>=512)this.fail('Original input marker bound exceeded');
            else this.edges.push(at);
        }
    }
    report(){
        const owners=[...this.owners.values()].map(record=>{
            const envelope=record.envelope.report();
            return {frames:record.frames,capture_span_us:record.last-record.first,
                capture_gap_count:record.gaps,missing_capture_us:record.missingUs,
                envelope_status:envelope.status,maximum_sample_gap_seconds:envelope.maximum_sample_gap_seconds,
                invalid:envelope.invalid,overflow:envelope.overflow,invalid_input:envelope.invalid_input};
        });
        return {status:owners.length&&!this.invalid.length&&owners.every(owner=>owner.envelope_status==='observer-valid')?
            'observer-valid':'observer-invalid',profile:'connect/original-audio-input/1',
            scope:'Input content/arrival attribution only; no decoded media or quality pass',
            equal_resolution_arrivals:this.tiedArrivals,owners,invalid:[...this.invalid],accepted_edge_seconds:[...this.edges]};
    }
}
