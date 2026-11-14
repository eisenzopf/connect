import {matrixFrameIdentity,MATRIX_FRAME_ID_PROFILE} from './browser_matrix_frame_identity.mjs';
import {rtcDecodedVideoOwners} from './browser_matrix_rtc_decoded.mjs';
import {nativeMembership} from "../../sdk/typescript/dist/browser_native_activation.js";
import {browserMatrixRtcObservation} from './browser_matrix_rtc_observation.mjs';
import {browserMatrixPipeline} from './browser_matrix_pipeline.mjs';
import {classifySourceFrame,sourceIdentityReport} from '../video/source_identity.mjs';
import {ordinalSync} from '../video/marker_calibration.mjs';
import {AudioEnvelope,combineAudioEnvelopeSync} from '../video/audio_envelope.mjs';
import {pcmIdentity} from './browser_matrix_pcm.mjs';
export {pcmIdentity} from './browser_matrix_pcm.mjs';
import {pcmBoundary,browserMatrixRtcStages} from './browser_matrix_stages.mjs';
import {transitionCoverage,cleanBinDurations} from './media_transition_coverage.mjs';
const check=(ok,message)=>{if(!ok)throw Error(message);};

function callbackTransitionEnvelope(rows,coverage,since,original) {
    if(!coverage.transitions.length)return original;
    const reports=[];let envelope=new AudioEnvelope(),next=0;
    for(const row of rows){
        if(next<coverage.transitions.length&&row.seconds*1000>=coverage.transitions[next].resumed_sample_ms){
            reports.push(envelope.report(since));envelope=new AudioEnvelope();next++;
        }
        envelope.sample(row.seconds,row.rms);
    }
    reports.push(envelope.report(since));
    return {...original,profile:'connect/transition-segmented-audio-envelope/1',
        status:reports.every(report=>report.status==='observer-valid')?'observer-valid':'observer-invalid',
        invalid:reports.flatMap(report=>report.invalid),
        accepted_edge_seconds:reports.flatMap(report=>report.accepted_edge_seconds),
        segments:reports,original_unsegmented_observer:original,
        onset_state_crossed_transition:false,
        scope:'Original decoded RMS, split only at measured contract-bounded public handoff gaps; onset proof restarts after every gap'};
}


function decodedPcmClock(owner, native, clock) {
    check(typeof owner === 'string' && owner.length > 0, 'Original decoded PCM owner required');
    if (clock !== null && clock !== undefined) {
        check(!native && clock.profile === 'connect/rtc-decoded-pcm-clock/1'
            && clock.owner === owner && Number.isSafeInteger(clock.epoch_id) && clock.epoch_id > 0,
            'Original RTC decoded PCM clock identity required');
        return Object.freeze({profile:clock.profile, owner, epoch_id:clock.epoch_id});
    }
    if (native !== null && native !== undefined) {
        const identity = native.identity;
        check(identity?.profile === 'native-sequence-cursor/1'
            && /^[0-9a-f]{32}$/.test(identity.physical_incarnation)
            && Number.isSafeInteger(identity.connection_generation) && identity.connection_generation > 0
            && Number.isSafeInteger(identity.stream_generation) && identity.stream_generation > 0
            && Number.isSafeInteger(native.ssrc) && native.ssrc >= 0 && native.ssrc <= 0xffffffff,
            'Original native decoded PCM clock identity required');
        return Object.freeze({profile:'connect/native-decoded-pcm-clock/1', owner,
            physical_incarnation:identity.physical_incarnation,
            connection_generation:identity.connection_generation,
            stream_generation:identity.stream_generation, ssrc:native.ssrc});
    }
    return Object.freeze({profile:'connect/decoded-pcm-owner-clock/1', owner});
}

function transitionEnvelope(rows, coverage, since, original) {
    const segments = [], onsets = [], transitions = [], invalid = [], oversized = [];
    const activeSegments = new Map();
    let nextCut = 0, overflow = false, invalidInput = false;
    let previousArrival = null, tiedArrivals = 0, maximumPcmGapSeconds = 0;
    const append = (output, value, limit) => {
        if (output.length < limit) output.push(value);
        else overflow = true;
    };
    for (const row of rows) {
        while (nextCut < coverage.transitions.length
            && row.seconds * 1000 >= coverage.transitions[nextCut].resumed_sample_ms) {
            nextCut++;
        }
        if (previousArrival === row.seconds) tiedArrivals++;
        previousArrival = row.seconds;
        let segment = activeSegments.get(row.clock_key);
        if (!segment || segment.cutOrdinal !== nextCut) {
            check(segments.length < 30, 'Bounded decoded PCM owner/epoch and handoff segments');
            segment = {id:segments.length + 1, key:row.clock_key, owner:row.owner, clock:row.clock,
                first:row.timestamp, last:null, firstPosition:row.pcm_position, lastPosition:null,
                rate:row.pcm_clock_rate, firstArrival:row.seconds, lastArrival:row.seconds,
                frames:0, envelope:new AudioEnvelope(), cutOrdinal:nextCut,
                afterValidatedHandoff:nextCut > 0};
            segments.push(segment);
            activeSegments.set(row.clock_key, segment);
        }
        if (segment.lastPosition !== null) {
            maximumPcmGapSeconds = Math.max(maximumPcmGapSeconds,
                (row.pcm_position - segment.lastPosition) / segment.rate);
        }
        const decodedSeconds = (row.pcm_position - segment.firstPosition) / segment.rate;
        const before = segment.envelope.transitions.length;
        const accepted = segment.envelope.sample(decodedSeconds, row.rms);
        if (accepted) append(onsets, {row_index:row.row_index, owner:row.owner, clock:row.clock,
            segment_id:segment.id, timestamp_us:row.timestamp, pcm_position:row.pcm_position,
            pcm_clock_rate:row.pcm_clock_rate, native_rtp_timestamp:row.native?.rtpTimestamp ?? null,
            decoded_seconds:decodedSeconds,
            callback_seconds:row.seconds}, 512);
        for (const value of segment.envelope.transitions.slice(before)) {
            append(transitions, {...value, decoded_seconds:value.at, at:row.seconds,
                row_index:row.row_index, timestamp_us:row.timestamp, pcm_position:row.pcm_position,
                pcm_clock_rate:row.pcm_clock_rate, native_rtp_timestamp:row.native?.rtpTimestamp ?? null,
                segment_id:segment.id,
                owner:row.owner, clock:row.clock}, 2048);
        }
        segment.last = row.timestamp;
        segment.lastPosition = row.pcm_position;
        segment.lastArrival = row.seconds;
        segment.frames++;
    }
    const summaries = segments.map(value => {
        const report = value.envelope.report();
        overflow ||= report.overflow;
        invalidInput ||= report.invalid_input;
        for (const row of report.invalid) append(invalid, {...row, owner:value.owner, segment_id:value.id}, 64);
        for (const row of report.extended_envelopes) append(oversized, {...row, owner:value.owner, segment_id:value.id}, 64);
        return {segment_id:value.id, owner:value.owner, clock:value.clock, frames:value.frames,
            first_timestamp_us:value.first, last_timestamp_us:value.last,
            first_pcm_position:value.firstPosition, last_pcm_position:value.lastPosition, pcm_clock_rate:value.rate,
            first_callback_seconds:value.firstArrival, last_callback_seconds:value.lastArrival,
            relative_delivery_span_drift_seconds:(value.lastArrival-value.firstArrival)-(value.lastPosition-value.firstPosition)/value.rate,
            after_validated_handoff:value.afterValidatedHandoff, status:report.status};
    });
    return {...original, profile:'connect/decoded-pcm-chronology-envelope/1',
        scope:'RMS chronology follows actual native48k RTP positions or RTC worklet timestamps. Original AudioData timestamps provide diagnostics. A/V edges use exact onset-row callbacks',
        status:!overflow && !invalidInput && !invalid.length ? 'observer-valid' : 'observer-invalid',
        invalid, overflow, invalid_input:invalidInput, extended_envelopes:oversized,
        maximum_sample_gap_seconds:maximumPcmGapSeconds,
        equal_resolution_arrivals:tiedArrivals,
        accepted_onset_rows:onsets, accepted_edge_seconds:onsets.filter(row=>row.callback_seconds>=since).map(row=>row.callback_seconds),
        raw_threshold_transitions:transitions, segments:summaries,
        segment_bound:30, onset_state_crossed_owner_epoch:false, onset_state_crossed_transition:false,
        original_unsegmented_callback_observer:original,
        original_segmented_callback_observer:callbackTransitionEnvelope(rows,coverage,since,original)};
}

// Bounded metadata only: these rows are selected by the unchanged report filter.
// A witness describes a PCM position gap, not a claim of network packet loss.
function audioProvenance(rows) {
    const limit=32,previous=new Map(),gaps=[];
    let omitted=0,beforeStart=null,atStart=null,beforeEnd=null,atEnd=null;
    const snapshot=row=>{
        const native=row.native;
        const sequences=Array.isArray(native?.datagramSequences)?native.datagramSequences:[];
        return Object.freeze({owner:row.owner,clock:Object.freeze({...row.clock}),row_index:row.row_index,
            callback_seconds:row.seconds,original_timestamp_us:row.timestamp,
            pcm_position:row.pcm_position,pcm_clock_rate:row.pcm_clock_rate,rms:row.rms,
            native_ssrc:native?.ssrc??null,native_rtp_timestamp:native?.rtpTimestamp??null,
            native_rtp_sequence:native?.rtpSequence??null,
            native_datagram_sequences:Array.isArray(native?.datagramSequences)?Object.freeze(sequences.slice(0,64)):null,
            omitted_native_datagram_sequences:Math.max(0,sequences.length-64),
            source_ntp:native?.sourceNtp??null,
            source_clock:native?.sourceClock?Object.freeze({...native.sourceClock}):null});
    };
    for(const row of rows){
        const last=previous.get(row.clock_key);
        if(last && row.pcm_position-last.pcm_position>row.pcm_clock_rate*.02){
            if(gaps.length<limit)gaps.push(Object.freeze({
                pcm_step_ms:(row.pcm_position-last.pcm_position)/row.pcm_clock_rate*1000,
                previous:snapshot(last),current:snapshot(row)}));
            else omitted++;
        }
        previous.set(row.clock_key,row);
        if(row.seconds<1)beforeStart=row;else if(atStart===null)atStart=row;
        if(row.seconds<61)beforeEnd=row;else if(atEnd===null)atEnd=row;
    }
    const edge=row=>row?snapshot(row):null;
    return Object.freeze({profile:'connect/decoded-audio-provenance/1',
        scope:'Original included decoder rows; diagnostic only, no added frame credit or packet-loss claim',
        limit,omitted,gaps:Object.freeze(gaps),
        window_edges:Object.freeze({start_seconds:1,end_seconds:61,
            last_before_start:edge(beforeStart),first_at_or_after_start:edge(atStart),
            last_before_end:edge(beforeEnd),first_at_or_after_end:edge(atEnd)})});
}

// Closed metadata from an already captured row. Only duplicate events call
// this helper; no extra canvas read, pixel copy, frame credit or network claim.
function duplicateVideoSnapshot(row,index,presentation=row.presentation){
    const value={owner:row.owner,row_index:index,original_timestamp_us:row.timestamp,
        callback_seconds:row.seconds,kind:row.kind,changed:row.changed,dimensions:row.dimensions,
        native:null,presentation:null,source_frame_identity:row.sourceFrameIdentity?Object.freeze({...row.sourceFrameIdentity}):null};
    if(row.native){
        const n=row.native,sequences=n.datagramSequences??[];
        value.native=Object.freeze({profile:n.profile??null,ssrc:n.ssrc??null,
            rtp_timestamp:n.rtpTimestamp??null,rtp_sequence:n.rtpSequence??null,
            datagram_sequences:Object.freeze(sequences.slice(0,32)),
            omitted_datagram_sequences:Math.max(0,sequences.length-32)});
    }
    if(presentation){
        const fields={};
        for(const key of ['profile','element_id','callbackTime','mediaTime','presentedFrames',
            'rtpTimestamp','presentationTime','expectedDisplayTime','captureTime','receiveTime',
            'processingDuration','width','height'])if(key in presentation)fields[key]=presentation[key];
        value.presentation=Object.freeze(fields);
    }
    return Object.freeze(value);
}

// Derive identity only AFTER the caller's exact public-membership filter,
// and only over the declared quality window. Private/prewindow observations
// can never spend a public frame ID or poison a later genuine public sample.
function includedSourceVideo(rows,start,end){
    const ids=new Map(),timestamps=new Map(),duplicates=[],reuses=[];
    let omittedDuplicates=0,omittedReuses=0;
    const derived=rows.map(original=>{
        const row={...original,duplicate:false};
        if(row.seconds<start||row.seconds>=end)return row;
        if(typeof row.sourceFrameKey==='string'){
            const first=ids.get(row.sourceFrameKey);
            row.duplicate=first!==undefined;
            if(first){
                if(duplicates.length<16)duplicates.push(Object.freeze({first:duplicateVideoSnapshot(first,first.row_index),
                    current:duplicateVideoSnapshot(row,row.row_index),matching_original_row_available:true}));
                else omittedDuplicates++;
            }else ids.set(row.sourceFrameKey,row);
        }
        const key=row.owner+':v:'+row.timestamp,first=timestamps.get(key);
        if(first){
            if(reuses.length<16)reuses.push(Object.freeze({first:duplicateVideoSnapshot(first,first.row_index),
                current:duplicateVideoSnapshot(row,row.row_index),matching_original_row_available:true}));
            else omittedReuses++;
        }else timestamps.set(key,row);
        return row;
    });
    return {rows:derived,duplicates,reuses,omittedDuplicates,omittedReuses};
}

export class DecodedRoute {
    constructor(index,expected,{sourceFrameIdentity=false}={}){this.sourceFrameIdentity=sourceFrameIdentity;this.index=index;this.expected=expected;this.audio=[];this.video=[];this.envelope=new AudioEnvelope();
        this.audioBoundaries=[];this.omittedAudioBoundaries=0;this.invalid=[];this.seen=new Set();this.videoHigh=false;this.lastPixels=null;this.videoEdges=[];this.begin=null;this.ownerPixels=new Map();
        this.audioCuts=[];this.videoCuts=[];
        this.audioClocks = new Map();
        this.audioCurrentClocks = new Map();
        this.lastAudioArrival = null;
        this.audioProvenance = null;
        this.presentationOwners = new Map();
        this.presentationSamples = [];
        this.presentationRefusals = [];
        this.omittedPresentationSamples = 0;
        this.omittedPresentationRefusals = 0;
        this.videoDuplicateEvents = [];
        this.omittedVideoDuplicateEvents = 0;
    }
    start(originMs){check(this.begin===null&&Number.isFinite(originMs),'One original measurement clock');this.begin=originMs;}
    audioFrame(owner,timestamp,at,samples,codec=null,native=null,clock=null){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.audio.length<16000,'Decoded audio bound');
        check(Number.isSafeInteger(timestamp), 'Safe original decoded PCM timestamp required');
        check(Number.isFinite(at) && (this.lastAudioArrival === null || at >= this.lastAudioArrival),
            'Decoded PCM callback clock moved backward');
        const originalClock = decodedPcmClock(owner, native, clock);
        const clockKey = JSON.stringify(originalClock);
        let history = this.audioClocks.get(clockKey);
        check(!history || this.audioCurrentClocks.get(owner) === clockKey, 'Retired or stale decoded PCM epoch');
        check(history || this.audioClocks.size < 10, 'Bounded original decoded PCM clocks');
        const nativeClock = native !== null && native !== undefined;
        const pcmRate = nativeClock ? 48000 : 1000000;
        let pcmPosition = timestamp;
        if (nativeClock) {
            check(Number.isSafeInteger(native.rtpTimestamp) && native.rtpTimestamp >= 0
                && native.rtpTimestamp <= 0xffffffff, 'Original native PCM RTP timestamp required');
            const delta = history ? (native.rtpTimestamp - history.lastRtp) >>> 0 : 0;
            check(delta < 0x80000000 && (delta === 0 || delta >= 960),
                'Overlapping or backward native PCM RTP position');
            pcmPosition = history ? history.position + delta : 0;
            check(Number.isSafeInteger(pcmPosition), 'Bounded native PCM RTP position');
        } else {
            check(!history || timestamp === history.last || timestamp - history.last >= 20000,
                'Overlapping or backward decoded PCM timestamp');
        }
        const identity = pcmIdentity(samples, this.expected);
        const key = JSON.stringify([owner, 'decoded-pcm', clockKey, pcmPosition]);
        const duplicate = this.seen.has(key);
        if (!history) {
            history = {last:timestamp};
            this.audioClocks.set(clockKey, history);
        }
        history.last = timestamp;
        history.position = pcmPosition;
        if (nativeClock) history.lastRtp = native.rtpTimestamp;
        this.audioCurrentClocks.set(owner, clockKey);
        this.lastAudioArrival = at;
        this.seen.add(key);
        this.audio.push({owner,seconds,timestamp,duplicate,native,clock:originalClock,clock_key:clockKey,
            pcm_position:pcmPosition, pcm_clock_rate:pcmRate, row_index:this.audio.length,...identity});
        this.envelope.sample(seconds, identity.rms);
        // Optional private counter sees only the same already accepted public
        // row and existing identity scalars. It changes no measurement result.
        globalThis.connectOriginalCompletionGate?.audio({owner,timestamp,at,index:this.index,identity});
        if(!identity.identified||identity.rms<.03){if(this.audioBoundaries.length<32)this.audioBoundaries.push({owner,original_timestamp_us:timestamp,
            callback_seconds:seconds,identity,boundary:pcmBoundary(samples),codec});else this.omittedAudioBoundaries++;}
    }
    recordVideoPresentation(owner, timestamp, at, presentation, refusal = null) {
        if (this.begin === null) return;
        const seconds = (at - this.begin) / 1000;
        if (seconds < 0 || seconds >= 62) return;
        const rows = refusal ? this.presentationRefusals : this.presentationSamples;
        if (rows.length >= 16) {
            if (refusal) this.omittedPresentationRefusals++;
            else this.omittedPresentationSamples++;
            return;
        }
        const row = {owner, original_timestamp_us:timestamp, callback_seconds:seconds};
        for (const key of ['profile', 'element_id', 'callbackTime', 'mediaTime', 'presentedFrames',
            'rtpTimestamp', 'presentationTime', 'expectedDisplayTime', 'captureTime', 'receiveTime',
            'processingDuration', 'width', 'height']) {
            if (key in presentation) row[key] = presentation[key];
        }
        if (refusal) row.refusal = refusal;
        rows.push(Object.freeze(row));
    }
    videoFrame(owner,timestamp,at,gray,pixels,width,height,native=null,presentation=null){
        if(this.begin===null)return;const seconds=(at-this.begin)/1000;if(seconds<0||seconds>=62)return;
        check(this.video.length<10000,'Decoded Video bound');
        let key = owner + ':v:' + timestamp;
        if (presentation !== null && presentation !== undefined) {
            try {
                check(native === null || native === undefined, 'RTC presentation cannot replace native provenance');
                check(presentation.profile === 'connect/rtc-video-presentation/1'
                    && Number.isSafeInteger(presentation.element_id) && presentation.element_id > 0,
                    'RTC presentation element identity unavailable');
                check(Number.isSafeInteger(presentation.presentedFrames) && presentation.presentedFrames > 0,
                    'RTC presentation counter unavailable');
                const previous = this.presentationOwners.get(presentation.element_id);
                check(!previous || previous.owner === owner, 'RTC presentation element owner changed');
                check(!previous || presentation.presentedFrames > previous.last,
                    'RTC presentation counter repeated or moved backward');
                check(previous || this.presentationOwners.size < 10, 'Bounded RTC presentation elements');
                this.presentationOwners.set(presentation.element_id, {owner, last:presentation.presentedFrames});
                key = JSON.stringify([owner, 'rtc-presentation', presentation.element_id, presentation.presentedFrames]);
            } catch (error) {
                this.recordVideoPresentation(owner, timestamp, at, presentation, String(error.message));
                throw error;
            }
            this.recordVideoPresentation(owner, timestamp, at, presentation);
        }
        const kind=classifySourceFrame(gray);
        const identity=this.sourceFrameIdentity?matrixFrameIdentity(pixels,kind,this.expected):null;
        // One authored programme/logical source continues across physical moves.
        // Neither owner changes nor PTS changes can hide original frame reuse.
        const sourceFrameKey=identity?.status==='identified'
            ?JSON.stringify([MATRIX_FRAME_ID_PROFILE,this.expected,identity.ordinal]):null;
        const timestampKey=owner+':v:'+timestamp;
        // Exact public membership and qualification window are known only in report().
        const duplicate=this.sourceFrameIdentity
            ?false:this.seen.has(key);
        this.seen.add(this.sourceFrameIdentity?timestampKey:key);

        let changed;
        if(native!==null&&native!==undefined){
            // Native private preparation has its own physical history. It must
            // not borrow an old owner's last pixels as decoder-startup evidence.
            const previous=this.ownerPixels.get(owner);check(this.ownerPixels.has(owner)||this.ownerPixels.size<10,"Bounded original pixel histories");
            changed=previous!==undefined&&pixels.some((v,i)=>v!==previous[i]);this.ownerPixels.set(owner,pixels.slice());
        }else{
            // RTC/default observations retain the original logical-route
            // comparison across an already validated owner switch.
            changed=this.lastPixels!==null&&pixels.some((v,i)=>v!==this.lastPixels[i]);this.lastPixels=pixels.slice();
        }
        if(kind==='pulse'&&!this.videoHigh)this.videoEdges.push(seconds);this.videoHigh=kind==='pulse';
        const row={owner,seconds,timestamp,kind,duplicate,changed,native,row_index:this.video.length,sourceFrameIdentity:identity,sourceFrameKey,
            presentation:presentation ? {element_id:presentation.element_id, presentedFrames:presentation.presentedFrames} : null,
            dimensions:width===640&&height===360};
        if(duplicate){
            if(this.videoDuplicateEvents.length<16){
                // At most16 searches of the existing bounded10000-row array.
                // Reconstruct the SAME original key; no new per-frame map.
                const first=this.video.findIndex(previous=>this.sourceFrameIdentity
                    ?previous.sourceFrameKey===sourceFrameKey:(previous.presentation
                    ?JSON.stringify([previous.owner,'rtc-presentation',previous.presentation.element_id,previous.presentation.presentedFrames])
                    :previous.owner+':v:'+previous.timestamp)===key);
                this.videoDuplicateEvents.push(Object.freeze({
                    first:first<0?null:duplicateVideoSnapshot(this.video[first],first),
                    current:duplicateVideoSnapshot(row,this.video.length,presentation),
                    matching_original_row_available:first>=0}));
            }else this.omittedVideoDuplicateEvents++;
        }
        this.video.push(row);
    }
    workingState(){return {audio_rows:this.audio.length,video_rows:this.video.length,
        duplicate_keys:this.seen.size,audio_boundaries:this.audioBoundaries.length,invalid_entries:this.invalid.length,
        audio_edges:this.envelope.edges.length,audio_transitions:this.envelope.transitions.length,
        audio_oversized:this.envelope.oversized.length,envelope_invalid:this.envelope.invalid.length,
        audio_clock_epochs:this.audioClocks.size, audio_clock_owners:this.audioCurrentClocks.size,
        audio_provenance_gaps:this.audioProvenance?.gaps.length??0,
        audio_provenance_edges:this.audioProvenance?Object.values(this.audioProvenance.window_edges).filter(value=>value!==null&&typeof value==='object').length:0,
        presentation_owners:this.presentationOwners.size, presentation_samples:this.presentationSamples.length,
        presentation_refusals:this.presentationRefusals.length,
        video_frame_identity_rows:this.video.filter(row=>row.sourceFrameIdentity?.status==='identified').length,video_duplicate_events:this.videoDuplicateEvents.length, video_duplicate_events_omitted:this.omittedVideoDuplicateEvents,
        video_edges:this.videoEdges.length,audio_cuts:this.audioCuts.length,video_cuts:this.videoCuts.length,
        last_pixel_bytes:this.lastPixels?.byteLength??0,
        owner_pixel_bytes:[...this.ownerPixels.values()].reduce((sum,pixels)=>sum+pixels.byteLength,0)};}
    retireWorkingState(){
        // Captured reports alias some diagnostic arrays. Reassign each working
        // field, never truncate or mutate evidence already returned to callers.
        this.audio=[];this.video=[];this.seen=new Set();this.audioBoundaries=[];this.invalid=[];
        this.envelope=new AudioEnvelope();this.videoEdges=[];this.audioCuts=[];this.videoCuts=[];
        this.lastPixels=null;this.ownerPixels=new Map();this.omittedAudioBoundaries=0;
        this.audioClocks = new Map();
        this.audioCurrentClocks = new Map();
        this.lastAudioArrival = null;
        this.audioProvenance = null;
        this.presentationOwners = new Map();
        this.presentationSamples = [];
        this.presentationRefusals = [];
        this.omittedPresentationSamples = 0;
        this.omittedPresentationRefusals = 0;
        this.videoDuplicateEvents = [];
        this.omittedVideoDuplicateEvents = 0;
        this.videoHigh=false;this.begin=null;
    }
    report(include=()=>true,renderedRoute=null){
        const originalAudio=this.audio.filter(row=>include(row,'audio'));
        const start=1,end=61,seconds=60,inWindow=row=>row.seconds>=start&&row.seconds<end;
        const capturedVideo=this.video.filter(row=>include(row,'video'));
        const sourceVideo=this.sourceFrameIdentity?includedSourceVideo(capturedVideo,start,end):null;
        const originalVideo=sourceVideo?.rows??capturedVideo;
        const includedIndices=new Set(originalVideo.map(row=>row.row_index));
        const videoIncluded=index=>this.sourceFrameIdentity?includedIndices.has(index):originalVideo.includes(this.video[index]);
        const duplicateEvents=sourceVideo?.duplicates??this.videoDuplicateEvents;
        const timestampEvents=sourceVideo?.reuses??[];
        const markerVideo=renderedRoute?renderedRoute.video:originalVideo;
        this.audioProvenance=audioProvenance(originalAudio);
        // Rebuild only bounded numeric observations. Original callbacks, times,
        // bytes and decoded media are neither replayed nor altered.
        this.envelope=new AudioEnvelope();for(const row of originalAudio)this.envelope.sample(row.seconds,row.rms);
        this.videoEdges=[];let high=false;for(const row of markerVideo){if(row.kind==='pulse'&&!high)this.videoEdges.push(row.seconds);high=row.kind==='pulse';}
        const audio=originalAudio.filter(inWindow),video=originalVideo.filter(inWindow),identified=audio.filter(row=>row.identified&&!row.duplicate);
        const boundaries=[start,...identified.map(row=>row.seconds),end];
        const maximumGap=Math.max(...boundaries.slice(1).map((at,i)=>Math.max(0,at-boundaries[i]-(i===0?0:.02))));
        const audioCoverage=transitionCoverage(identified.map(row=>row.seconds*1000),this.audioCuts,
            {startMs:start*1000,endMs:end*1000,frameMs:20,minimumGapMs:50});
        const quantumSeen=new Set(),videoSourceSeen=new Set();let sourceOverlaps=0,videoSourceOverlaps=0,sourceIdentified=0;
        const originNtp=((BigInt(Math.round((performance.timeOrigin+this.begin)*1000))/1000000n+2208988800n)<<32n)+
            ((BigInt(Math.round((performance.timeOrigin+this.begin)*1000))%1000000n)<<32n)/1000000n;
        let sourceUnbound=0;const sourceAnchors=new Map();
        const sourceQuantumExamples = [];
        let omittedSourceQuantumExamples = 0;
        const sourceKey=row=>JSON.stringify([row.owner,row.native.identity.physical_incarnation,row.native.ssrc]);
        for(const row of originalAudio)if(row.native?.sourceClock){const key=sourceKey(row),clock=row.native.sourceClock;
            let anchors=sourceAnchors.get(key);if(!anchors){anchors=new Map();sourceAnchors.set(key,anchors);}
            const id=JSON.stringify(clock);check(anchors.has(id)||anchors.size<512,'Bounded original source clock anchors');anchors.set(id,clock);
        }
        for(const row of audio)if(row.native){
            const anchors=sourceAnchors.get(sourceKey(row))?.values()??[];
            const quanta=new Set();
            let anchorCount = 0, eligibleAnchors = 0;
            for(const clock of anchors){const delta=(row.native.rtpTimestamp-clock.rtpTimestamp)|0;
                anchorCount++;
                if(delta===-2147483648||Math.abs(delta)>3*48000)continue;
                eligibleAnchors++;
                const denominator=960n*(1n<<32n),num=(BigInt(clock.ntp)-originNtp)*48000n+BigInt(delta)*(1n<<32n);
                const rounded=num+denominator/2n;const quantum=Number(rounded>=0n?rounded/denominator:-((-rounded+denominator-1n)/denominator));if(quantum>=0&&quantum<3100)quanta.add(quantum);
            }
            if (quanta.size !== 1) {
                sourceUnbound++;
                if (sourceQuantumExamples.length < 16) {
                    const candidates = [...quanta];
                    sourceQuantumExamples.push({reason:quanta.size === 0 ? 'unbound' : 'ambiguous',
                        row_index:row.row_index, owner:row.owner, original_timestamp_us:row.timestamp,
                        callback_seconds:row.seconds, rtp_timestamp:row.native.rtpTimestamp, ssrc:row.native.ssrc,
                        identity:{...row.native.identity},
                        current_source_clock:row.native.sourceClock ? {...row.native.sourceClock} : null,
                        origin_ntp:originNtp.toString(), anchor_count:anchorCount, eligible_anchor_count:eligibleAnchors,
                        candidate_quantum_count:quanta.size, candidate_quanta:candidates.slice(0,16),
                        omitted_candidate_quanta:Math.max(0,candidates.length-16)});
                } else omittedSourceQuantumExamples++;
                continue;
            }
            const quantum=[...quanta][0];sourceIdentified++;
            if(quantumSeen.has(quantum))sourceOverlaps++;quantumSeen.add(quantum);
        }
        // The frozen source video clock retains its frame timestamp across
        // physical moves. Quantized callback times are never duplicate keys.
        for(const row of video)if(row.native){const key=row.native.rtpTimestamp;if(videoSourceSeen.has(key))videoSourceOverlaps++;videoSourceSeen.add(key);}
        const counts={};for(const row of video)counts[row.kind]=(counts[row.kind]??0)+1;
        const validFrameId=row=>!this.sourceFrameIdentity||row.sourceFrameIdentity?.status==='identified';
        const perSecond=Array(60).fill(0);for(const row of video)if(validFrameId(row)&&row.changed&&!row.duplicate&&row.dimensions&&
            [String(this.expected),'pulse'].includes(row.kind))perSecond[Math.floor(row.seconds-start)]++;
        const changingVideo=video.filter(row=>validFrameId(row)&&row.changed&&!row.duplicate&&row.dimensions&&[String(this.expected),'pulse'].includes(row.kind));
        const videoCoverage=transitionCoverage(changingVideo.map(row=>row.seconds*1000),this.videoCuts,
            {startMs:start*1000,endMs:end*1000,frameMs:1000/30,minimumGapMs:50});
        const requiredFrames=cleanBinDurations(videoCoverage,start*1000,seconds).map(ms=>Math.ceil(ms*24/1000-1e-9));
        const identity=sourceIdentityReport(counts,this.expected);
        const envelope=transitionEnvelope(originalAudio,audioCoverage,start,this.envelope.report(start));
        // Chrome direct decoded pixels qualify FPS; its independent rendered
        // pixels retain the ORIGINAL marker onset/transition clock for A/V.
        const markerCoverage=renderedRoute?transitionCoverage(markerVideo.filter(row=>inWindow(row)&&row.changed&&!row.duplicate&&row.dimensions&&[String(this.expected),'pulse'].includes(row.kind)).map(row=>row.seconds*1000),renderedRoute.videoCuts,
            {startMs:start*1000,endMs:end*1000,frameMs:1000/30,minimumGapMs:50}):videoCoverage;
        if(markerCoverage.transitions.length){
            this.videoEdges=[];let high=null,next=0;
            for(const row of markerVideo){
                if(next<markerCoverage.transitions.length&&row.seconds*1000>=markerCoverage.transitions[next].resumed_sample_ms){high=null;next++;}
                if(row.kind==='pulse'&&high===false)this.videoEdges.push(row.seconds);
                high=row.kind==='pulse';
            }
        }
        const sync=combineAudioEnvelopeSync(ordinalSync(this.videoEdges.filter(at=>at>=start&&at<end),
            envelope.accepted_edge_seconds.filter(at=>at<end),seconds),envelope);
        const audioResult={quality_profile:'connect/decoded-audio-quality/3',
            source_quantum_scope:'diagnostic-only-source-clock-attribution',
            metric_scope:'measurable-decoded-quality-source-quanta-diagnostic',
            original_source_sample_coverage_proved:false,
            source_quantum_diagnostics:{limit:16, events:sourceQuantumExamples, omitted:omittedSourceQuantumExamples,
                scope:'Estimated source quanta provide clock attribution. Decoded quality uses the declared measurable scope'},
            decoded_seconds:audio.length*.02,identified_seconds:identified.length*.02,
            maximum_gap_ms:maximumGap*1000,maximum_pcm_sample_gap_ms:envelope.maximum_sample_gap_seconds*1000,
            bad_identity_frames:audio.filter(row=>!row.identified).length,
            duplicate_frames:audio.filter(row=>row.duplicate).length,original_source_quantum_overlaps:sourceOverlaps,source_quantum_identity_available:sourceIdentified,source_quantum_identity_unbound_or_ambiguous:sourceUnbound,
            transition_coverage:audioCoverage,
            passed:identified.length*.02>=59.9-audioCoverage.transition_missing_ms/1000
                &&audioCoverage.maximum_clean_gap_ms<=220&&envelope.maximum_sample_gap_seconds<=.22&&identified.length>0&&identified[0].seconds-start<=.2
                &&audio.every(row=>row.identified&&!row.duplicate)};
        const decodedPerSecond=Array(60).fill(0);for(const row of video)decodedPerSecond[Math.floor(row.seconds-start)]++;
        const callbackTimes=[start,...video.map(row=>row.seconds),end];
        const renderedCounts={};if(renderedRoute)for(const row of markerVideo.filter(inWindow))renderedCounts[row.kind]=(renderedCounts[row.kind]??0)+1;
        const renderedIdentity=renderedRoute?sourceIdentityReport(renderedCounts,this.expected):null;
        const presentationRoute=renderedRoute??this;
        const videoResult={observer_profile:renderedRoute?'native-decoded-frames/1':(this.presentationOwners.size?'render-callback/1':'original-decoder-output'),
            ...(renderedRoute?{rendered_observation:{profile:'render-callback/1',scope:'Independent actual rendered pixels and original marker onsets; no additional FPS credit',
                observed_frames:markerVideo.filter(inWindow).length,identity:renderedIdentity,transition_coverage:markerCoverage,
                invalid:renderedRoute.invalid,passed:renderedIdentity.status==='passed'&&!renderedRoute.invalid.length
                    &&markerVideo.filter(inWindow).every(row=>row.dimensions&&!row.duplicate)}}:{}),
            presentation_identity_diagnostics:{
                scope:'Actual per-element RTC presentation metadata identifies observed callbacks. Frame credit counts each observed presentation once',
                limit:16, samples:presentationRoute.presentationSamples.slice(), refusals:presentationRoute.presentationRefusals.slice(),
                omitted_samples:presentationRoute.omittedPresentationSamples, omitted_refusals:presentationRoute.omittedPresentationRefusals},
            decoded_frames_per_second:decodedPerSecond,
            maximum_decode_callback_gap_ms:Math.max(...callbackTimes.slice(1).map((at,i)=>Math.max(0,at-callbackTimes[i])))*1000,
            decoded_frames:video.length,changing_frames_per_second:perSecond,minimum_fps:Math.min(...perSecond),
            identity,dimension_refusals:video.filter(row=>!row.dimensions).length,original_source_timestamp_overlaps:videoSourceOverlaps,duplicate_frames:video.filter(row=>row.duplicate).length,
            duplicate_diagnostics:Object.freeze({profile:'connect/decoded-video-duplicate-evidence/1',limit:16,
                scope:this.sourceFrameIdentity?'Repeated decoded source ordinal among exact included public quality-window rows; no inferred transport cause':'Repeated original observer identity only; no packet-duplicate cause, pixel equality, additional frame credit or quality exception',
                source_frame_bits:this.sourceFrameIdentity?MATRIX_FRAME_ID_PROFILE:null,
                source_frame_bits_unavailable:this.sourceFrameIdentity?null:'Generic decoded routes have no declared source-bit layout',
                omitted:sourceVideo?.omittedDuplicates??this.omittedVideoDuplicateEvents,
                events:Object.freeze(duplicateEvents.filter(event=>videoIncluded(event.current.row_index)
                    &&(!event.first||videoIncluded(event.first.row_index))))}),
            source_frame_identity:{profile:this.sourceFrameIdentity?MATRIX_FRAME_ID_PROFILE:null,
                scope:'Original decoded footer ordinal; expected source is logical-route association. White pulses retain the declared source-identity exception, never an invented pixel-source ID',
                status:!this.sourceFrameIdentity?'not-requested':video.length>0&&video.every(validFrameId)?'passed':'failed',
                identified_frames:video.filter(row=>row.sourceFrameIdentity?.status==='identified').length,
                unavailable_frames:video.filter(row=>this.sourceFrameIdentity&&!validFrameId(row)).length,
                pulse_exception_frames:video.filter(row=>row.sourceFrameIdentity?.pulse_exception).length},
            timestamp_reuse_diagnostics:{profile:'connect/decoded-video-timestamp-reuse/1',limit:16,
                scope:'Repeated browser/decoder timestamp only; source-frame duplicates remain independently mandatory',
                omitted:sourceVideo?.omittedReuses??0,events:Object.freeze(timestampEvents.filter(event=>
                    videoIncluded(event.current.row_index)&&(!event.first||videoIncluded(event.first.row_index))))},
            transition_coverage:videoCoverage,required_changing_frames_per_second:requiredFrames,
            passed:video.every(validFrameId)&&videoSourceOverlaps===0&&perSecond.every((count,index)=>count>=requiredFrames[index])
                &&changingVideo.length>=1440&&identity.status==='passed'&&video.every(row=>row.dimensions&&!row.duplicate)
                &&(!renderedRoute||(renderedIdentity.status==='passed'&&!renderedRoute.invalid.length&&markerVideo.filter(inWindow).every(row=>row.dimensions&&!row.duplicate)))};
        return {route:this.index,expected_source_id:this.expected,window_start_ms:1000,window_end_ms:61000,
            audio:audioResult,video:videoResult,sync,invalid:this.invalid,
            audio_provenance_diagnostics:this.audioProvenance,
            audio_boundary_diagnostics:{scope:'Actual low-RMS or unidentified PCM; diagnostic only, no quality exception',limit:32,
                omitted:this.omittedAudioBoundaries,events:this.audioBoundaries},
            passed:!this.invalid.length&&audioResult.passed&&videoResult.passed&&sync.status==='passed'};
    }
}

/** Fixture-owned observer of actual borrowed decoder outputs. The public render
 * owner still receives every frame normally. Copies contain only this run's
 * synthetic media; no borrowed frame is retained or closed by the observer. */
export function browserMatrixObservation(Renderer,plan,Peer,Rtc,{webTransportStatistics=null,rtcVideoObserver='render-callback/1'}={}) {
    check(!Rtc||['native-decoded-frames/1','render-callback/1'].includes(rtcVideoObserver),'Declared RTC decoded video observer required');
    const nativeRtcVideo=Boolean(Rtc&&rtcVideoObserver==='native-decoded-frames/1');
    const routes=plan.routes.map(route=>{
        const peers=plan.routes.filter(other=>other.session===route.session),at=peers.indexOf(route);
        const selected=peers[(at+peers.length-1)%peers.length];
        return {measure:new DecodedRoute(route.index,Number(selected.role.split('-').at(-1))-1,{sourceFrameIdentity:true}),
            rendered:nativeRtcVideo?new DecodedRoute(route.index,Number(selected.role.split('-').at(-1))-1,{sourceFrameIdentity:true}):null,selected,active:null};
    });
    const bindings=new Map(),owners=[],surfaces=new Set(),pendingRenders=new Set(),initialRenders=new Map(),originalAudio=Renderer.prototype.audio,originalVideo=Renderer.prototype.video;
    let closed=false,closing,lastReport,measurementsRetired=false;const rtcStages=Rtc?browserMatrixRtcStages():null;
    const pipeline=Peer?browserMatrixPipeline(Peer,(connection,kind,stream)=>{
        const owner=bindings.get(connection);
        return owner&&routes[owner.index].active===connection&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;
    },(connection,kind,stream)=>{const owner=bindings.get(connection);
        return owner&&stream===(kind==='audio'?owner.audio:owner.sendVideo)?owner:null;}):null;
    function observe(renderer,binding,frame,kind,native){
        const owner=bindings.get(binding.connectionId);if(closed||!owner)return;
        if(Rtc && routes[owner.index].active!==binding.connectionId)return;
        if(!Rtc && !owner.initial && !owner.activation && !owner.readActivation)owner.readActivation=()=>renderer.nativeActivationBoundary;
        if(binding.streamId!==owner[kind])return;
        const route=routes[owner.index].measure,at=performance.now();
        try{
            if(kind==='audio'){
                check(frame.sampleRate===48000&&frame.numberOfChannels===1&&frame.numberOfFrames===960,'Decoded audio format');
                const samples=new Float32Array(960);frame.copyTo(samples,{planeIndex:0,format:'f32-planar'});
                route.audioFrame(binding.connectionId,frame.timestamp,at,samples,pipeline?.snapshot(binding.connectionId)??null,native);
            }else{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                route.videoFrame(binding.connectionId,frame.timestamp,at,gray,pixels,frame.displayWidth,frame.displayHeight,native);
            }
        }catch(error){if(route.invalid.length<16)route.invalid.push(String(error.message));}
    }
    const invalidRtc=(connection,message,presentation)=>{const owner=bindings.get(connection);if(!owner)return;
        const route=routes[owner.index].measure;
        if (presentation) route.recordVideoPresentation(connection,
            Math.round(presentation.mediaTime * 1000000), performance.now(), presentation, message);
        if(route.invalid.length<16)route.invalid.push(message);};
    const rtc=Rtc?browserMatrixRtcObservation(Rtc,{
        measurementOrigin(connection){
            const owner=bindings.get(connection);
            return !closed&&owner&&routes[owner.index].active===connection?routes[owner.index].measure.begin:null;
        },
        audio(connection,timestamp,at,samples,clock){
            const owner=bindings.get(connection);if(closed||!owner||routes[owner.index].active!==connection)return;
            try{routes[owner.index].measure.audioFrame(connection,timestamp,at,samples,null,null,clock);}
            catch(error){invalidRtc(connection,String(error.message));}
        },
        video(connection,timestamp,at,frame,width,height,presentation){
            const owner=bindings.get(connection);if(closed||!owner||routes[owner.index].active!==connection)return;
            try{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                (routes[owner.index].rendered??routes[owner.index].measure).videoFrame(connection,timestamp,at,gray,pixels,width,height,null,presentation);
            }catch(error){invalidRtc(connection,String(error.message));}
        },
        videoObserver:rtcVideoObserver,decodedVideoOwnerFactory:rtcDecodedVideoOwners,
        decodedVideo(connection,timestamp,at,frame,width,height){
            const owner=bindings.get(connection);if(closed||!owner||routes[owner.index].active!==connection)return;
            try{
                const context=owner.context;context.drawImage(frame,0,0,8,8);const bytes=context.getImageData(0,0,8,8).data;
                const gray=Array.from({length:64},(_,i)=>.299*bytes[i*4]+.587*bytes[i*4+1]+.114*bytes[i*4+2]);
                context.drawImage(frame,0,0,128,72);const pixels=context.getImageData(0,0,128,72).data;
                routes[owner.index].measure.videoFrame(connection,timestamp,at,gray,pixels,width,height,null,null);
            }catch(error){invalidRtc(connection,String(error.message));}
        },
        invalid:invalidRtc,
    }):null;
    Renderer.prototype.audio=function(binding,frame,native){observe(this,binding,frame,'audio',native);return originalAudio.call(this,binding,frame,native);};
    Renderer.prototype.video=function(binding,frame,native){observe(this,binding,frame,'video',native);return originalVideo.call(this,binding,frame,native);};
    function render(binding,connection,options){
        const work=(async()=>{
            const output=binding.surfaces[1];rtc?.attach(connection.id,output);
            const owner=await connection.render({audio:true,video:[{stream:connection.streams.byId(binding.video),canvas:output}]},options);
            owners.push({owner,index:binding.index,connectionId:connection.id,finished:owner.finished,retirement:owner.retirement,
                surfaces:binding.surfaces});
            binding.pendingConnection=undefined;
            if(closed)owner.close();
        })();
        pendingRenders.add(work);
        void work.then(()=>pendingRenders.delete(work),()=>pendingRenders.delete(work));
        return work;
    }
    return {
        async attach(index,connection,{audio,video,sendVideo,active=false},options){
            check(!closed&&!bindings.has(connection.id)&&bindings.size<10,'Bounded original render owners');
            const canvas=document.createElement('canvas');canvas.width=128;canvas.height=72;
            const context=canvas.getContext('2d',{willReadFrequently:true});context.imageSmoothingEnabled=false;
            surfaces.add(canvas);
            const output=document.createElement('canvas');surfaces.add(output);
            const binding={index,audio,video,sendVideo,context,initial:active,surfaces:[canvas,output],
                sourceId:Number(plan.routes[index].role.split('-').at(-1))-1};
            bindings.set(connection.id,binding);
            if(active)routes[index].active=connection.id;
            if(Rtc&&!active){
                // The SDK owns private decoder readiness. The fixture creates
                // public playback only after validating both original receipts.
                binding.pendingConnection=connection;
                return;
            }
            await render(binding,connection,options);
        },
        startInitialConnection(index,connection,options){
            const route=plan.routes[index];
            check(Rtc&&!closed&&route?.index===index&&connection?.id===route.connection,
                'Exact original planned RTC render Connection required');
            const existing=initialRenders.get(connection.id);
            if(existing){
                check(existing.index===index&&existing.connection===connection,
                    'Original initial render owner changed');
                return;
            }
            const failed=[...initialRenders.values()].find(value=>value.failure);
            if(failed)throw failed.failure;
            const selected=routes[index].selected;
            const entry={index,connection,failure:null,work:null};
            // Start the original render invocation now. Its SDK readiness and
            // play promise may need a later source; retain the work instead of
            // serializing offers. The final all-six barrier awaits this work.
            entry.work=this.attach(index,connection,{audio:route.audio,video:Rtc?route.video:selected.video,
                sendVideo:route.video,active:true},options);
            initialRenders.set(connection.id,entry);
            void entry.work.catch(error=>{entry.failure=error;});
        },
        async initial(state,options){
            if(!Rtc){
                for(const route of plan.routes){const selected=routes[route.index].selected;
                    await this.attach(route.index,state.connections[route.index],{audio:route.audio,
                        video:selected.video,sendVideo:route.video,active:true},options);
                }
                return;
            }
            let startupFailure;
            try{
                for(const route of plan.routes)
                    this.startInitialConnection(route.index,state.connections[route.index],options);
            }catch(error){startupFailure=error;}
            // Even an earlier refusal cannot abandon another original render
            // already waiting for real media under its original startup budget.
            const starts=[...initialRenders.values()];
            const results=await Promise.allSettled(starts.map(value=>value.work));
            if(startupFailure)throw startupFailure;
            const refused=results.find(value=>value.status==='rejected');
            if(refused)throw refused.reason;
            check(starts.length===plan.routes.length,'All six original initial render owners required');
            for(const entry of starts){
                const binding=bindings.get(entry.connection.id);
                check(binding?.initial&&binding.index===entry.index&&routes[entry.index].active===entry.connection.id
                    &&owners.some(value=>value.connectionId===entry.connection.id&&value.index===entry.index
                        &&value.owner.diagnostics?.closed!==true),
                    'Original initial renderer must have completed startup and remain owned');
            }
        },
        start(origin){pipeline?.start(origin.originMs);rtcStages?.start(origin.originMs);webTransportStatistics?.start(origin.originMs);for(const route of routes){route.measure.start(origin.originMs);route.rendered?.start(origin.originMs);}},
        activate(index,connectionId,options){const owner=bindings.get(connectionId);check(!closed&&owner?.index===index,'Original prepared observer required');
            const recordCut=()=>{
                const measure=routes[index].measure;if(measure.begin===null)return;
                const cut={id:connectionId,atMs:performance.now()-measure.begin,
                    basis:Rtc?'validated-receipts-local-public-render-cut':'validated-native-public-cut-observed'};
                for(const route of routes){
                    if(plan.routes[route.measure.index].session!==plan.routes[index].session)continue;
                    check(route.measure.audioCuts.length<2,'Bounded original Session handoffs');
                    route.measure.audioCuts.push(cut);
                    if(route.measure.index===index||route.selected.index===index){route.measure.videoCuts.push(cut);route.rendered?.videoCuts.push(cut);}
                }
            };
            if(!Rtc){const boundary=owner.readActivation?.();owner.readActivation=undefined;
                check(!owner.activation,'One original public native observer activation');
                check(boundary?.audience?.scope==='public'&&boundary.target_connection_id===connectionId,'Original completed public native cut required');owner.activation=boundary;
                recordCut();routes[index].active=connectionId;return;}
            check(owner.pendingConnection&&!owner.activating,'One original public RTC renderer activation');
            owner.activating=true;
            // This is the known local public-render cut, after signed receipt
            // validation. Select before startup so its first actual output is
            // observed even while render() is still resolving.
            recordCut();routes[index].active=connectionId;
            return render(owner,owner.pendingConnection,options).catch(error=>{
                if(routes[index].measure.invalid.length<16)routes[index].measure.invalid.push('Original public RTC render activation failed');
                throw error;
            });},
        report(){if(measurementsRetired)return lastReport;const measured=routes.map(route=>route.measure.report((row,kind)=>{
                if(Rtc)return true;const owner=bindings.get(row.owner);
                try{check(owner,'Original native output owner missing');check(row.native,'Original native decoded datagram provenance missing');
                    if(owner.initial)return true;
                    check(owner.activation?.audience?.scope==='public','Prepared owner has no completed public cut');
                    const cut=owner.activation.audience.streams.find(c=>c.stream_id===owner[kind]);check(cut,'Original native stream cut missing');
                    return nativeMembership(cut,row.native)==='public';
                }catch(error){if(route.measure.invalid.length<16)route.measure.invalid.push(String(error.message));return false;}
            },route.rendered));const nativeVideo=rtc?.decodedVideoReport()??null;
            const decodedVideoComplete=!nativeRtcVideo||(nativeVideo?.owners.length>=6&&nativeVideo.owners.every(owner=>owner.frames>0&&!owner.failed));
            return lastReport=structuredClone({format:Rtc?'connect/browser-matrix-decoded-media/1':'connect/browser-matrix-decoded-media/2',
            quality_profile:'connect/decoded-audio-quality/3',
            metric_scope:'measurable-decoded-quality-source-quanta-diagnostic',
            original_source_sample_coverage_proved:false,
            native_activation: Rtc?null:[...bindings].filter(([,owner])=>!owner.initial).map(([connection,owner])=>({connection,boundary:owner.activation??null})),
            video_observer:Rtc?rtcVideoObserver:'original-decoder-output',native_decoded_observer:nativeVideo,
            method:nativeRtcVideo?'Independent public RTC PCM and borrowed received-track decoded video pixels; independent rendered A/V marker pixels retain original onset callbacks and receipt-bound public cuts; no stats-as-frame or physical playback claim':Rtc?'Independent content measurements on actual public WebRTC render-graph PCM and video pixels, started after original completion receipt validation at a known local renderer cut; no server RTP-cut or physical playback claim':
                'Independent content measurements on actual WebCodecs decoded PCM/pixels at local receive callback times; no physical playback claim',
            window_ms:60000,source_counters_used:false,rtc_media_pipeline:rtcStages?.report()??null,
            webtransport_statistics:webTransportStatistics?.report()??null,
            rtc_audio_boundary_diagnostics:rtc?.report()??null,
            pipeline:pipeline?.report(routes.map((route,index)=>({index,source:route.selected.index,
                audio:route.measure.envelope.edges,audioEnvelope:route.measure.envelope.report(1),video:route.measure.videoEdges}))),passed:decodedVideoComplete&&measured.every(route=>route.passed),routes:measured});},
        // Stop and join the original diagnostic read while its peer is still
        // live. The same bounded receipt is reused by close; pending work never
        // becomes a synthetic join after peer retirement.
        stopStatistics(){return rtcStages?.close();},
        close(){return closing??=(async()=>{closed=true;
            // New borrowed-frame owners always start retirement, even if an
            // unrelated original renderer/audio cleanup fails. Their original
            // read/clone joins are awaited in finally, never abandoned.
            const decodedVideoWork=rtc?.stopDecodedVideo();
            try{
            pipeline?.close();Renderer.prototype.audio=originalAudio;Renderer.prototype.video=originalVideo;
            // Stop scheduling diagnostics at original close, before joining
            // render owners. A pending original getStats is never a fake join.
            const statisticsWork=rtcStages?.close(),webTransportStatisticsWork=webTransportStatistics?.close();
            await Promise.allSettled([...pendingRenders]);
            for(const {owner} of owners)owner.close();
            const finishedWork=Promise.allSettled(owners.map(entry=>entry.finished));
            // Retain each exact original promise at attach. A replacement
            // property or another owner's same-ID receipt proves no cleanup.
            const retirements=await Promise.allSettled(owners.map(async entry=>{
                check(entry.retirement&&typeof entry.retirement.then==='function','Original renderer retirement promise required');
                const receipt=await entry.retirement;
                check(receipt&&Object.isFrozen(receipt)&&receipt.connectionId===entry.connectionId
                    &&receipt.scope==='renderer-local'&&receipt.outputsDetached===true
                    &&['closed','not-created'].includes(receipt.audioGraph),'Original renderer retirement receipt differs');
                return receipt;
            }));
            const rtcClosure=await rtc?.close();const statistics=await statisticsWork,webTransportStatisticsClosure=await webTransportStatisticsWork;
            const diagnosticOwnersJoined=(!rtcStages||statistics?.joined===true)
                &&(!webTransportStatistics||webTransportStatisticsClosure?.joined===true)
                &&(!nativeRtcVideo||rtcClosure?.decoded_video?.joined===true);
            if(diagnosticOwnersJoined)retirements.forEach((result,index)=>{
                if(result.status!=='fulfilled')return;
                for(const canvas of owners[index].surfaces){canvas.width=0;canvas.height=0;surfaces.delete(canvas);}
            });
            // These prepared bindings never created a renderer or a borrower.
            // They cannot supply a fabricated renderer-retirement receipt.
            for(const binding of bindings.values())if(binding.pendingConnection&&!binding.activating){
                binding.pendingConnection=undefined;
                for(const canvas of binding.surfaces){canvas.width=0;canvas.height=0;surfaces.delete(canvas);}
            }
            const outcomes=await finishedWork,bounded=value=>typeof value==='string'?value.slice(0,64):null;
            const details=(outcome,ordinal,stage)=>({owner_index:ordinal,route_index:owners[ordinal].index,
                connection_id:owners[ordinal].connectionId,status:outcome.status,stage,
                ...(outcome.status==='rejected'?{name:bounded(outcome.reason?.name),code:bounded(outcome.reason?.code),kind:bounded(outcome.reason?.kind),
                    certainty:bounded(outcome.reason?.certainty)}:{})});
            const ownerOutcomes=outcomes.map((outcome,index)=>details(outcome,index,'original-render-finished'));
            const retirementOutcomes=retirements.map((outcome,index)=>details(outcome,index,'original-render-retirement'));
            const count=owners.length,workingState=()=>{
                const total={};for(const route of routes)for(const [key,value] of Object.entries(route.measure.workingState()))
                    total[key]=(total[key]??0)+value;
                for(const route of routes)if(route.rendered)for(const [key,value] of Object.entries(route.rendered.workingState()))
                    total['rendered_'+key]=(total['rendered_'+key]??0)+value;
                return total;
            };
            const before=workingState();
            const playbackPassed=outcomes.every(value=>value.status==='fulfilled')&&(!nativeRtcVideo||rtcClosure?.decoded_video?.passed===true);
            const joined=retirements.every(value=>value.status==='fulfilled')&&diagnosticOwnersJoined&&surfaces.size===0;
            if(joined){
                // Preserve the most recent complete proof before dropping only
                // no-longer-needed numeric measurement working histories.
                lastReport=this.report();
                for(const route of routes){route.measure.retireWorkingState();route.rendered?.retireWorkingState();}
                measurementsRetired=true;
            }
            owners.length=0;bindings.clear();initialRenders.clear();
            return {owners:count,statistics,webtransport_statistics:webTransportStatisticsClosure,native_decoded_observer:rtcClosure?.decoded_video??null,joined,playback_passed:playbackPassed,
                measurement_working_state:{retired:joined,before,retained:workingState(),
                    scope:'Decoded-route working histories only. Captured report evidence remains unchanged. No garbage collection or RSS return claim.'},
                owner_outcomes:ownerOutcomes,retirement_outcomes:retirementOutcomes,retained_surfaces:surfaces.size};
            }finally{await decodedVideoWork;}
        })();},
    };
}
