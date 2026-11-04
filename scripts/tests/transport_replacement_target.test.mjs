import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { replacementInput, physicalReplacementTarget, verifyReplacementStreamIdentities, verifyPhysicalOwnerEndReceipt } from "../lib/transport_replacement_target.mjs";

const intent = () => ({ operationId: "original-operation", conversationId: "original-conversation",
    sessionId: "original-session", connectionId: "original-source", retainedConnectionId: "original-retained",
    expectedTopologyRevision: 1, originalGeneration: 1, retainedGeneration: 1, streamId: "original-target-audio", target: {
        endpointId: "original-endpoint", deviceId: "separate-device", nativeConfig: "/private/client.toml" },
    source: { seconds: 1, ssrc: 7, audioPackets: Array.from({ length: 50 }, () => [1, 2, 3]) }, output: "/private/observation.json" });

test("physical target input preserves the exact command, copies source packets and uses native codec objects", () => {
    const original = intent(), captured = replacementInput(original);
    original.source.audioPackets[0][0] = 99;
    assert.equal(captured.media.audio_packets[0][0], 1);
    assert.deepEqual(captured.command, { operation_id: "original-operation", conversation_id: "original-conversation",
        session_id: "original-session", connection_id: "original-source", expected_revision: 1,
        intent: { type: "connection.replace", endpoint_id: "original-endpoint", candidate_connid: null,
            retained_connid: "original-retained" }, payload_extensions: {} });
    assert.deepEqual(captured.offer.capabilities.audio_codecs, [{ name: "opus", params: { sample_rate: 48000, channels: 1 } }]);
    assert.equal(captured.offer.streams_offered[0].id, "original-target-audio");
});

test("unbounded sources, duplicate physical legs and invalid originals refuse before target IO", () => {
    for (const change of [v => { v.source.seconds = 31; }, v => { v.source.ssrc = 0; },
        v => { v.source.audioPackets.pop(); }, v => { v.source.audioPackets[0] = Array(1169).fill(1); },
        v => { v.source.audioPackets[0][0] = 256; }, v => { v.retainedConnectionId = v.connectionId; },
        v => { v.expectedTopologyRevision = 0; }, v => { v.operationId = "invalid/id"; }]) {
        const value = intent(); change(value); assert.throws(() => replacementInput(value));
    }
});

const avIntent = () => {
    const value = intent();
    value.source.video = { streamId: "original-target-camera", previousStreamId: "previous-camera", previousReceiveVideoStreamId: "retained-camera", receiveAudioStreamId: "retained-audio",
        receiveVideoStreamId: "retained-camera", ssrc: 8, program: { executable: "/private/.local/encoder", executable_sha256: "a".repeat(64), raw_path: "/private/.local/source.i420", raw_sha256: "b".repeat(64), frame_count: 30 } };
    return value;
};

test("physical Data originals bind both streams and stay distinct from replacement and terminal mutation keys", () => {
    const value = avIntent();
    value.source.seconds = 62; value.source.programStartUnixUs = 1793600000000000;
    value.source.audioPackets = Array.from({ length: 3100 }, () => [1, 2, 3]);
    value.source.video.program.frame_count = 1860;
    value.source.video.receiveDataStreamId = "incoming-data"; value.dataStreamId = "outgoing-data";
    value.source.data = { incoming: { operation_id: "incoming-original", message_id: "incoming-message", stream_id: "incoming-data",
        from_participant: "upstream", to_participant: "local", body_base64: Buffer.alloc(65536, 7).toString("base64") },
        outgoing: { operation_id: "outgoing-original", message_id: "outgoing-message", stream_id: "outgoing-data",
            from_participant: "local", to_participant: "downstream", body_base64: Buffer.alloc(65536, 8).toString("base64") } };
    assert.deepEqual(replacementInput(value).media.data, value.source.data);
    for (const change of [v => { v.source.data.outgoing.stream_id = "foreign"; },
        v => { v.source.data.incoming.stream_id = "foreign"; }, v => { v.source.data.outgoing.operation_id = v.operationId; },
        v => { v.source.terminalEndOperationId = v.source.data.incoming.operation_id; }, v => { v.source.programStartUnixUs = undefined; }]) {
        const bad = structuredClone(value); change(bad); assert.throws(() => replacementInput(bad));
    }
});

test("live AV target retains stream identities and copies the finite raw-source descriptor", () => {
    const value = avIntent();
    value.source.seconds = 62;
    value.source.audioPackets = Array.from({ length: 62 * 50 }, () => [1, 2, 3]);
    value.source.video.program.frame_count = 1860;
    const captured = replacementInput(value);
    value.source.video.program.raw_sha256 = "c".repeat(64);
    assert.equal(captured.media.video.program.raw_sha256, "b".repeat(64));
    assert.equal(captured.media.video.frames, undefined);
    assert.equal(captured.media.seconds, 62);
    assert.deepEqual(captured.offer.capabilities.video_codecs,
        [{ name: "vp8", params: { sample_rate: 90000, channels: 0 } }]);
    assert.deepEqual(captured.offer.streams_offered.map(s => [s.id, s.kind, s.direction]),
        [[value.streamId, "audio", "sendrecv"], [value.source.video.streamId, "video", "sendonly"], [value.source.video.receiveVideoStreamId, "video", "recvonly"]]);
    assert.equal(captured.media.video.receive_audio_stream_id, "retained-audio");
    assert.equal(captured.media.video.receive_video_stream_id, "retained-camera");
});

test("replacement carries its distinct reliable Data offer alongside the original AV program", () => {
    const value = avIntent(); value.dataStreamId = "original-target-data";
    value.source.video.receiveDataStreamId = "retained-data";
    const captured = replacementInput(value);
    assert.deepEqual(captured.offer.capabilities.data_protocols, ["binary"]);
    assert.deepEqual(captured.offer.streams_offered.map(s => [s.id, s.kind, s.direction]),
        [[value.streamId, "audio", "sendrecv"], [value.source.video.streamId, "video", "sendonly"], [value.source.video.receiveVideoStreamId, "video", "recvonly"],
            [value.dataStreamId, "data", "sendrecv"]]);
    assert.deepEqual(captured.offer.streams_offered[3].codec_preferences, ["binary"]);
    assert.deepEqual(captured.media.audio_packets, value.source.audioPackets);
    assert.equal(captured.offer.capabilities.max_streams_per_connection, 8);
    assert.equal(captured.media.video.receive_data_stream_id, "retained-data");
    for (const wrong of [value.streamId, value.source.video.streamId,
        value.source.video.receiveAudioStreamId, value.source.video.receiveVideoStreamId, "retained-data", "", null, "invalid:id"]) {
        assert.throws(() => replacementInput({ ...value, dataStreamId: wrong }));
    }
    for (const wrong of [value.dataStreamId, value.streamId, value.source.video.streamId,
        value.source.video.receiveAudioStreamId, value.source.video.receiveVideoStreamId, "invalid:id"]) {
        assert.throws(() => replacementInput({ ...value, source: { ...value.source,
            video: { ...value.source.video, receiveDataStreamId: wrong } } }));
    }
    assert.throws(() => replacementInput({ ...value, dataStreamId: undefined }));
});

test("live AV source rejects immutable frames, invalid counts, paths and hashes before target IO", () => {
    for (const change of [v => { v.source.seconds = 63; },
        v => { v.source.video.ssrc = v.source.ssrc; },
        v => { v.source.video.streamId = v.streamId; },
        v => { v.source.video.receiveVideoStreamId = v.source.video.receiveAudioStreamId; },
        v => { v.source.video.program.frame_count--; },
        v => { v.source.video.program.raw_path = "relative.i420"; },
        v => { v.source.video.program.executable = "/private/.local/../foreign"; },
        v => { v.source.video.program.raw_sha256 = "bad"; },
        v => { v.source.video.program.extra = true; },
        v => { v.source.video.frames = [[4, 0, 0]]; }]) {
        const value = avIntent(); change(value); assert.throws(() => replacementInput(value));
    }
});

test("accepted-root reads share the original deadline and a changed preparation cannot launch a target", async () => {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "physical-original-root-"));
    try {
        const config = join(directory, "client.toml"), executable = join(directory, "target");
        await writeFile(config, "synthetic-private-config", { mode: 0o600 });
        // It must never execute: an executable hash alone cannot authorize a target.
        await writeFile(executable, "invalid-original-must-refuse-before-spawn", { mode: 0o700 });
        const value = intent(); value.target.nativeConfig = config;
        const reference = { command: "connection.replace", operation_id: value.operationId,
            scope: Object.assign(Object.create(null), { conversation_id: value.conversationId,
                session_id: value.sessionId, connection_id: value.connectionId }) };
        let originalSubmitted = false, sourceClosed = false, reads = 0, lastBudget = 8001;
        const controller = { conversations: { byId: () => ({ sessions: { byId: () => ({ connections: { byId: () => ({
            replaceDevice: async (_, options) => {
                assert.equal(options.operationId, value.operationId);
                const saved = JSON.parse(await readFile(join(directory, "stage/original-command.json"), "utf8"));
                assert.equal(saved.operation_id, value.operationId); originalSubmitted = true;
                return { referenceJson: () => reference, wait: () => assert.fail("wrong root reached completion wait") };
            },
        }) } }) } }) }, raw: { validate: v => v, request: async (frame, options) => {
            assert.equal(frame.payload.operation_id, value.operationId);
            assert.ok(options.timeoutMs < lastBudget); lastBudget = options.timeoutMs; reads++;
            return [{ value: { type: "operation.updated", payload: {
                operation_id: value.operationId, record_id: "original-record", revision: reads - 1,
                command_type: "connection.replace", expected_revision: reads === 1 ? 1 : 999,
                state: reads === 1 ? "accepted" : "preparing",
            } } }];
        } } };
        await assert.rejects(physicalReplacementTarget({ controller, retireOriginal: async () => { sourceClosed = true; },
            privateRoot: parent, directory: join(directory, "stage"), executable,
            executableSha256: createHash("sha256").update(await readFile(executable)).digest("hex"), ...value }));
        const result = JSON.parse(await readFile(join(directory, "stage/stage.json"), "utf8"));
        assert.equal(originalSubmitted, true); assert.equal(sourceClosed, false);
        assert.equal(reads, 2);
        assert.equal(result.passed, false); assert.equal(result.target_worker_joined, false);
        assert.equal(result.failure.phase, "original-allocation-root");
        assert.ok(result.failure.source.line > 0);
        assert.equal(result.target_config_unchanged, true); assert.equal(result.target_executable_unchanged, true);
    } finally { await rm(directory, { recursive: true, force: true }); }
});

test("replacement inputs carry one immutable AV program origin without shifting source packets", () => {
    const value = avIntent(); value.source.programStartUnixUs = 1793600000123456;
    const captured = replacementInput(value);
    assert.equal(captured.media.program_start_unix_us, value.source.programStartUnixUs);
    assert.deepEqual(captured.media.audio_packets, value.source.audioPackets);
    assert.deepEqual(captured.media.video.program, value.source.video.program);
    for (const origin of [0, -1, 1.25, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
        value.source.programStartUnixUs = origin;
        assert.throws(() => replacementInput(value));
    }
});


test("live and joined replacement resource identities require both exact Data bindings", () => {
    const value = avIntent(); value.dataStreamId = "own-data";
    value.source.video.receiveDataStreamId = "retained-data";
    const av = [value.streamId, value.source.video.streamId, value.source.video.receiveVideoStreamId];
    const complete = [...av, value.dataStreamId, value.source.video.receiveDataStreamId];
    const streams = ids => ids.map(strm_id => ({ strm_id }));
    verifyReplacementStreamIdentities(streams(complete.reverse()), value);
    for (const ids of [av, [...av, value.dataStreamId], [...av, value.source.video.receiveDataStreamId],
        [...complete, "foreign-data"], complete.map(id => id === value.dataStreamId ? "foreign-data" : id),
        [...complete, value.dataStreamId]]) {
        assert.throws(() => verifyReplacementStreamIdentities(streams(ids), value));
    }
    const withoutData = avIntent();
    verifyReplacementStreamIdentities(streams(av), withoutData);
    assert.throws(() => verifyReplacementStreamIdentities(streams(complete), withoutData));
    assert.throws(() => verifyReplacementStreamIdentities([{ strm_id: "bad:id" }], value));
});


test("terminal owner input preserves its original62second programme and separate end key", () => {
    const value = avIntent(); value.source.seconds = 62;
    value.source.audioPackets = Array.from({ length: 3100 }, () => [1, 2, 3]);
    value.source.video.program.frame_count = 1860;
    value.source.programStartUnixUs = 1793600000000000;
    value.source.terminalEndOperationId = "original-final-end";
    const captured = replacementInput(value);
    assert.equal(captured.media.terminal_end_operation, "original-final-end");
    assert.equal(captured.media.program_start_unix_us, value.source.programStartUnixUs);
    assert.equal(captured.media.seconds, 62);
    assert.deepEqual(captured.media.audio_packets, value.source.audioPackets);
    for (const change of [v => { delete v.source.video; }, v => { v.source.seconds = 61; },
        v => { delete v.source.programStartUnixUs; }, v => { v.source.terminalEndOperationId = v.operationId; },
        v => { v.source.terminalEndOperationId = "invalid:id"; }]) {
        const wrong = structuredClone(value); change(wrong);
        assert.throws(() => replacementInput(wrong));
    }
});

test("final end receipt belongs to the current physical owner, one original mutation and original cutoff", () => {
    const value = avIntent(); value.source.terminalEndOperationId = "original-final-end";
    const receipt = { format: "connect/physical-owner-end-receipt/1", completed: true,
        original_operation_id_preserved: true, mutation_submissions: 1,
        control_only_after_programme: true, media_programme_extended: false,
        elapsed_since_programme_end_ms: 400,
        original: { format: "connect/physical-owner-original-end/1", type: "connection.end",
            scope: { conversation_id: value.conversationId, session_id: value.sessionId,
                connection_id: "actual-current-owner", operation_id: "original-final-end" },
            payload: { reason_code: 200, reason: "Synthetic workload complete" },
            programme_end_monotonic_ms: 62000, original_cutoff_monotonic_ms: 67000 } };
    verifyPhysicalOwnerEndReceipt(receipt, value, "actual-current-owner");
    for (const change of [v => { v.completed = false; }, v => { v.mutation_submissions = 2; },
        v => { v.original_operation_id_preserved = false; }, v => { v.control_only_after_programme = false; },
        v => { v.media_programme_extended = true; }, v => { v.elapsed_since_programme_end_ms = 5000; },
        v => { v.original.scope.connection_id = value.connectionId; },
        v => { v.original.scope.operation_id = value.operationId; },
        v => { v.original.original_cutoff_monotonic_ms = 68000; }]) {
        const wrong = structuredClone(receipt); change(wrong);
        assert.throws(() => verifyPhysicalOwnerEndReceipt(wrong, value, "actual-current-owner"));
    }
});

 test("prepared video intent and both directional resources precede original replacement submission", () => {
    const original = avIntent(), captured = replacementInput(original);
    assert.deepEqual(captured.command.intent.video, { publications: [{ previous: "previous-camera", replacement: "original-target-camera" }],
        receivers: [{ previous: "retained-camera", replacement: "retained-camera" }] });
    assert.deepEqual(captured.offer.streams_offered.filter(s => s.kind === "video").map(s => [s.id, s.direction]),
        [["original-target-camera", "sendonly"], ["retained-camera", "recvonly"]]);
    for (const key of ["previousStreamId", "previousReceiveVideoStreamId"]) {
        const invalid = avIntent(); delete invalid.source.video[key];
        assert.throws(() => replacementInput(invalid));
    }
 });

test("prepared video mapping reaches the SDK unchanged under the persisted original operation", async () => {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "prepared-video-submission-"));
    try {
        const config = join(directory, "client.toml"), executable = join(directory, "target");
        await writeFile(config, "synthetic-private-config", { mode: 0o600 });
        const bytes = Buffer.from("must-not-execute-before-accepted-original");
        await writeFile(executable, bytes, { mode: 0o700 });
        const value = avIntent(); value.target.nativeConfig = config;
        const refusal = Error("original-submission-refused"), submissions = [];
        const connection = { async replaceDevice(input, options) {
            submissions.push({ input, options });
            const saved = JSON.parse(await readFile(join(directory, "stage/original-command.json"), "utf8"));
            assert.deepEqual(input.video, saved.intent.video);
            assert.equal(options.operationId, saved.operation_id);
            throw refusal;
        } };
        const controller = { conversations: { byId: () => ({ sessions: { byId: () => ({ connections: { byId: () => connection } }) } }) } };
        await assert.rejects(physicalReplacementTarget({ ...value, controller,
            retireOriginal: async () => assert.fail("refused move cannot retire original"), privateRoot: directory,
            directory: join(directory, "stage"), executable, executableSha256: createHash("sha256").update(bytes).digest("hex") }), error => error === refusal);
        assert.equal(submissions.length, 1);
        assert.deepEqual(submissions[0].input.video, replacementInput(value).command.intent.video);
    } finally { await rm(directory, { recursive: true, force: true }); }
});
