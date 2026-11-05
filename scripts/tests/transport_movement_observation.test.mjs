import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, readFile, rm, chmod, unlink, symlink } from "node:fs/promises";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { physicalMovementSegments } from "../lib/transport_movement_observation.mjs";
import { writeJoinedMovementCapture } from "../lib/transport_movement_capture_owner.mjs";
import { startProgrammeClock } from "../lib/transport_programme_clock.mjs";

import { MOVE_START_OFFSETS_MS, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256 } from "../lib/transport_movement_profile.mjs";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const save = (path, value) => writeFile(path, JSON.stringify(value) + "\n", { mode: 0o600 });
async function fixture(dataProfile = false) {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "movement-observation-"));
    const connectionIds = Array.from({ length: 6 }, (_, i) => "original-" + i);
    const streamIds = connectionIds.map(id => ({ audio: id + "-audio", video: id + "-video",
        ...(dataProfile ? { data: id + "-data" } : {}) }));
    const program = 1_800_000_000_000_000, executableSha256 = "a".repeat(64);
    const movement = { original_moves_completed: 4, concurrent_session_chains: 2, moves_per_source: [2, 2],
        movement_profile: MOVEMENT_PROFILE, movement_profile_sha256: MOVEMENT_PROFILE_SHA256,
        scheduled_start_offsets_ms: MOVE_START_OFFSETS_MS, active_av_routes: 6, maximum_pending_candidates: 2, shared_sdk_closed_during_replacement: false,
        original_program_start_unix_us: program, moves: [], physical_segments: [] };
    const dataIo = dataProfile === "io";
    if (dataIo) { movement.replacement_data_io_verified = true; movement.replacement_data_exchanges = []; }
    const plan = [], sessionIds = ["session-0", "session-1"];
    for (const sourceIndex of [0, 4]) {
        let current = connectionIds[sourceIndex];
        const sid = Number(sourceIndex === 4), peer = sourceIndex === 0 ? 3 : 5;
        for (let ordinal = 0; ordinal < 2; ordinal++) {
            const key = sourceIndex + "-" + ordinal, folder = join(directory, key);
            await mkdir(folder, { mode: 0o700 });
            const step = { sourceIndex, ordinal, scheduled_start_ms: MOVE_START_OFFSETS_MS[ordinal], operationId: "move-" + key,
                subscriptionId: "camera-" + key, audioId: "audio-" + key, videoId: "video-" + key,
                ...(dataProfile ? { dataId: "data-" + key } : {}) };
            plan.push(step);
            const binding = (strm_id, kind, direction, stream_local_id) => ({ strm_id, kind, direction, stream_local_id,
                codec: { name: kind === "audio" ? "opus" : "vp8", params: {
                    sample_rate: kind === "audio" ? 48000 : 90000, channels: kind === "audio" ? 1 : 0,
                    payload_type: kind === "audio" ? 111 : 96 } } });
            const streams = [binding(step.audioId, "audio", "sendrecv", 7),
                binding(step.videoId, "video", "recvonly", 8), binding(streamIds[peer].video, "video", "sendonly", 9)];
            if (dataProfile) for (const [strm_id, direction, stream_local_id] of [
                [step.dataId, "sendrecv", 10], [streamIds[peer].data, "sendonly", 11]])
                streams.push({ strm_id, kind: "data", direction, stream_local_id,
                    codec: { name: "binary", params: { reliability: "reliable_ordered" } } });
            const datagram = [1, 0, 0, 7, 0, 0, 0, 0, 0x80, 111, 0, 1, 0, 0, 3, 192, 0, 0, 0, 12, 1, 2];
            const observed = { format: "connect/native-replacement-peer-observation/1", passed: true,
                original_client_joined: true, video_enabled: true, original_root_completed_observed: true,
                video_transfer_admitted: true, operation_completed_inferred: false, decoded_media_verified: false,
                program_start_unix_us: program, negotiated_streams: streams,
                private_decoder_started_programme_us: MOVE_START_OFFSETS_MS[ordinal]*1000,
                private_decoded_readiness:{profile:"connect/native-private-decoded-readiness/1",
                    connection_id:"candidate-"+key,source_generation:ordinal+1,target_generation:ordinal+2,stream_revision:2,
                    readiness_frozen:true,decoder_reset_on_root_completion:false,
                    frozen_private_observations:{audio_decoded_samples:2880,audio_ssrc:12,video_decoded_frames:1,video_decoded_keyframes:1,video_epoch:null},
                    decoder_process:{joined:true,clean:true,forced:false,exit_code:0},video_decode_receipts:[]},
                root_completed_observed_programme_us: MOVE_START_OFFSETS_MS[ordinal] * 1000 + 2_000_000,
                received: [{ elapsed_us: MOVE_START_OFFSETS_MS[ordinal] * 1000 + 2_000_000, datagram, original_root_completed_observed: true }] };
            const next = "candidate-" + key;
            let dataFiles, inputHash;
            if (dataIo) {
                const incoming = { operation_id: "data-in-" + key, message_id: "message-in-" + key, stream_id: streamIds[peer].data,
                    from_participant: "upstream", to_participant: "local", body_base64: Buffer.alloc(65536, 7).toString("base64") };
                const outgoing = { operation_id: "data-out-" + key, message_id: "message-out-" + key, stream_id: step.dataId,
                    from_participant: "local", to_participant: "downstream", body_base64: Buffer.alloc(65536, 8).toString("base64") };
                step.data = { incoming, outgoing };
                const scope = { conversation_id: "room", session_id: sessionIds[sid], connection_id: next };
                const cutoff = MOVE_START_OFFSETS_MS[ordinal] + 10100;
                const receipt = { format: "connect/physical-data-exchange/1", root_operation_id: step.operationId,
                    conversation_id: "room", session_id: sessionIds[sid], connection_id: next,
                    incoming_operation_id: incoming.operation_id, incoming_message_id: incoming.message_id,
                    outgoing_operation_id: outgoing.operation_id, outgoing_message_id: outgoing.message_id,
                    incoming_exact_body_base64: incoming.body_base64, incoming_received: true, outgoing_core_queue_admission: true,
                    outgoing_mutation_submissions: 1, outgoing_recovered_read_only: false, original_root_completed_observed: true,
                    cutoff_programme_ms: cutoff, observed_programme_ms: MOVE_START_OFFSETS_MS[ordinal] + 2150, remote_delivery_inferred: false };
                observed.data_exchange_receipt = receipt;
                const reference = { schema: "connect-operation-reference/1", command: "message.send", operation_id: incoming.operation_id,
                    scope: { ...scope, connection_id: connectionIds[peer] } };
                const payload = original => ({ msg_id: original.message_id, to: [original.to_participant], label: original.stream_id,
                    content_type: "application/octet-stream", body_encoding: "base64", body: original.body_base64, reliability: { mode: "reliable_ordered" } });
                const values = {
                    "data-start": { format: "connect/physical-data-start/1", root_operation_id: step.operationId, connection_id: next,
                        incoming_operation_id: incoming.operation_id, outgoing_operation_id: outgoing.operation_id,
                        armed_programme_ms: MOVE_START_OFFSETS_MS[ordinal] + 2100, cutoff_programme_ms: cutoff },
                    "data-original-send": { format: "connect/physical-data-original/1", type: "message.send",
                        scope: { ...scope, operation_id: outgoing.operation_id }, payload: payload(outgoing) },
                    "data-receipt": receipt,
                    "data-upstream-original": { reference, original_cutoff_programme_ms: cutoff, message: incoming },
                    "data-wire": { frames: [{ v: 2, type: "message.send", cid: "room", sid: sessionIds[sid], connid: next,
                        operation_id: incoming.operation_id, payload: { ...payload(incoming), from: incoming.from_participant } },
                    { v: 2, type: "operation.updated", cid: "room", sid: sessionIds[sid], connid: next, in_reply_to: "send-wire",
                        payload: { state: "completed", operation_id: outgoing.operation_id, command_type: "message.send",
                            result: { message_id: outgoing.message_id, connection_id: next, core_queue_admission: true } } },
                    { v: 2, type: "ack", cid: "room", sid: sessionIds[sid], connid: next, in_reply_to: "send-wire", payload: {} }] },
                };
                dataFiles = {};
                const dataPath = suffix => join(folder, (["data-upstream-original", "data-exchange"].includes(suffix) ? suffix : "target-observation." + suffix) + ".json");
                for (const [suffix, value] of Object.entries(values)) { await save(dataPath(suffix), value); dataFiles[suffix] = sha(await readFile(dataPath(suffix))); }
                const exchange = { root_operation_id: step.operationId, connection_id: next, upstream_core_queue_admission: true,
                    physical_incoming_exact_bytes: 65536, downstream: { message_id: outgoing.message_id, exact_received_bytes: 65536,
                        payload_sha256: sha(Buffer.from(outgoing.body_base64, "base64")), connection_id: connectionIds[sourceIndex === 0 ? 1 : 5] },
                    physical_receipt_sha256: dataFiles["data-receipt"], original_cutoff_programme_ms: cutoff, mutations_replayed: false,
                    messages: [incoming, outgoing].map(original => ({ operation_id: original.operation_id, message_id: original.message_id,
                        from_participant: original.from_participant, to_participant: original.to_participant,
                        payload_sha256: sha(Buffer.from(original.body_base64, "base64")) })) };
                await save(dataPath("data-exchange"), exchange); dataFiles["data-exchange"] = sha(await readFile(dataPath("data-exchange")));
                movement.replacement_data_exchanges.push(exchange);
                const input = { media: { data: step.data }, command: { operation_id: step.operationId, conversation_id: "room", session_id: sessionIds[sid], connection_id: current } };
                await save(join(folder, "target-input.json"), input); inputHash = sha(Buffer.from(JSON.stringify(input.media)));
            }
            await save(join(folder, "target-observation.json"), observed);
            await save(join(folder, "target-observation.owner-ready.json"), {
                format: "connect/native-replacement-owner-ready/1", operation_id: step.operationId,
                connection_id: next, video_enabled: true, decoded_media_verified: false,
                operation_completed_inferred: false, negotiated_streams: streams });
            const captureHash = sha(await readFile(join(folder, "target-observation.json")));
            await save(join(folder, "stage.json"), { format: "connect/physical-native-replacement-stage/1", passed: true,
                original_reference_preserved: true, original_root_completed: true, original_source_owner_joined: true,
                target_worker_joined: true, target_worker_forced: false, target_config_unchanged: true,
                target_executable_unchanged: true, target_executable_sha256: executableSha256,
                target_owner_ready_sha256: sha(await readFile(join(folder, "target-observation.owner-ready.json"))),
                ...(dataIo ? { physical_data_exchange_verified: true, physical_data_files: dataFiles, source_input_sha256: inputHash } : {}),
                target_observation_sha256: captureHash });
            await save(join(folder, "original-reference.json"), { command: "connection.replace", operation_id: step.operationId,
                scope: { conversation_id: "room", session_id: sessionIds[sid], connection_id: current } });
            await save(join(folder, "admission.json"), { movement: {
                retired: [current], retained: [connectionIds[peer]], connection: next, binding:{generation:ordinal+2} } });
            movement.moves.push({ source_index: sourceIndex, ordinal, session_index: sid, stage_directory: folder,
                original_reference_preserved: true, original_retired: true, camera_transfer_admitted: true,
                scheduled_start_ms: MOVE_START_OFFSETS_MS[ordinal], actual_start_ms: MOVE_START_OFFSETS_MS[ordinal] + 100 });
            movement.physical_segments.push({ sourceIndex, observation: join(folder, "target-observation.json"), sha256: captureHash });
            current = next;
        }
    }
    return { directory, options: { directory, movement, plan, packetWorkersJoined: true,
        conversationId: "room", sessionIds, connectionIds, streamIds, executableSha256 } };
}
async function run(change, check, dataProfile = false) {
    const { directory, options } = await fixture(dataProfile);
    try { if (change) await change(options); await check(options); }
    finally { await rm(directory, { recursive: true, force: true }); }
}
async function edit(options, name, change) {
    const path = join(options.directory, "0-0", name), value = JSON.parse(await readFile(path, "utf8"));
    change(value); await save(path, value);
}

async function interpreterInput(options) {
    const outputDirectory = join(options.directory, "received");
    await mkdir(outputDirectory, { mode: 0o700 });
    await save(join(options.directory, "plan.json"), options.plan);
    const hashes = [];
    for (let index = 0; index < 6; index++) {
        const bytes = JSON.stringify({ synthetic_original_capture: index }) + "\n";
        await writeFile(join(outputDirectory, "received-" + index + ".json"), bytes, { mode: 0o600 });
        hashes.push(sha(bytes));
    }
    // This synthetic custody test exercises the existing cleanup window; it
    // neither runs nor claims a programme or a decoded media qualification.
    const now = startProgrammeClock();
    const clock = { ...now, origin_ns: (BigInt(now.origin_ns) - 62_000_000_000n).toString() };
    options.movement.original_program_clock = clock;
    options.movement.current_owner_end = { original_cutoff_monotonic_ms: Number(BigInt(clock.origin_ns)) / 1e6 + 67000 };
    const { plan, ...input } = options;
    return { input: { ...input, outputDirectory }, hashes };
}

test("owned interpreter preserves exact capture interpretation and returns only metadata after reaping", () => run(undefined, async options => {
    const { input, hashes } = await interpreterInput(options);
    const expected = await physicalMovementSegments(options);
    const result = await writeJoinedMovementCapture(input);
    const bytes = await readFile(join(input.outputDirectory, "movement-segments.json"), "utf8");
    assert.equal(bytes, JSON.stringify(expected, null, 2) + "\n");
    assert.equal(result.sha256, sha(bytes)); assert.deepEqual(result.original_capture_sha256, hashes);
    assert.equal(result.original_move_records_bound, 4); assert.equal(result.reliable_data_exchanges_bound, 4);
    assert.equal(result.capture_interpreter_joined, true);
    const receiptBytes = await readFile(join(input.outputDirectory, "movement-capture-joined.json"));
    assert.equal(sha(receiptBytes), result.interpreter_process_sha256);
    const receipt = JSON.parse(receiptBytes);
    assert.equal(receipt.parent_pid, process.pid); assert.notEqual(receipt.pid, process.pid);
    assert.equal(receipt.joined, true); assert.equal(receipt.forced, false);
    assert.equal(receipt.exit_code, 0); assert.equal(receipt.signal, null);
    assert.ok(receipt.closed_monotonic_ms < receipt.original_cutoff_monotonic_ms);
    assert.ok(Buffer.byteLength(JSON.stringify(result)) < 2048);
    assert.equal(result.segments, undefined); assert.equal(result.plan, undefined);
}, "io"));

test("an interpreter refusal still joins its actual child and cannot return a capture receipt", () => run(undefined, async options => {
    const { input } = await interpreterInput(options);
    await edit(options, "stage.json", stage => { stage.target_worker_forced = true; });
    await assert.rejects(writeJoinedMovementCapture(input));
    const receipt = JSON.parse(await readFile(join(input.outputDirectory, "movement-capture-joined.json")));
    assert.equal(receipt.joined, true); assert.equal(receipt.forced, false);
    assert.equal(receipt.exit_code, 1);
    await assert.rejects(readFile(join(input.outputDirectory, "movement-segments.json")), { code: "ENOENT" });
}));

test("capture interpretation cannot acquire a fresh deadline or start after original cancellation", () => run(undefined, async options => {
    const { input } = await interpreterInput(options);
    input.movement.current_owner_end.original_cutoff_monotonic_ms += 1;
    await assert.rejects(writeJoinedMovementCapture(input));
    input.movement.current_owner_end.original_cutoff_monotonic_ms -= 1;
    const stop = new AbortController(), failure = Error("original cancelled"); stop.abort(failure);
    await assert.rejects(writeJoinedMovementCapture(input, { signal: stop.signal }), error => error === failure);
    await assert.rejects(readFile(join(input.outputDirectory, "movement-capture-owner.json")), { code: "ENOENT" });
    await assert.rejects(writeJoinedMovementCapture({ ...input, packetWorkersJoined: false }), /packet owners must join/);
}));

test("four capture records bind two exact original chains without claiming decoded quality", () => run(undefined, async options => {
    const result = await physicalMovementSegments(options);
    assert.equal(result.original_move_records_bound, 4);
    assert.equal(result.segments.length, 4);
    assert.deepEqual(result.final_connections, [{ source_index: 0, connection_id: "candidate-0-1" },
        { source_index: 4, connection_id: "candidate-4-1" }]);
    assert.equal(result.segments[0].capture.audio[0].rows[0].elapsed_ms, 17000);
    assert.equal(result.segments[1].capture.audio[0].rows[0].elapsed_ms, 42000);
    assert.equal(result.segments[0].capture.video.length, 0, "fixture has no decoded or assembled video");
    assert.equal(result.full_timeline_decoded, false); assert.equal(result.cell_qualified, false);
}));

test("four Data captures retain both original reliable bindings without claiming Data IO", () => run(undefined, async options => {
    const result = await physicalMovementSegments(options);
    assert.equal(result.segments.length, 4);
    for (const segment of result.segments) {
        assert.equal(segment.capture.negotiated_data_resources.length, 2);
        assert.equal(segment.capture.reliable_data_io_verified, false);
    }
    delete options.plan[0].dataId;
    await assert.rejects(physicalMovementSegments(options), /every replacement must retain its Data contract/);
    options.plan[0].dataId = "other-data";
    await assert.rejects(physicalMovementSegments(options));
    options.plan[0].dataId = options.plan[1].dataId;
    await assert.rejects(physicalMovementSegments(options), /movement identities must be unique/);
}, true));

test("four complete Data proofs bind source contract, original intents, actual receives, parent bytes and every private artifact", () => run(undefined, async options => {
    const result = await physicalMovementSegments(options);
    assert.equal(result.reliable_data_exchanges_bound, 4);
    assert.ok(result.segments.every(segment => segment.reliable_data.original_data_messages_bound === 2));
    assert.equal(result.full_timeline_decoded, false);
    await edit(options, "target-observation.data-receipt.json", value => { value.connection_id = "foreign"; });
    const path = join(options.directory, "0-0/target-observation.data-receipt.json");
    const digest = sha(await readFile(path));
    await edit(options, "stage.json", value => { value.physical_data_files["data-receipt"] = digest; });
    await assert.rejects(physicalMovementSegments(options));
}, "io"));

test("live packet owners and incomplete movement sets refuse before reading a directory", async () => {
    await assert.rejects(physicalMovementSegments({ packetWorkersJoined: false }), /packet workers must join/);
    await assert.rejects(physicalMovementSegments({ packetWorkersJoined: true, movement: { original_moves_completed: 19 } }));
});

test("historical stress counts, another profile and an early move cannot qualify the amended cadence", async () => {
    for (const change of [
        o => { o.movement.original_moves_completed = 20; },
        o => { o.movement.movement_profile = "historical-stress"; },
        o => { o.movement.movement_profile_sha256 = "f".repeat(64); },
        o => { o.movement.scheduled_start_offsets_ms = [0, 1000]; },
        o => { o.plan[0].scheduled_start_ms = 0; },
        o => { o.movement.moves[0].actual_start_ms = 14999; },
        o => { o.movement.moves[0].actual_start_ms = 62000; },
    ]) await run(change, o => assert.rejects(physicalMovementSegments(o)));
});

test("even internally rehashed capture bytes cannot place the original completion before the scheduled move", () => run(async o => {
    await edit(o, "target-observation.json", value => { value.root_completed_observed_programme_us = 14_999_999; });
    const digest = sha(await readFile(join(o.directory, "0-0/target-observation.json")));
    o.movement.physical_segments[0].sha256 = digest;
    await edit(o, "stage.json", value => { value.target_observation_sha256 = digest; });
}, o => assert.rejects(physicalMovementSegments(o), /captured-root-observation-cannot-precede-scheduled-move/)));

test("original operation, Session and predecessor identity cannot be substituted", async () => {
    for (const change of [
        r => { r.operation_id = "different-original"; },
        r => { r.scope.session_id = "session-1"; },
        r => { r.scope.connection_id = "candidate-0-1"; },
    ]) await run(o => edit(o, "original-reference.json", change), o => assert.rejects(physicalMovementSegments(o)));
});

test("changed original bytes, forced cleanup and changed peer executables refuse", async () => {
    for (const [name, change] of [
        ["target-observation.json", r => { r.received[0].datagram[21]++; }],
        ["stage.json", r => { r.target_worker_forced = true; }],
        ["stage.json", r => { r.target_executable_sha256 = "b".repeat(64); }],
        ["target-observation.owner-ready.json", r => { r.connection_id = "different-peer"; }],
    ]) await run(o => edit(o, name, change), o => assert.rejects(physicalMovementSegments(o)));
});

test("duplicate plan roots, repeated candidates and escaped capture folders refuse", async () => {
    const changes = [
        o => { o.plan[1].operationId = o.plan[0].operationId; },
        o => edit(o, "admission.json", r => { r.movement.connection = "original-0"; }),
        o => { o.movement.moves[0].stage_directory = resolve(o.directory, ".."); },
    ];
    for (const change of changes) await run(change, o => assert.rejects(physicalMovementSegments(o)));
});

test("evidence stays private and a symlink cannot substitute an original capture", async () => {
    await run(async o => { await chmod(join(o.directory, "0-0/stage.json"), 0o644); },
        o => assert.rejects(physicalMovementSegments(o)));
    await run(async o => {
        const path = join(o.directory, "0-0/target-observation.json");
        await unlink(path); await symlink(join(o.directory, "4-0/target-observation.json"), path);
    }, o => assert.rejects(physicalMovementSegments(o)));
});

test("actual completed movement generation must match its original private decoder challenge",()=>run(
    o=>edit(o,"admission.json",v=>{v.movement.binding.generation++;}),
    o=>assert.rejects(physicalMovementSegments(o))));
