import {nativeIdentity,bindNativeStreams,nativeBoundary} from '../../sdk/tests/native_activation_fixture.mjs';
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { physicalAvSegment } from "../lib/transport_replacement_observation.mjs";
import { DecodedRoute } from "../lib/browser_matrix_observation.mjs";
import { encodeRtcpSenderReport } from "../../sdk/typescript/dist/browser_rtcp.js";

const expected = { programStartUnixUs: 1_800_000_000_000_000, connectionId: "candidate", targetGeneration: 2,
    operationId:"move",sourceConnectionId:"old",audioStreamId: "own-audio", sendVideoStreamId: "own-video", receiveVideoStreamId: "remote-video" };
function fixture(received) {
    const binding = (strm_id, kind, direction, stream_local_id) => ({ strm_id, kind, direction, stream_local_id,
        codec: { name: kind === "audio" ? "opus" : "vp8", params: {
            sample_rate: kind === "audio" ? 48000 : 90000, channels: kind === "audio" ? 1 : 0,
            payload_type: kind === "audio" ? 111 : 96 } } });
    const streams=bindNativeStreams([binding("remote-video","video","sendonly",9),binding("own-video","video","recvonly",8),binding("own-audio","audio","sendrecv",7)],2);
    const floors={};for(const s of streams.filter(s=>s.native_egress)){
      const records=received.filter(r=>new DataView(Uint8Array.from(r.datagram).buffer).getUint16(2)===s.stream_local_id);
      // Model constructors assign each original outer reservation. Preserve
      // those bytes, including a tagged epoch's signed first_sequence.
      const sequence=r=>new DataView(Uint8Array.from(r.datagram).buffer).getUint32(4);
      const firstPublic=records.find(r=>r.original_root_completed_observed);
      floors[s.strm_id]=firstPublic?sequence(firstPublic):records.length?Math.max(...records.map(sequence))+1:0;
    }
    const boundary=nativeBoundary({recordId:'record',operationId:expected.operationId,sourceConnectionId:expected.sourceConnectionId,targetConnectionId:expected.connectionId},streams,2,3,floors);
    return { format: "connect/native-replacement-peer-observation/1", passed: true,
        original_client_joined: true, video_enabled: true, original_root_completed_observed: true,
        video_transfer_admitted: true, operation_completed_inferred: false, decoded_media_verified: false,
        program_start_unix_us: expected.programStartUnixUs, private_decoder_started_programme_us: 0,
        // Parser fixtures model the shape of a separately bound decoder receipt;
        // these synthetic rows are not actual codec or cell qualification.
        private_decoded_readiness: { profile: "connect/native-private-decoded-readiness/1",
            operation_id:expected.operationId,record_id:"record",native_activation:boundary,connection_id: expected.connectionId, source_generation: 1, target_generation: 2, stream_revision: 2,
            readiness_frozen: true, decoder_reset_on_root_completion: false,
            frozen_private_observations: { audio_decoded_samples: 2880, audio_ssrc: 12,
                video_decoded_frames: 1, video_decoded_keyframes: 1, video_epoch: null },
            decoder_process: { joined: true, clean: true, forced: false, exit_code: 0 }, video_decode_receipts: [] },
        root_completed_observed_programme_us: received.find(row => row.original_root_completed_observed)?.elapsed_us ?? 0,
        negotiated_streams: streams, received };
}
function rtp({ handle = 7, pt = 111, ssrc = 12, timestamp = 960, sequence = 1, outer = sequence,
    marker = true, payload = [1, 2] } = {}) {
    const b = new Uint8Array(20 + payload.length), v = new DataView(b.buffer);
    b.set([1, 0]); v.setUint16(2, handle); v.setUint32(4, outer); b[8] = 0x80; b[9] = pt | (marker ? 128 : 0);
    v.setUint16(10, sequence); v.setUint32(12, timestamp); v.setUint32(16, ssrc); b.set(payload, 20);
    return [...b];
}
const row = (elapsed_us, datagram, completed = true) => ({ elapsed_us, datagram,
    original_root_completed_observed: completed });
const video = options => rtp({ handle: 9, pt: 96, payload: [0x10, 0, 1, 2], ...options });
function clock(outer = 2) {
    const body = encodeRtcpSenderReport({ ssrc: 12, ntp: 1234n, rtpTimestamp: 960,
        packets: 2, octets: 4, cname: "synthetic" });
    const b = new Uint8Array(body.length + 8), v = new DataView(b.buffer); b[0] = 1; v.setUint16(2, 7); v.setUint32(4, outer); b.set(body, 8);
    return [...b];
}

test("declared reliable Data resources stay in custody without becoming RTP decoder inputs", () => {
    const scope = { ...expected, sendDataStreamId: "own-data", receiveDataStreamId: "remote-data" };
    const capture = fixture([row(100, rtp())]);
    const binding = (strm_id, direction, stream_local_id) => ({ strm_id, kind: "data", direction,
        stream_local_id, codec: { name: "binary", params: { reliability: "reliable_ordered" } } });
    capture.negotiated_streams.push(binding("own-data", "sendrecv", 10), binding("remote-data", "sendonly", 11));
    const segment = physicalAvSegment(capture, scope);
    assert.equal(segment.negotiated_data_resources.length, 2);
    assert.equal(segment.reliable_data_io_verified, false);
    assert.equal(segment.audio[0].rows.length, 1);
    assert.throws(() => physicalAvSegment(capture, expected));
    assert.throws(() => physicalAvSegment(capture, { ...scope, receiveDataStreamId: undefined }));
    for (const mutate of [s => { s.stream_local_id = 7; }, s => { s.strm_id = "other-data"; },
        s => { s.codec.params.payload_type = 111; }, s => { s.codec.params.reliability = "unordered"; }]) {
        const changed = structuredClone(capture); mutate(changed.negotiated_streams[4]);
        assert.throws(() => physicalAvSegment(changed, scope));
    }
    const wrongPacket = structuredClone(capture); wrongPacket.received.push(row(200, rtp({ handle: 11 })));
    assert.throws(() => physicalAvSegment(wrongPacket, scope));
});

test("private packets without actual original decoder evidence cannot initialize later histories", () => {
    const capture = fixture([
        row(100, rtp(), false), row(200, clock(), false),
        row(300, video({ marker: false, payload: [0x10, 0, 1] }), false),
        row(400, video({ sequence: 2, payload: [0, 2, 3] }), false),
        row(500, rtp({ timestamp: 1920, sequence: 2, outer: 3 })), row(600, clock(4)),
        row(700, video({ sequence: 3, timestamp: 3000 })),
    ]), original = structuredClone(capture);
    const result = physicalAvSegment(capture, expected);
    assert.deepEqual(capture, original);
    assert.deepEqual(result.quarantined_precompletion_rows, [0, 1, 2, 3]);
    assert.equal(result.audio[0].rows.length, 1);
    assert.equal(result.audio[0].rows[0].elapsed_ms, .5);
    assert.equal(result.audio[0].rows[0].timestamp, 1920);
    assert.equal(result.clock_reports.audio.length, 1);
    assert.equal(result.clock_reports.audio[0].physical_observation_row, 5);
    assert.equal(result.video[0].rows.length, 1);
    assert.equal(result.video[0].rows[0].timestamp, 3000);
    assert.equal(result.decoded_media_verified, false);
    assert.equal(result.full_timeline_decoded, false);
});

test("physical SSRC histories stay separate even when RTP timestamps repeat", () => {
    const capture = fixture([row(2_000_000, rtp()), row(2_000_010, rtp({ ssrc: 13, outer: 2 })),
        row(2_000_020, video()), row(2_000_030, video({ ssrc: 13, outer: 2 }))]);
    const result = physicalAvSegment(capture, expected);
    for (const medium of ["audio", "video"]) {
        assert.deepEqual(result[medium].map(v => v.ssrc), [12, 13]);
        assert.deepEqual(result[medium].map(v => v.rows[0].timestamp), [960, 960]);
    }
    assert.equal(result.audio[0].rows[0].elapsed_ms, 2000);
    assert.equal(result.audio[1].rows[0].elapsed_ms, 2000.01);
});

test("assembly expiry uses original capture arrival intervals during fast offline replay", () => {
    const result = physicalAvSegment(fixture([
        row(1_000, video({ marker: false, payload: [0x10, 0, 1] })),
        row(202_000, video({ sequence: 2, payload: [0, 2, 3] })),
    ]), expected);
    assert.equal(result.video[0].rows.length, 0);
    assert.equal(result.pending_video_fragments, 1);
});

test("unknown handles, changed negotiated resources and payload types refuse", () => {
    for (const change of [
        c => { c.received[0].datagram = rtp({ handle: 8 }); },
        c => { c.received[0].datagram = rtp({ pt: 96 }); },
        c => { c.negotiated_streams[0].strm_id = "wrong-camera"; },
        c => { c.negotiated_streams[0].stream_local_id = 7; },
        c => { c.negotiated_streams[0].codec.params.sample_rate = 48000; },
    ]) {
        const capture = fixture([row(100, rtp())]); change(capture);
        assert.throws(() => physicalAvSegment(capture, expected));
    }
});

test("a failed or unjoined original, changed program and regressed boundary refuse", () => {
    for (const change of [
        c => { c.passed = false; }, c => { c.original_client_joined = false; },
        c => { c.original_root_completed_observed = false; },
        c => { c.program_start_unix_us++; },
        c => { c.received[1].original_root_completed_observed = false; },
        c => { c.received[1].elapsed_us = 0; },
        c => { c.received[1].elapsed_us = 62_000_001; },
        c => { c.received.forEach(r => { r.original_root_completed_observed = false; }); },
    ]) {
        const capture = fixture([row(100, rtp()), row(200, rtp({ sequence: 2 }))]); change(capture);
        assert.throws(() => physicalAvSegment(capture, expected));
    }
});

test("malformed RTCP cannot fall back to RTP or become a clock observation", () => {
    const datagram = rtp(); datagram[9] = 200;
    assert.throws(() => physicalAvSegment(fixture([row(100, datagram)]), expected));
});

test("selected replacement capture splits same-SSRC signed epochs and refuses stale or unbound history", () => {
    const epoch = (n, firstSequence, firstTimestamp) => ({ epoch: n.toString(16).padStart(32,"0"),
        incarnation: "f".repeat(32), revision:n, ssrc:12, first_sequence:firstSequence, first_timestamp:firstTimestamp });
    const tagged = (time, n, sequence, timestamp, key) => {
        const plain=Uint8Array.from(video({sequence,timestamp,payload:[0x10,key?0:1,1,2]}));
        new DataView(plain.buffer).setUint32(4,sequence);
        const wire=new Uint8Array(plain.length+16);wire.set(plain.subarray(0,8));wire[1]=1;
        wire.set(Buffer.from(n.epoch,"hex"),8);wire.set(plain.subarray(8),24);
        return {...row(time,[...wire]), video_epoch: n};
    };
    const one=epoch(1,1,960),two=epoch(2,2,3960);
    const capture=fixture([tagged(1_000,one,1,960,true), tagged(34_000,two,2,3960,false),tagged(67_000,two,3,6960,true)]);
    for(const binding of capture.negotiated_streams.filter(s=>s.kind==="video"))binding.video_feedback={profile:"rtcp-pli-epoch/1",original_operation:"move",original_offer:"offer",stream_generation:1,connection_generation:2};
    const before=structuredClone(capture),result=physicalAvSegment(capture,expected);
    assert.deepEqual(capture,before);
    assert.deepEqual(result.video[0].rows.map(r=>r.timestamp),[960,6960]);
    assert.deepEqual(result.video[0].rows.map(r=>r.decoder_history),[1,2]);
    assert.deepEqual(result.video[0].rows.map(r=>r.elapsed_ms),[1,67]);
    for(const change of [c=>{c.received[1].video_epoch.epoch=one.epoch;},
        c=>{c.received[1].video_epoch.ssrc=13;},c=>{c.received[1].video_epoch.first_timestamp++;},
        c=>{c.received[1].video_epoch.revision=1;},c=>{delete c.received[1].video_epoch;},
        c=>{delete c.negotiated_streams[0].video_feedback;}]) {
        const wrong=structuredClone(capture);change(wrong);assert.throws(()=>physicalAvSegment(wrong,expected));
    }
});

const epoch = (n, firstSequence, firstTimestamp, ssrc=12) => ({ epoch: n.toString(16).padStart(32,"0"),
    incarnation: "f".repeat(32), revision:n, ssrc, first_sequence:firstSequence, first_timestamp:firstTimestamp });
function tagged(time, n, sequence, timestamp, key, completed) {
    const plain=Uint8Array.from(video({sequence,timestamp,ssrc:n.ssrc,payload:[0x10,key?0:1,1,2]}));
    new DataView(plain.buffer).setUint32(4,sequence);
    const wire=new Uint8Array(plain.length+16);wire.set(plain.subarray(0,8));wire[1]=1;
    wire.set(Buffer.from(n.epoch,"hex"),8);wire.set(plain.subarray(8),24);
    return {...row(time,[...wire],completed),video_epoch:n};
}
function retained() {
    const one=epoch(1,1,960), capture=fixture([
        {...row(100,rtp(),false),decoder_audio_samples:960},
        tagged(200,one,1,960,true,false),tagged(1000,one,2,3960,false,true)]);
    for(const stream of capture.negotiated_streams.filter(s=>s.kind==="video"))
        stream.video_feedback={profile:"rtcp-pli-epoch/1",original_operation:"move",original_offer:"offer",stream_generation:1,connection_generation:2};
    capture.private_decoded_readiness.frozen_private_observations.video_epoch=one;
    capture.private_decoded_readiness.video_decode_receipts=[{ordinal:0,timestamp_rtp:960,keyframe:true,
        video_epoch:one,encoded_sha256:createHash("sha256").update(Buffer.from([0,1,2])).digest("hex"),
        received_programme_us:200,decoded_programme_us:250}];
    return capture;
}
test("same original private decoder context is retained while every private output stays explicitly excluded",()=>{
    const capture=retained(), before=structuredClone(capture), result=physicalAvSegment(capture,expected);
    assert.deepEqual(capture,before);
    assert.equal(result.format,"connect/physical-av-segment/3");
    assert.equal(result.root_completed_observed_programme_us,1000);
    assert.deepEqual(result.private_preparation_packet_rows,[0,1]);
    assert.deepEqual(result.private_decoder_context_rows,[0,1]);
    assert.deepEqual(result.video[0].rows.map(r=>[r.timestamp,r.elapsed_ms,r.decoder_context_only]),[[960,.2,true],[3960,1,false]]);
    assert.deepEqual(result.video[0].rows.map(r=>r.decoder_history),[1,1]);
    assert.equal(result.audio[0].rows[0].decoder_context_only,true);
    assert.equal(result.video[0].rows[0].original_decoder_ordinal,0);
    assert.equal(result.decoded_media_verified,false);
    assert.equal(result.cell_qualified,false);
});
test("missing actual private keyframe output cannot become an offline startup keyframe",()=>{
    const capture=retained();capture.private_decoded_readiness.video_decode_receipts=[];
    const result=physicalAvSegment(capture,expected);
    assert.deepEqual(result.quarantined_precompletion_rows,[1]);
    assert.ok(result.video[0].rows.every(r=>!r.key));
    assert.deepEqual(result.video[0].rows.map(r=>r.decoder_context_only),[false]);
});
test("wrong physical generation, decoder seal, encoded bytes, epoch or source cannot lend private state",()=>{
    for(const mutate of [
        c=>{c.private_decoded_readiness.connection_id="foreign";},
        c=>{c.private_decoded_readiness.target_generation=3;},
        c=>{c.private_decoded_readiness.decoder_process.forced=true;},
        c=>{c.private_decoded_readiness.decoder_reset_on_root_completion=true;},
        c=>{c.private_decoded_readiness.video_decode_receipts[0].encoded_sha256="0".repeat(64);},
        c=>{c.private_decoded_readiness.video_decode_receipts[0].decoded_programme_us=200200;},
        c=>{c.received[1].video_epoch.ssrc=13;},
        c=>{c.negotiated_streams[0].video_feedback.connection_generation=3;},
    ]) {const capture=structuredClone(retained());mutate(capture);assert.throws(()=>physicalAvSegment(capture,expected));}
    const different=retained();different.private_decoded_readiness.frozen_private_observations.video_epoch=epoch(2,2,3960);
    const result=physicalAvSegment(different,expected);
    assert.ok(result.video[0].rows.every(r=>!r.key));
    assert.ok(result.quarantined_precompletion_rows.includes(1));
});
test("decoder start and actual audio output gate private context without moving original times",()=>{
    const beforeStart=retained();beforeStart.private_decoder_started_programme_us=300;
    beforeStart.private_decoded_readiness.video_decode_receipts=[];
    const result=physicalAvSegment(beforeStart,expected);
    assert.deepEqual(result.quarantined_precompletion_rows,[0,1]);
    assert.equal(result.root_completed_observed_programme_us,1000);
    for(const mutate of [c=>{c.received[0].decoder_audio_samples=0;},c=>{c.private_decoded_readiness.frozen_private_observations.audio_ssrc=13;}]) {
        const capture=retained();mutate(capture);const result=physicalAvSegment(capture,expected);
        assert.ok(result.quarantined_precompletion_rows.includes(0));assert.equal(result.audio.length,0);
    }
});


test("public native packet membership survives a delayed durable root reply without moving original arrival time",()=>{
 const capture=retained();const originalRow=structuredClone(capture.received[2]);
 capture.received[2].original_root_completed_observed=false;
 const last=structuredClone(capture.received[2]);last.elapsed_us=2000;last.original_root_completed_observed=true;
 // A later RTP source frame, with its own original outer reservation.
 const bytes=Uint8Array.from(last.datagram),v=new DataView(bytes.buffer);v.setUint32(4,3);v.setUint16(26,3);v.setUint32(28,6960);last.datagram=[...bytes];
 capture.received.push(last);capture.root_completed_observed_programme_us=2000;
 const segment=physicalAvSegment(capture,expected);const row=segment.video[0].rows.find(r=>r.timestamp===3960);
 assert.equal(row.elapsed_ms,originalRow.elapsed_us/1000);assert.equal(row.decoder_context_only,false);
 assert.equal(segment.root_completed_observed_programme_us,2000);assert.equal(segment.cell_qualified,false);
 const wrong=structuredClone(capture);wrong.private_decoded_readiness.native_activation.audience.scope='private-consultation';
 assert.throws(()=>physicalAvSegment(wrong,expected));
});


test("modeled signed epochs preserve their original outer reservations through capture construction",()=>{
    const one=epoch(1,1,960), received=[tagged(200,one,1,960,true,false),tagged(1000,one,2,3960,false,true)];
    const original=structuredClone(received),capture=fixture(received);
    assert.deepEqual(received,original);
    assert.deepEqual(capture.received,original);
    assert.deepEqual(capture.received.map(r=>new DataView(Uint8Array.from(r.datagram).buffer).getUint32(4)),[1,2]);
    assert.deepEqual(capture.received.map(r=>r.video_epoch.first_sequence),[1,1]);
    assert.equal(capture.private_decoded_readiness.native_activation.audience.streams.find(s=>s.stream_id==='remote-video').public_sequence_floor,2);
});

const sourceGray=()=>{const gray=Array(64).fill(100);gray[0]=200;gray[1]=40;gray[8]=40;gray[9]=200;return gray;};
test("RTC and default route observations preserve changing pixels across the original owner switch",()=>{
    for(const explicitNull of [false,true]){
        const route=new DecodedRoute(0,1);route.start(0);
        route.videoFrame('original',0,1000,sourceGray(),new Uint8Array([1]),640,360);
        const args=['target',33333,1033.333,sourceGray(),new Uint8Array([2]),640,360];
        if(explicitNull)args.push(null);
        route.videoFrame(...args);
        assert.deepEqual(route.video.map(r=>r.changed),[false,true]);
        assert.equal(route.ownerPixels.size,0);
    }
});
test("native private pixel history stays with its original physical owner",()=>{
    const route=new DecodedRoute(0,1);route.start(0);
    const provenance=(handle,sequence)=>({profile:'connect/native-decoded-sequence/1',identity:nativeIdentity(handle,2),datagramSequences:[sequence]});
    route.videoFrame('original',0,1000,sourceGray(),new Uint8Array([1]),640,360,provenance(9,0));
    route.videoFrame('target',33333,1033.333,sourceGray(),new Uint8Array([2]),640,360,provenance(10,0));
    route.videoFrame('target',66667,1066.667,sourceGray(),new Uint8Array([3]),640,360,provenance(10,1));
    assert.deepEqual(route.video.map(r=>r.changed),[false,false,true]);
    assert.equal(route.ownerPixels.size,2);
    assert.equal(route.lastPixels,null);
});
