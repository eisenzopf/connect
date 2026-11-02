// Twenty original physical replacements while the six-route AV owner is live.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdir, open } from "node:fs/promises";
import { join } from "node:path";
import { startPhysicalReplacementTarget } from "./transport_replacement_target.mjs";
import { operationFailure } from "./transport_failure.mjs";
import { inspectReplacementCapture } from "./transport_capture_inspection.mjs";

async function save(path, value) {
    const file = await open(path, "wx", 0o600);
    try { await file.writeFile(JSON.stringify(value, null, 2) + "\n"); await file.sync(); }
    finally { await file.close(); }
}

export async function deviceMovementWorkload({ workload, controllers, conversationId, sessions,
    connections, streamIds, participantIds, targets, sourcePrograms, privateRoot, directory,
    executable, executableSha256, startTarget = startPhysicalReplacementTarget }) {
    assert.equal(workload.routes.length, 6);
    assert.equal(connections.length, 6);
    assert.equal(sessions.length, 2);
    // Each Node helper serializes its ordinary control RPCs. A long original
    // wait in one Session must not queue the other Session's original mutation.
    assert.equal(controllers.length, 2);
    assert.notEqual(controllers[0], controllers[1]);
    assert.equal(targets.length, 2);
    assert.notEqual(targets[0].deviceId, targets[1].deviceId);
    assert.equal(workload.routes[0].role, workload.routes[4].role);
    assert.notEqual(workload.routes[0].sessionIndex, workload.routes[4].sessionIndex);
    assert.ok(Number.isSafeInteger(workload.programStartUnixUs));
    const sources = [0, 4];
    assert.deepEqual(sources.map(index => workload.routes[index].sessionIndex), [0, 1]);
    for (const index of sources) {
        assert.equal(sourcePrograms[index].audioPackets.length, 3100);
        assert.equal(sourcePrograms[index].videoFrames.length, 1860);
    }
    await mkdir(directory, { mode: 0o700 });
    const plan = sources.flatMap(sourceIndex => Array.from({ length: 10 }, (_, ordinal) => ({
        sourceIndex, ordinal, targetIndex: (ordinal + 1) % 2,
        operationId: "move_" + randomUUID(), subscriptionId: "camera_" + randomUUID(),
        audioId: "audio_" + randomUUID(), videoId: "video_" + randomUUID(),
    })));
    await save(join(directory, "plan.json"), plan);
    const owners = new Set(), completed = [], segments = [], earlyJoins = new Map();
    const stopOwners = () => {
        for (const owner of owners) if (!earlyJoins.has(owner)) {
            const joining = Promise.resolve().then(() => owner.stopAndJoin());
            joining.catch(() => {});
            earlyJoins.set(owner, joining);
        }
    };
    const stop = new AbortController(), abort = () => { stop.abort(); stopOwners(); };
    workload.signal.addEventListener("abort", abort, { once: true });
    if (workload.signal.aborted) abort();
    let firstFailure, firstFailureStage;
    const stages = new Map();
    const assertCurrent = () => {
        if (stop.signal.aborted) throw firstFailure ?? Object.assign(Error("original movement workload cancelled"), { kind: "cancelled", certainty: "read-only" });
    };
    const collect = async (owner, sourceIndex, moveDirectory, natural = false) => {
        const stage = await (natural ? owner.join() : owner.stopAndJoin());
        assert.equal(stage.passed, true);
        owners.delete(owner);
        const observation = join(moveDirectory, "target-observation.json");
        const inspected = await inspectReplacementCapture(observation, 80 * 1024 * 1024, 16000);
        const captured = inspected.observation;
        assert.equal(captured.program_start_unix_us, workload.programStartUnixUs);
        assert.ok(captured.received.every(row => typeof row.original_root_completed_observed === "boolean"),
            "each physical packet must retain its actual original-root observation boundary");
        segments.push({ sourceIndex, observation, sha256: inspected.sha256 });
        return stage;
    };
    async function chain(sourceIndex) {
        const route = workload.routes[sourceIndex], incoming = route.peerIndex;
        const receiver = workload.routes.find(v => v.peerIndex === sourceIndex);
        assert.ok(receiver);
        const sid = route.sessionIndex, source = sourcePrograms[sourceIndex], controller = controllers[sid];
        let current = connections[sourceIndex].connection, previous, previousDirectory;
        for (const step of plan.filter(v => v.sourceIndex === sourceIndex)) {
            const stage = name => stages.set(sourceIndex, {
                source_index: sourceIndex, session_index: sid, ordinal: step.ordinal, stage: name,
            });
            stage("current-workload");
            assertCurrent();
            stage("session-refresh");
            const session = await sessions[sid].refresh({ timeoutMs: 8000 });
            stage("source-connection-refresh");
            const original = await current.refresh({ timeoutMs: 8000 });
            stage("retained-connection-refresh");
            const retained = await connections[incoming].connection.refresh({ timeoutMs: 8000 });
            const confirm = previous ? undefined : route.expectRetirement(step.operationId);
            const moveDirectory = join(directory, sourceIndex + "-" + step.ordinal);
            const began = performance.now();
            assertCurrent();
            stage("physical-target");
            const owner = await startTarget({ controller, privateRoot, directory: moveDirectory,
                executable, executableSha256, operationId: step.operationId,
                conversationId, sessionId: sessions[sid].id, connectionId: current.id,
                retainedConnectionId: connections[incoming].connection.id,
                expectedTopologyRevision: session.topologyRevision,
                originalGeneration: original.generation, retainedGeneration: retained.generation,
                target: targets[step.targetIndex], streamId: step.audioId,
                source: { seconds: 62, ssrc: 0x47000001 + sourceIndex * 256 + step.ordinal * 2,
                    audioPackets: source.audioPackets, programStartUnixUs: workload.programStartUnixUs,
                    video: { streamId: step.videoId, receiveAudioStreamId: streamIds[incoming].audio,
                        receiveVideoStreamId: streamIds[incoming].video,
                        ssrc: 0x47000002 + sourceIndex * 256 + step.ordinal * 2, frames: source.videoFrames } },
                retireOriginal: async proof => {
                    stage("previous-owner-join");
                    if (previous) await collect(previous, sourceIndex, previousDirectory);
                    else { confirm(proof); await route.stopAndJoin(); }
                    stage("physical-target-readiness");
                },
            });
            owners.add(owner);
            current = owner.connection;
            previous = owner; previousDirectory = moveDirectory;
            const observedMove = { source_index: sourceIndex, session_index: sid, ordinal: step.ordinal,
                original_reference_preserved: owner.reference.operation_id === step.operationId,
                original_retired: owner.movement.retired.includes(original.id),
                camera_subscription_completed: false, elapsed_ms: performance.now() - began,
                stage_directory: moveDirectory };
            completed.push(observedMove);
            assertCurrent();
            stage("incoming-video-subscription");
            await receiver.switchIncomingVideo(step.subscriptionId, step.videoId, async () => {
                const currentReceiver = await receiver.connection.refresh({ timeoutMs: 8000 });
                return receiver.connection.streams.subscribe({ selectors: {
                    first: { participant: participantIds[sourceIndex], kinds: { first: "audio", rest: [] } },
                    rest: [{ stream: step.videoId }],
                }, expectedStreamRevision: currentReceiver.streamRevision },
                { operationId: step.subscriptionId, timeoutMs: 8000 });
            });
            stage("live-session-refresh");
            const live = await sessions[sid].refresh({ timeoutMs: 8000 });
            assert.ok(!["ended", "failed", "cancelled"].includes(live.state));
            observedMove.camera_subscription_completed = true;
            observedMove.elapsed_ms = performance.now() - began;
        }
        // The final physical owners keep their original program clock and
        // natural62second fail-safe. The caller still owns Session end/drain.
        stages.set(sourceIndex, { source_index: sourceIndex, session_index: sid, ordinal: 9,
            stage: "final-natural-owner-join" });
        await collect(previous, sourceIndex, previousDirectory, true);
        return { sourceIndex, connection: current };
    }
    try {
        const results = await Promise.allSettled(sources.map(index => chain(index).catch(failure => {
            if (!firstFailure) { firstFailure = failure; firstFailureStage = stages.get(index); }
            abort(); throw failure;
        })));
        if (firstFailure) throw firstFailure;
        assertCurrent();
        assert.ok(results.every(v => v.status === "fulfilled"));
        assert.equal(completed.length, 20);
        assert.equal(segments.length, 20);
        assert.ok(completed.every(v => v.original_reference_preserved && v.original_retired && v.camera_subscription_completed));
        const observation = { original_moves_completed: 20, concurrent_session_chains: 2,
            moves_per_source: [10, 10], active_av_routes: 6, maximum_pending_candidates: 2,
            planned_peak_connection_slots: 8, shared_sdk_closed_during_replacement: false,
            original_program_start_unix_us: workload.programStartUnixUs,
            moves: completed, physical_segments: segments,
            full_timeline_decoded: false, cell_qualified: false, task_complete: false, whole_gate_qualified: false };
        await save(join(directory, "movement.json"), observation);
        return { observation, currentConnections: results.map(v => v.value) };
    } catch (failure) { firstFailure ??= failure; throw failure; }
    finally {
        workload.signal.removeEventListener("abort", abort);
        stopOwners();
        const joins = await Promise.allSettled([...earlyJoins.values()]);
        const failure = joins.find(v => v.status === "rejected");
        if (firstFailure) await save(join(directory, "movement-failure.json"), {
            passed: false, original_moves_completed: completed.length, moves: completed,
            physical_segments: segments, failure: { name: firstFailure.name,
                kind: firstFailure.kind ?? null, certainty: firstFailure.certainty ?? null,
                observation: firstFailureStage ?? null, ...operationFailure(firstFailure) },
            target_joins_failed: joins.filter(v => v.status === "rejected").length,
            task_complete: false, whole_gate_qualified: false,
        });
        if (failure && !firstFailure) throw failure.reason;
    }
}
