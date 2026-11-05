import {nativeCut,nativeEgressIdentity,nativeMembership} from "../../sdk/typescript/dist/browser_native_activation.js";
// Offline interpretation of one already-joined physical replacement capture.
// No authority, mutation, clock correction or decoded-quality claim is created.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { decodeRtpDatagram, Vp8Assembler, MEDIA_LIMITS } from "../../sdk/typescript/dist/browser.js";
import { decodeNativeVideoEpoch } from "../../sdk/typescript/dist/browser_video_wire.js";
import { VideoReceiveHistory } from "./transport_video_recovery.mjs";
import { decodeRtcpSenderReport } from "../../sdk/typescript/dist/browser_rtcp.js";

const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const uint = (value, maximum) => Number.isSafeInteger(value) && value >= 0 && value <= maximum;

export function physicalAvSegment(observed, expected) {
    assert.equal(observed.format, "connect/native-replacement-peer-observation/1");
    for (const key of ["passed", "original_client_joined", "video_enabled",
        "original_root_completed_observed", "video_transfer_admitted"])
        assert.equal(observed[key], true, key);
    assert.equal(observed.operation_completed_inferred, false);
    assert.equal(observed.decoded_media_verified, false);
    assert.ok(uint(expected.programStartUnixUs, Number.MAX_SAFE_INTEGER) && expected.programStartUnixUs > 0);
    assert.equal(observed.program_start_unix_us, expected.programStartUnixUs);
    const rootObservedUs = observed.root_completed_observed_programme_us;
    assert.ok(uint(rootObservedUs, 61_999_999), "actual completed-root programme observation is required");
    const readiness = observed.private_decoded_readiness, frozen = readiness?.frozen_private_observations;
    assert.equal(readiness?.profile, "connect/native-private-decoded-readiness/1");
    assert.equal(readiness.connection_id, expected.connectionId);
    assert.ok(id(expected.connectionId) && uint(expected.targetGeneration, Number.MAX_SAFE_INTEGER) && expected.targetGeneration >= 2);
    assert.equal(readiness.target_generation, expected.targetGeneration);
    assert.equal(readiness.source_generation + 1, expected.targetGeneration);
    assert.equal(readiness.stream_revision, 2);
    assert.equal(readiness.readiness_frozen, true);
    assert.equal(readiness.decoder_reset_on_root_completion, false);
    assert.ok(uint(frozen?.audio_decoded_samples, Number.MAX_SAFE_INTEGER) && frozen.audio_decoded_samples >= 2880);
    assert.ok(uint(frozen.audio_ssrc, 0xffffffff));
    assert.ok(uint(frozen.video_decoded_keyframes, Number.MAX_SAFE_INTEGER) && frozen.video_decoded_keyframes > 0 &&
        uint(frozen.video_decoded_frames, Number.MAX_SAFE_INTEGER) && frozen.video_decoded_keyframes <= frozen.video_decoded_frames);
    const decoderStartedUs = observed.private_decoder_started_programme_us;
    assert.ok(uint(decoderStartedUs, rootObservedUs), "original private decoder starts before public ownership");
    assert.equal(readiness.decoder_process?.joined, true);
    assert.equal(readiness.decoder_process.clean, true);
    assert.equal(readiness.decoder_process.forced, false);
    assert.equal(readiness.decoder_process.exit_code, 0);
    const activation=readiness.native_activation;
    assert.deepEqual(Object.keys(activation??{}).sort(),["profile","operation_record_id","operation_id","source_connection_id","target_connection_id","target_generation","stream_revision","audience"].sort());
    assert.equal(activation.profile,"connect/native-public-sequence-cut/1");
    assert.ok(id(readiness.record_id));assert.equal(activation.operation_record_id,readiness.record_id);
    assert.equal(activation.operation_id,expected.operationId);assert.equal(readiness.operation_id,expected.operationId);
    assert.equal(activation.source_connection_id,expected.sourceConnectionId);assert.notEqual(expected.sourceConnectionId,expected.connectionId);
    assert.equal(activation.target_connection_id,expected.connectionId);assert.equal(activation.target_generation,expected.targetGeneration);
    assert.ok(uint(activation.stream_revision,Number.MAX_SAFE_INTEGER)&&activation.stream_revision>2);
    assert.deepEqual(Object.keys(activation.audience).sort(),["scope","streams"]);assert.equal(activation.audience.scope,"public");
    assert.equal(activation.audience.streams.length,2);const cuts=activation.audience.streams.map(nativeCut);
    assert.equal(new Set(cuts.map(c=>c.stream_id)).size,2);assert.equal(new Set(cuts.map(c=>c.stream_local_id)).size,2);
    assert.equal(new Set(cuts.map(c=>c.physical_incarnation)).size,2);
    const decodedReceipts = readiness.video_decode_receipts;
    assert.ok(Array.isArray(decodedReceipts) && decodedReceipts.length <= 1860);
    const originalDecoded = new Map(); let previousOrdinal = -1;
    for (const receipt of decodedReceipts) {
        assert.ok(uint(receipt.ordinal, 1859) && receipt.ordinal > previousOrdinal); previousOrdinal = receipt.ordinal;
        assert.ok(uint(receipt.timestamp_rtp, 0xffffffff) && typeof receipt.keyframe === "boolean");
        assert.match(receipt.encoded_sha256, /^[a-f0-9]{64}$/);
        assert.ok(uint(receipt.received_programme_us, 62_000_000) && receipt.received_programme_us >= decoderStartedUs);
        assert.ok(uint(receipt.decoded_programme_us, 62_000_000) && receipt.decoded_programme_us >= receipt.received_programme_us &&
            receipt.decoded_programme_us - receipt.received_programme_us < 200_000);
        const key = JSON.stringify([receipt.video_epoch?.epoch,receipt.video_epoch?.incarnation,receipt.video_epoch?.revision,
            receipt.video_epoch?.ssrc,receipt.timestamp_rtp]);
        assert.ok(!originalDecoded.has(key), "one actual original decoder output per frame"); originalDecoded.set(key, receipt);
    }
    const ids = [expected.audioStreamId, expected.sendVideoStreamId, expected.receiveVideoStreamId];
    const hasData = expected.sendDataStreamId !== undefined || expected.receiveDataStreamId !== undefined;
    const dataIds = hasData ? [expected.sendDataStreamId, expected.receiveDataStreamId] : [];
    assert.ok([...ids, ...dataIds].every(id) && new Set([...ids, ...dataIds]).size === 3 + dataIds.length);
    assert.equal(observed.negotiated_streams.length, 3 + dataIds.length);
    const streams = new Map(), handles = new Set();
    for (const [index, streamId] of ids.entries()) {
        const matches = observed.negotiated_streams.filter(s => s.strm_id === streamId);
        assert.equal(matches.length, 1, "capture must match the original negotiated stream ID");
        const stream = matches[0], medium = index === 0 ? "audio" : "video";
        assert.equal(stream.kind, medium);
        assert.equal(stream.direction, index === 0 ? "sendrecv" : index === 1 ? "recvonly" : "sendonly");
        assert.equal(stream.codec.name, index === 0 ? "opus" : "vp8");
        assert.equal(stream.codec.params.sample_rate, index === 0 ? 48000 : 90000);
        assert.equal(stream.codec.params.channels, index === 0 ? 1 : 0);
        assert.ok(uint(stream.codec.params.payload_type, 127));
        assert.ok(uint(stream.stream_local_id, 65535) && stream.stream_local_id > 0);
        assert.ok(!handles.has(stream.stream_local_id), "physical stream handles must be distinct");
        handles.add(stream.stream_local_id);
        if (stream.video_feedback) assert.equal(stream.video_feedback.connection_generation, expected.targetGeneration);
        if(index!==1) {
            const identity=nativeEgressIdentity(stream.native_egress),cut=cuts.find(c=>c.stream_id===streamId);
            assert.ok(cut);assert.equal(cut.stream_local_id,stream.stream_local_id);assert.equal(cut.stream_generation,identity.stream_generation);
            assert.equal(cut.physical_incarnation,identity.physical_incarnation);assert.equal(identity.connection_generation,expected.targetGeneration);
            streams.set(stream.stream_local_id,{streamId,medium,pt:stream.codec.params.payload_type,feedback:stream.video_feedback?.profile==="rtcp-pli-epoch/1",
                generation:identity.stream_generation,identity,cut});
        }
    }
    const dataResources = dataIds.map((streamId, index) => {
        const matches = observed.negotiated_streams.filter(stream => stream.strm_id === streamId);
        assert.equal(matches.length, 1, "Data binding must retain its declared identity");
        const stream = matches[0], direction = index === 0 ? "sendrecv" : "sendonly";
        assert.equal(stream.kind, "data"); assert.equal(stream.direction, direction);
        assert.equal(stream.codec.name, "binary");
        assert.deepEqual(stream.codec.params, { reliability: "reliable_ordered" });
        assert.ok(uint(stream.stream_local_id, 65535) && stream.stream_local_id > 0 && !handles.has(stream.stream_local_id));
        handles.add(stream.stream_local_id);
        // Reliable Data uses its canonical message path. These handles must
        // never become RTP/RTCP decoder inputs merely because they negotiated.
        return { stream_id: streamId, direction, stream_local_id: stream.stream_local_id };
    });
    assert.ok(Array.isArray(observed.received) && observed.received.length <= 16000);
    const audio = new Map(), video = new Map(), clocks = { audio: [], video: [] }, assembler = new Vp8Assembler();
    const quarantine = [], privateContext = [], privatePackets = [], recovery = new VideoReceiveHistory(true);
    let currentEpoch;
    let previous = -1, completedSeen = false, bytes = 0;
    for (const [index, row] of observed.received.entries()) {
        assert.ok(uint(row.elapsed_us, 62_000_000) && row.elapsed_us >= previous, "original program arrival order");
        previous = row.elapsed_us;
        assert.equal(typeof row.original_root_completed_observed, "boolean");
        assert.equal(row.original_root_completed_observed, row.elapsed_us >= rootObservedUs,
            "packet completion boundary must match the actual original-root observation");
        assert.ok(!completedSeen || row.original_root_completed_observed, "completion observation cannot regress");
        assert.ok(Array.isArray(row.datagram) && row.datagram.length >= 20 &&
            row.datagram.length <= MEDIA_LIMITS.rtpBytes + 24,
            "capture exceeds its negotiated native framing bound");
        assert.ok(row.datagram.every(value => uint(value, 255)));
        const wire = Uint8Array.from(row.datagram), view = new DataView(wire.buffer);
        assert.equal(wire[0], 1); assert.ok(wire[1] === 0 || wire[1] === 1);
        const stream = streams.get(view.getUint16(2));
        assert.ok(stream, "received physical handle must be the negotiated audio or retained camera");
        // The selected epoch adds 16 framing bytes, not RTP payload capacity.
        // Match the public strict decoders, including for quarantined rows.
        const tagged = wire[1] === 1;
        if (tagged) assert.ok(stream.medium === "video" && stream.feedback,
            "tagged packet requires its selected video allocation");
        assert.ok(wire.length >= (tagged ? 36 : 20) &&
            wire.length <= MEDIA_LIMITS.rtpBytes + (tagged ? 24 : 8),
            "capture exceeds its negotiated native framing bound");
        bytes += row.datagram.length;
        assert.ok(bytes <= 16 * 1024 * 1024);
        // Original same-epoch private bytes can warm only this already-live
        // decoder. Its outputs stay outside the unchanged public window.
        const outer=view.getUint32(4);
        const provenance={profile:"connect/native-decoded-sequence/1",identity:stream.identity,datagramSequences:[outer]};
        const contextOnly=nativeMembership(stream.cut,provenance)==="private";
        if (contextOnly) privatePackets.push(index);
        if (row.elapsed_us < decoderStartedUs) { quarantine.push(index); continue; }
        completedSeen ||= row.original_root_completed_observed;
        const elapsed_ms = row.elapsed_us / 1000;
        if (wire[1] === 0 && wire[9] >= 192 && wire[9] <= 223) {
            // A malformed RTCP claim is never reinterpreted as RTP.
            const report = decodeRtcpSenderReport(wire.subarray(8));
            if (contextOnly) { quarantine.push(index); continue; }
            assert.ok(clocks[stream.medium].length < 512);
            clocks[stream.medium].push({ elapsed_ms, physical_observation_row: index,
                stream_id: stream.streamId, ssrc: report.ssrc, ntp: report.ntp.toString(),
                rtp: report.rtpTimestamp, packets: report.packets, octets: report.octets });
            continue;
        }
        let packet, epoch;
        if (wire[1] === 1) {
            assert.ok(stream.medium === "video" && stream.feedback, "tagged packet requires its selected video allocation");
            const tagged = decodeNativeVideoEpoch(wire); packet = tagged.packet; epoch = row.video_epoch;
            assert.ok(epoch && /^[0-9a-f]{32}$/.test(epoch.incarnation) && !/^0+$/.test(epoch.incarnation));
            assert.deepEqual(Object.keys(epoch).sort(), ["epoch", "incarnation", "revision", "ssrc", "first_sequence", "first_timestamp"].sort());
            assert.ok(uint(epoch.revision, Number.MAX_SAFE_INTEGER) && epoch.revision > 0);
            assert.ok(uint(epoch.first_sequence, 0xffffffff) && uint(epoch.first_timestamp, 0xffffffff));
            assert.equal(epoch.epoch, tagged.epoch); assert.equal(epoch.ssrc, packet.ssrc);
            assert.ok(((packet.datagramSequence - epoch.first_sequence) >>> 0) < 0x80000000);
            if (packet.datagramSequence === epoch.first_sequence) assert.equal(packet.timestamp, epoch.first_timestamp);
            if (currentEpoch && currentEpoch.epoch === epoch.epoch) assert.deepEqual(epoch, currentEpoch);
            else if (currentEpoch) assert.ok(epoch.revision > currentEpoch.revision, "recorded video epoch revision cannot regress");
            currentEpoch = epoch;
        } else {
            assert.ok(!(stream.medium === "video" && stream.feedback), "selected video requires its original epoch packet");
            assert.ok(row.video_epoch === undefined || row.video_epoch === null);
            packet = decodeRtpDatagram(wire);
        }
        if (contextOnly && (stream.medium === "audio" ? packet.ssrc !== frozen.audio_ssrc || row.decoder_audio_samples !== 960
            : !epoch || !isDeepStrictEqual(epoch, frozen.video_epoch))) {
            // Another private source/history never lends startup state to this
            // completed target. Raw custody remains in the original capture.
            quarantine.push(index); continue;
        }
        assert.equal(packet.payloadType, stream.pt);
        assert.ok(packet.payload.length > 0);
        const common = { elapsed_ms, physical_observation_row: index, stream_id: stream.streamId,
            ssrc: packet.ssrc, timestamp: packet.timestamp, decoder_context_only: contextOnly,
            connection_generation: expected.targetGeneration, stream_generation: stream.generation,
            native_datagram_sequences:[outer],native_physical_incarnation:stream.identity.physical_incarnation };
        const segments = stream.medium === "audio" ? audio : video;
        if (!segments.has(packet.ssrc)) {
            assert.ok(segments.size < 8, "bounded physical codec histories");
            segments.set(packet.ssrc, []);
        }
        if (stream.medium === "audio") {
            if (contextOnly) privateContext.push(index);
            assert.ok(segments.get(packet.ssrc).length < 3200);
            segments.get(packet.ssrc).push({ ...common, sequence: packet.sequence,
                payload: Buffer.from(packet.payload).toString("base64") });
        } else {
            // Use the actual recorded receive time for the unchanged200ms
            // assembly expiry, never the speed of this offline reader.
            const frame = epoch ? recovery.push(stream.streamId, { header: { ssrc: packet.ssrc,
                timestamp: packet.timestamp, sequenceNumber: packet.sequence, datagramSequence:packet.datagramSequence, marker: packet.marker,
                payloadType: packet.payloadType }, payload: packet.payload, videoEpoch: epoch }, elapsed_ms)
                : assembler.push(packet, elapsed_ms);
            if (frame) {
                const frameSequences=frame.datagramSequences;
                const framePrivate=nativeMembership(stream.cut,{...provenance,datagramSequences:frameSequences})==="private";
                assert.equal(framePrivate,contextOnly,"actual complete frame cannot straddle the original cut");
                common.native_datagram_sequences=frameSequences;
                let receipt;
                if (contextOnly) {
                    const key = JSON.stringify([epoch.epoch,epoch.incarnation,epoch.revision,epoch.ssrc,frame.timestamp]);
                    receipt = originalDecoded.get(key);
                    if (!receipt) { quarantine.push(index); continue; }
                    assert.ok(isDeepStrictEqual(receipt.video_epoch, epoch));
                    assert.equal(receipt.keyframe, frame.key);
                    assert.equal(receipt.encoded_sha256, createHash("sha256").update(frame.bytes).digest("hex"));
                    assert.ok(receipt.decoded_programme_us >= row.elapsed_us && receipt.decoded_programme_us - row.elapsed_us < 200_000);
                    privateContext.push(index);
                }
                assert.ok(segments.get(packet.ssrc).length < 2000);
                segments.get(packet.ssrc).push({ ...common, key: frame.key,
                    ...(epoch ? { video_epoch: epoch, decoder_history: frame.decoderHistory } : {}),
                    ...(receipt ? { original_decoder_ordinal: receipt.ordinal, original_decoder_encoded_sha256: receipt.encoded_sha256,
                        original_decoder_completed_programme_us: receipt.decoded_programme_us } : {}),
                    payload: Buffer.from(frame.bytes).toString("base64") });
            }
        }
    }
    assert.ok(completedSeen, "capture needs an actual post-completion observation");
    const histories = groups => [...groups].map(([ssrc, rows]) => ({ ssrc, rows }));
    return { format: "connect/physical-av-segment/3", native_activation:activation, program_start_unix_us: observed.program_start_unix_us,
        root_completed_observed_programme_us: rootObservedUs,
        audio: histories(audio), video: histories(video), clock_reports: clocks,
        decoder_context: { profile: "connect/native-retained-private-context/1", connection_id: expected.connectionId,
            source_generation: readiness.source_generation, target_generation: expected.targetGeneration,
            decoder_started_programme_us: decoderStartedUs, audio_stream_id: expected.audioStreamId,
            video_stream_id: expected.receiveVideoStreamId, audio_ssrc: frozen.audio_ssrc,
            video_epoch: frozen.video_epoch ?? null, video_stream_generation: [...streams.values()].find(s => s.medium === "video").generation },
        private_preparation_packet_rows: privatePackets, private_decoder_context_rows: privateContext,
        quarantined_precompletion_rows: quarantine, original_datagram_count: observed.received.length,
        negotiated_data_resources: dataResources, reliable_data_io_verified: false,
        original_datagram_bytes: bytes, pending_video_fragments: assembler.queuedFragments + recovery.pendingFragments,
        decoded_media_verified: false, full_timeline_decoded: false, cell_qualified: false,
        scope: "Original receive times and codec bytes only, split by physical SSRC. Public membership is the original signed per-writer cut, independent of root reply arrival. Only original same-physical/stream/SSRC/generation/epoch private bytes can initialize the retained decoder; their outputs never enter public coverage. No PLC, gap filling, retiming or media quality inference." };
}
