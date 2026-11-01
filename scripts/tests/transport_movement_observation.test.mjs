import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, readFile, rm, chmod, unlink, symlink } from "node:fs/promises";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { physicalMovementSegments } from "../lib/transport_movement_observation.mjs";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const save = (path, value) => writeFile(path, JSON.stringify(value) + "\n", { mode: 0o600 });
async function fixture() {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "movement-observation-"));
    const connectionIds = Array.from({ length: 6 }, (_, i) => "original-" + i);
    const streamIds = connectionIds.map(id => ({ audio: id + "-audio", video: id + "-video" }));
    const program = 1_800_000_000_000_000, executableSha256 = "a".repeat(64);
    const movement = { original_moves_completed: 20, concurrent_session_chains: 2, moves_per_source: [10, 10],
        active_av_routes: 6, maximum_pending_candidates: 2, shared_sdk_closed_during_replacement: false,
        original_program_start_unix_us: program, moves: [], physical_segments: [] };
    const plan = [], sessionIds = ["session-0", "session-1"];
    for (const sourceIndex of [0, 4]) {
        let current = connectionIds[sourceIndex];
        const sid = Number(sourceIndex === 4), peer = sourceIndex === 0 ? 3 : 5;
        for (let ordinal = 0; ordinal < 10; ordinal++) {
            const key = sourceIndex + "-" + ordinal, folder = join(directory, key);
            await mkdir(folder, { mode: 0o700 });
            const step = { sourceIndex, ordinal, operationId: "move-" + key,
                subscriptionId: "camera-" + key, audioId: "audio-" + key, videoId: "video-" + key };
            plan.push(step);
            const binding = (strm_id, kind, direction, stream_local_id) => ({ strm_id, kind, direction, stream_local_id,
                codec: { name: kind === "audio" ? "opus" : "vp8", params: {
                    sample_rate: kind === "audio" ? 48000 : 90000, channels: kind === "audio" ? 1 : 0,
                    payload_type: kind === "audio" ? 111 : 96 } } });
            const streams = [binding(step.audioId, "audio", "sendrecv", 7),
                binding(step.videoId, "video", "sendrecv", 8), binding(streamIds[peer].video, "video", "sendonly", 9)];
            const datagram = [1, 0, 0, 7, 0, 0, 0, 0, 0x80, 111, 0, 1, 0, 0, 3, 192, 0, 0, 0, 12, 1, 2];
            const observed = { format: "connect/native-replacement-peer-observation/1", passed: true,
                original_client_joined: true, video_enabled: true, original_root_completed_observed: true,
                video_controls_completed: true, operation_completed_inferred: false, decoded_media_verified: false,
                program_start_unix_us: program, negotiated_streams: streams,
                received: [{ elapsed_us: ordinal * 3_000_000 + 2_000_000, datagram, original_root_completed_observed: true }] };
            const next = "candidate-" + key;
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
                target_observation_sha256: captureHash });
            await save(join(folder, "original-reference.json"), { command: "connection.replace", operation_id: step.operationId,
                scope: { conversation_id: "room", session_id: sessionIds[sid], connection_id: current } });
            await save(join(folder, "admission.json"), { movement: {
                retired: [current], retained: [connectionIds[peer]], connection: next } });
            movement.moves.push({ source_index: sourceIndex, ordinal, session_index: sid, stage_directory: folder,
                original_reference_preserved: true, original_retired: true, camera_subscription_completed: true });
            movement.physical_segments.push({ sourceIndex, observation: join(folder, "target-observation.json"), sha256: captureHash });
            current = next;
        }
    }
    return { directory, options: { directory, movement, plan, packetWorkersJoined: true,
        conversationId: "room", sessionIds, connectionIds, streamIds, executableSha256 } };
}
async function run(change, check) {
    const { directory, options } = await fixture();
    try { if (change) await change(options); await check(options); }
    finally { await rm(directory, { recursive: true, force: true }); }
}
async function edit(options, name, change) {
    const path = join(options.directory, "0-0", name), value = JSON.parse(await readFile(path, "utf8"));
    change(value); await save(path, value);
}

test("twenty capture records bind two exact original chains without claiming decoded quality", () => run(undefined, async options => {
    const result = await physicalMovementSegments(options);
    assert.equal(result.original_move_records_bound, 20);
    assert.equal(result.segments.length, 20);
    assert.deepEqual(result.final_connections, [{ source_index: 0, connection_id: "candidate-0-9" },
        { source_index: 4, connection_id: "candidate-4-9" }]);
    assert.equal(result.segments[0].capture.audio[0].rows[0].elapsed_ms, 2000);
    assert.equal(result.segments[9].capture.audio[0].rows[0].elapsed_ms, 29000);
    assert.equal(result.segments[0].capture.video.length, 0, "fixture has no decoded or assembled video");
    assert.equal(result.full_timeline_decoded, false); assert.equal(result.cell_qualified, false);
}));

test("live packet owners and incomplete movement sets refuse before reading a directory", async () => {
    await assert.rejects(physicalMovementSegments({ packetWorkersJoined: false }), /packet workers must join/);
    await assert.rejects(physicalMovementSegments({ packetWorkersJoined: true, movement: { original_moves_completed: 19 } }));
});

test("original operation, Session and predecessor identity cannot be substituted", async () => {
    for (const change of [
        r => { r.operation_id = "different-original"; },
        r => { r.scope.session_id = "session-1"; },
        r => { r.scope.connection_id = "candidate-0-9"; },
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
