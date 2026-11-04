// Offline interpretation of one already-joined physical replacement capture.
// No authority, mutation, clock correction or decoded-quality claim is created.
import assert from "node:assert/strict";
import { decodeRtpDatagram, Vp8Assembler } from "../../sdk/typescript/dist/browser.js";
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
        if (index !== 1) streams.set(stream.stream_local_id, { streamId, medium, pt: stream.codec.params.payload_type, feedback: stream.video_feedback?.profile === "rtcp-pli-epoch/1" });
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
    const quarantine = [], recovery = new VideoReceiveHistory(true);
    let currentEpoch;
    let previous = -1, completedSeen = false, bytes = 0;
    for (const [index, row] of observed.received.entries()) {
        assert.ok(uint(row.elapsed_us, 62_000_000) && row.elapsed_us >= previous, "original program arrival order");
        previous = row.elapsed_us;
        assert.equal(typeof row.original_root_completed_observed, "boolean");
        assert.equal(row.original_root_completed_observed, row.elapsed_us >= rootObservedUs,
            "packet completion boundary must match the actual original-root observation");
        assert.ok(!completedSeen || row.original_root_completed_observed, "completion observation cannot regress");
        assert.ok(Array.isArray(row.datagram) && row.datagram.length >= 20 && row.datagram.length <= 1188);
        assert.ok(row.datagram.every(value => uint(value, 255)));
        bytes += row.datagram.length;
        assert.ok(bytes <= 16 * 1024 * 1024);
        // Never feed a pre-completion fragment or clock into a later decoder.
        // Keep its original row index so the raw input still accounts for it.
        if (!row.original_root_completed_observed) { quarantine.push(index); continue; }
        completedSeen = true;
        const wire = Uint8Array.from(row.datagram), view = new DataView(wire.buffer);
        assert.equal(wire[0], 1); assert.ok(wire[1] === 0 || wire[1] === 1);
        const stream = streams.get(view.getUint16(2));
        assert.ok(stream, "received physical handle must be the negotiated audio or retained camera");
        const elapsed_ms = row.elapsed_us / 1000;
        if (wire[1] === 0 && wire[9] >= 192 && wire[9] <= 223) {
            // A malformed RTCP claim is never reinterpreted as RTP.
            const report = decodeRtcpSenderReport(wire.subarray(8));
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
        assert.equal(packet.payloadType, stream.pt);
        assert.ok(packet.payload.length > 0);
        const common = { elapsed_ms, physical_observation_row: index, stream_id: stream.streamId,
            ssrc: packet.ssrc, timestamp: packet.timestamp };
        const segments = stream.medium === "audio" ? audio : video;
        if (!segments.has(packet.ssrc)) {
            assert.ok(segments.size < 8, "bounded physical codec histories");
            segments.set(packet.ssrc, []);
        }
        if (stream.medium === "audio") {
            assert.ok(segments.get(packet.ssrc).length < 3200);
            segments.get(packet.ssrc).push({ ...common, sequence: packet.sequence,
                payload: Buffer.from(packet.payload).toString("base64") });
        } else {
            // Use the actual recorded receive time for the unchanged200ms
            // assembly expiry, never the speed of this offline reader.
            const frame = epoch ? recovery.push(stream.streamId, { header: { ssrc: packet.ssrc,
                timestamp: packet.timestamp, sequenceNumber: packet.sequence, marker: packet.marker,
                payloadType: packet.payloadType }, payload: packet.payload, videoEpoch: epoch }, elapsed_ms)
                : assembler.push(packet, elapsed_ms);
            if (frame) {
                assert.ok(segments.get(packet.ssrc).length < 2000);
                segments.get(packet.ssrc).push({ ...common, key: frame.key,
                    ...(epoch ? { video_epoch: epoch, decoder_history: frame.decoderHistory } : {}),
                    payload: Buffer.from(frame.bytes).toString("base64") });
            }
        }
    }
    assert.ok(completedSeen, "capture needs an actual post-completion observation");
    const histories = groups => [...groups].map(([ssrc, rows]) => ({ ssrc, rows }));
    return { format: "connect/physical-av-segment/1", program_start_unix_us: observed.program_start_unix_us,
        root_completed_observed_programme_us: rootObservedUs,
        audio: histories(audio), video: histories(video), clock_reports: clocks,
        quarantined_precompletion_rows: quarantine, original_datagram_count: observed.received.length,
        negotiated_data_resources: dataResources, reliable_data_io_verified: false,
        original_datagram_bytes: bytes, pending_video_fragments: assembler.queuedFragments + recovery.pendingFragments,
        decoded_media_verified: false, full_timeline_decoded: false, cell_qualified: false,
        scope: "Original receive times and codec bytes only, split by physical SSRC. Pre-completion rows excluded from codec state. No PLC, gap filling, retiming or media quality inference." };
}
