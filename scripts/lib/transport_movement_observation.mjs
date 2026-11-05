// Offline custody checks for already completed original movement operations.
import assert from "node:assert/strict";
import { constants } from "node:fs";
import { open, realpath } from "node:fs/promises";
import { resolve, join } from "node:path";
import { createHash } from "node:crypto";
import { physicalAvSegment } from "./transport_replacement_observation.mjs";
import { physicalDataCustody } from "./transport_movement_data_observation.mjs";
import { MOVE_START_OFFSETS_MS, MOVES_PER_SESSION, TOTAL_MOVES, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256 } from "./transport_movement_profile.mjs";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
async function originalFile(path, maximum) {
    assert.equal(await realpath(path), path, "capture evidence cannot follow a substituted path");
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
        const before = await file.stat();
        assert.ok(before.isFile() && !(before.mode & 0o077) && before.size > 0 && before.size <= maximum);
        const bounded = Buffer.alloc(before.size + 1);
        let count = 0;
        while (count < bounded.length) {
            const read = await file.read(bounded, count, bounded.length - count, count);
            if (!read.bytesRead) break;
            count += read.bytesRead;
        }
        const bytes = bounded.subarray(0, count), after = await file.stat();
        for (const key of ["dev", "ino", "size", "mtimeMs", "ctimeMs"]) assert.equal(before[key], after[key]);
        assert.equal(bytes.length, before.size);
        return { bytes, digest: sha(bytes), value: JSON.parse(bytes) };
    } finally { await file.close(); }
}

export async function physicalMovementSegments({ directory, movement, plan,
    packetWorkersJoined, conversationId, sessionIds, connectionIds, streamIds, executableSha256 }) {
    assert.equal(packetWorkersJoined, true, "original packet workers must join before offline interpretation");
    assert.equal(movement.original_moves_completed, TOTAL_MOVES);
    assert.equal(movement.movement_profile, MOVEMENT_PROFILE);
    assert.equal(movement.movement_profile_sha256, MOVEMENT_PROFILE_SHA256);
    assert.deepEqual(movement.scheduled_start_offsets_ms, MOVE_START_OFFSETS_MS);
    assert.equal(movement.concurrent_session_chains, 2);
    assert.deepEqual(movement.moves_per_source, [MOVES_PER_SESSION, MOVES_PER_SESSION]);
    assert.equal(movement.active_av_routes, 6);
    assert.equal(movement.maximum_pending_candidates, 2);
    assert.equal(movement.shared_sdk_closed_during_replacement, false);
    assert.ok(Number.isSafeInteger(movement.original_program_start_unix_us) && movement.original_program_start_unix_us > 0);
    assert.ok(id(conversationId) && sessionIds.length === 2 && sessionIds.every(id));
    assert.equal(new Set(sessionIds).size, 2);
    assert.equal(connectionIds.length, 6); assert.ok(connectionIds.every(id));
    assert.equal(new Set(connectionIds).size, 6);
    assert.equal(streamIds.length, 6);
    assert.ok(streamIds.every(v => id(v.audio) && id(v.video)));
    assert.match(executableSha256, /^[0-9a-f]{64}$/);
    assert.equal(plan.length, TOTAL_MOVES); assert.equal(movement.moves.length, TOTAL_MOVES);
    const dataExpected = streamIds.some(streams => streams.data !== undefined);
    const dataIoExpected = movement.replacement_data_io_verified === true;
    if (dataIoExpected) {
        assert.equal(dataExpected, true); assert.equal(movement.replacement_data_exchanges.length, TOTAL_MOVES);
    }
    if (dataExpected) {
        assert.ok(streamIds.every(streams => id(streams.data)), "every original Data identity is required");
        assert.ok(plan.every(step => id(step.dataId)), "every replacement must retain its Data contract");
    } else assert.ok(plan.every(step => step.dataId === undefined), "Data captures need the original Data profile");
    assert.equal(movement.physical_segments.length, TOTAL_MOVES);
    const keys = new Set(), operations = new Set(), streamNames = new Set();
    for (const step of plan) {
        assert.ok([0, 4].includes(step.sourceIndex) && Number.isInteger(step.ordinal) && step.ordinal >= 0 && step.ordinal < MOVES_PER_SESSION);
        assert.equal(step.scheduled_start_ms, MOVE_START_OFFSETS_MS[step.ordinal]);
        const key = step.sourceIndex + "-" + step.ordinal;
        assert.ok(!keys.has(key)); keys.add(key);
        assert.ok(id(step.operationId) && !operations.has(step.operationId)); operations.add(step.operationId);
        for (const name of [step.subscriptionId, step.audioId, step.videoId, ...(dataExpected ? [step.dataId] : [])]) {
            assert.ok(id(name) && !streamNames.has(name), "movement identities must be unique across the original plan");
            streamNames.add(name);
        }
    }
    const root = resolve(directory);
    assert.ok(root.split("/").includes(".local"));
    assert.equal(await realpath(root), root);
    const segments = [], current = new Map([[0, connectionIds[0]], [4, connectionIds[4]]]);
    const usedConnections = new Set(connectionIds), usedCaptures = new Set(), usedMoves = new Set();
    for (const sourceIndex of [0, 4]) for (let ordinal = 0; ordinal < MOVES_PER_SESSION; ordinal++) {
        const step = plan.find(v => v.sourceIndex === sourceIndex && v.ordinal === ordinal);
        assert.ok(step);
        const folder = join(root, sourceIndex + "-" + ordinal), sid = sourceIndex === 0 ? 0 : 1;
        const peer = sourceIndex === 0 ? 3 : 5;
        const moves = movement.moves.filter(v => v.source_index === sourceIndex && v.ordinal === ordinal);
        assert.equal(moves.length, 1);
        const move = moves[0]; usedMoves.add(move);
        assert.equal(move.scheduled_start_ms, MOVE_START_OFFSETS_MS[ordinal]);
        assert.ok(Number.isFinite(move.actual_start_ms) && move.actual_start_ms >= move.scheduled_start_ms && move.actual_start_ms < 62000);
        assert.equal(move.session_index, sid);
        assert.equal(resolve(move.stage_directory), folder);
        for (const flag of ["original_reference_preserved", "original_retired", "camera_transfer_admitted"])
            assert.equal(move[flag], true, flag);
        const path = join(folder, "target-observation.json");
        const captures = movement.physical_segments.filter(v => v.sourceIndex === sourceIndex && resolve(v.observation) === path);
        assert.equal(captures.length, 1);
        const capture = captures[0]; usedCaptures.add(capture);
        const stage = await originalFile(join(folder, "stage.json"), 65536);
        assert.equal(stage.value.format, "connect/physical-native-replacement-stage/1");
        for (const flag of ["passed", "original_reference_preserved", "original_root_completed",
            "original_source_owner_joined", "target_worker_joined", "target_config_unchanged", "target_executable_unchanged"])
            assert.equal(stage.value[flag], true, flag);
        assert.equal(stage.value.target_worker_forced, false);
        assert.equal(stage.value.target_executable_sha256, executableSha256);
        const reference = await originalFile(join(folder, "original-reference.json"), 65536);
        assert.equal(reference.value.command, "connection.replace");
        assert.equal(reference.value.operation_id, step.operationId);
        assert.equal(reference.value.scope.conversation_id, conversationId);
        assert.equal(reference.value.scope.session_id, sessionIds[sid]);
        assert.equal(reference.value.scope.connection_id, current.get(sourceIndex));
        const admission = await originalFile(join(folder, "admission.json"), 65536);
        const moved = admission.value.movement;
        assert.deepEqual(moved.retired, [current.get(sourceIndex)]);
        assert.deepEqual(moved.retained, [connectionIds[peer]]);
        assert.ok(id(moved.connection) && !usedConnections.has(moved.connection));
        usedConnections.add(moved.connection);
        const ready = await originalFile(join(folder, "target-observation.owner-ready.json"), 65536);
        assert.equal(ready.digest, stage.value.target_owner_ready_sha256);
        assert.equal(ready.value.format, "connect/native-replacement-owner-ready/1");
        assert.equal(ready.value.operation_id, step.operationId);
        assert.equal(ready.value.connection_id, moved.connection);
        assert.equal(ready.value.video_enabled, true);
        assert.equal(ready.value.operation_completed_inferred, false);
        assert.equal(ready.value.decoded_media_verified, false);
        const observed = await originalFile(path, 80 * 1024 * 1024);
        assert.equal(observed.digest, capture.sha256);
        assert.equal(observed.digest, stage.value.target_observation_sha256);
        assert.deepEqual(observed.value.negotiated_streams, ready.value.negotiated_streams);
        const normalized = physicalAvSegment(observed.value, {
            programStartUnixUs: movement.original_program_start_unix_us,
            connectionId: moved.connection, targetGeneration: moved.binding.generation,
            audioStreamId: step.audioId, sendVideoStreamId: step.videoId,
            receiveVideoStreamId: streamIds[peer].video,
            ...(step.dataId === undefined ? {} : { sendDataStreamId: step.dataId,
                receiveDataStreamId: streamIds[peer].data }),
        });
        assert.ok(normalized.root_completed_observed_programme_us >= move.scheduled_start_ms * 1000,
            "captured-root-observation-cannot-precede-scheduled-move");
        let reliableData;
        if (dataIoExpected) {
            const exchanges = movement.replacement_data_exchanges.filter(exchange => exchange.root_operation_id === step.operationId);
            assert.equal(exchanges.length, 1);
            reliableData = await physicalDataCustody({ readOriginal: originalFile, folder, stage: stage.value,
                observed: observed.value, step, conversationId, sessionId: sessionIds[sid], connectionId: moved.connection,
                predecessor: current.get(sourceIndex), upstream: connectionIds[peer], downstream: connectionIds[sourceIndex === 0 ? 1 : 5],
                expectedExchange: exchanges[0] });
        }
        segments.push({ source_index: sourceIndex, ordinal, operation_id: step.operationId,
            scheduled_start_ms: move.scheduled_start_ms, actual_start_ms: move.actual_start_ms,
            connection_id: moved.connection, original_capture_sha256: observed.digest,
            published_stream_ids: { audio: step.audioId, video: step.videoId, ...(step.dataId ? { data: step.dataId } : {}) },
            custody_sha256: { stage: stage.digest, reference: reference.digest,
                admission: admission.digest, ready: ready.digest }, capture: normalized,
            ...(reliableData ? { reliable_data: reliableData } : {}) });
        current.set(sourceIndex, moved.connection);
    }
    assert.equal(usedMoves.size, TOTAL_MOVES); assert.equal(usedCaptures.size, TOTAL_MOVES);
    return { format: "connect/physical-movement-segments/1", original_move_records_bound: TOTAL_MOVES,
        movement_profile: MOVEMENT_PROFILE, movement_profile_sha256: MOVEMENT_PROFILE_SHA256,
        scheduled_start_offsets_ms: MOVE_START_OFFSETS_MS,
        program_start_unix_us: movement.original_program_start_unix_us, segments,
        original_connection_ids: connectionIds, session_ids: sessionIds, original_stream_ids: streamIds,
        reliable_data_exchanges_bound: dataIoExpected ? TOTAL_MOVES : 0,
        final_connections: [...current].map(([source_index, connection_id]) => ({ source_index, connection_id })),
        full_timeline_decoded: false, cell_qualified: false, task_complete: false,
        scope: "Offline custody and original packet interpretation only. These four target segments still require the original six captures, separate physical codec histories and independent whole-window decoding. No decoded quality or transport qualification." };
}
