import { isolatedPrivateTargetFailure, blockedMovementSlots, privateTargetCleanup, emitOriginalObservation } from "./transport_failure_isolation.mjs";
// One switch and return per Session while the six-route AV owner is live.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdir, open } from "node:fs/promises";
import { join } from "node:path";
import { startPhysicalReplacementTarget } from "./transport_replacement_target.mjs";
import { liveVideoProgram } from "./transport_live_video_program.mjs";
import { operationFailure } from "./transport_failure.mjs";
import { inspectReplacementCapture } from "./transport_capture_inspection.mjs";
import { endRetainedConnections } from "./transport_current_end.mjs";
import { exchangeMovementData } from "./transport_movement_data.mjs";
import { setTimeout as sleep } from "node:timers/promises";
import { MOVE_START_OFFSETS_MS, MOVES_PER_SESSION, TOTAL_MOVES, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256 } from "./transport_movement_profile.mjs";

async function save(path, value) {
    const file = await open(path, "wx", 0o600);
    try { await file.writeFile(JSON.stringify(value, null, 2) + "\n"); await file.sync(); }
    finally { await file.close(); }
}

export async function deviceMovementWorkload({ workload, controllers, conversationId, sessions,
    connections, streamIds, participantIds, targets, sourcePrograms, privateRoot, directory,
    executable, executableSha256, currentEnds, dataClients, startTarget = startPhysicalReplacementTarget,
    endRetained = endRetainedConnections, exchangeData = exchangeMovementData, now = workload.now ?? (() => performance.now()),
    wait = (ms, signal) => sleep(ms, undefined, { signal }), observe = () => {} }) {
    assert.equal(workload.routes.length, 6);
    assert.equal(connections.length, 6);
    assert.equal(sessions.length, 2);
    // Each Node helper serializes its ordinary control RPCs. A long original
    // wait in one Session must not queue the other Session's original mutation.
    assert.equal(controllers.length, 2);
    assert.notEqual(controllers[0], controllers[1]);
    assert.ok(Array.isArray(streamIds));
    assert.equal(streamIds.length, 6);
    const dataWorkload = streamIds.some(streams => streams.data !== undefined);
    if (dataWorkload) {
        assert.ok(Array.isArray(dataClients) && dataClients.length === 6, "six original Data SDK owners are required");
        assert.ok(streamIds.every(streams => typeof streams.data === "string" &&
            /^[A-Za-z0-9._~-]{1,128}$/.test(streams.data)), "all six original Data resources are required");
        assert.equal(new Set(streamIds.flatMap(streams => [streams.audio, streams.video, streams.data])).size, 18);
    }
    assert.equal(targets.length, 2);
    assert.notEqual(targets[0].deviceId, targets[1].deviceId);
    assert.equal(workload.routes[0].role, workload.routes[4].role);
    assert.notEqual(workload.routes[0].sessionIndex, workload.routes[4].sessionIndex);
    assert.ok(Number.isSafeInteger(workload.programStartUnixUs));
    assert.ok(Number.isFinite(workload.startedAt) && workload.startedAt >= 0);
    assert.equal(currentEnds.length, 6);
    assert.deepEqual(currentEnds.map(end => end.index).sort((a, b) => a - b), [0, 1, 2, 3, 4, 5]);
    assert.ok(currentEnds.every(end => /^[A-Za-z0-9._~-]{1,128}$/.test(end.operationId)));
    assert.equal(new Set(currentEnds.map(end => end.operationId)).size, 6);
    const sources = [0, 4];
    assert.deepEqual(sources.map(index => workload.routes[index].sessionIndex), [0, 1]);
    for (const index of sources) {
        assert.equal(sourcePrograms[index].audioPackets.length, 3100);
        liveVideoProgram(sourcePrograms[index].videoProgram, 1860);
    }
    await mkdir(directory, { mode: 0o700 });
    const plan = sources.flatMap(sourceIndex => Array.from({ length: MOVES_PER_SESSION }, (_, ordinal) => ({
        sourceIndex, ordinal, targetIndex: (ordinal + 1) % 2, scheduled_start_ms: MOVE_START_OFFSETS_MS[ordinal],
        operationId: "move_" + randomUUID(), subscriptionId: "data_subscription_" + randomUUID(),
        audioId: "audio_" + randomUUID(), videoId: "video_" + randomUUID(),
        ...(dataWorkload ? { dataId: "data_" + randomUUID() } : {}),
    })));
    if (dataWorkload) for (const step of plan) {
        const route = workload.routes[step.sourceIndex], receiver = workload.routes.find(route => route.peerIndex === step.sourceIndex);
        const original = (direction, stream, from, to) => {
            const seed = (step.sourceIndex === 0 ? 0 : 20) + step.ordinal * 2 + direction;
            return { operation_id: "data_send_" + randomUUID(), message_id: "data_message_" + randomUUID(),
                stream_id: stream, from_participant: participantIds[from], to_participant: participantIds[to],
                body_base64: Buffer.from(Array.from({ length: 65536 }, (_, offset) => (offset * 13 + seed * 5) % 256)).toString("base64") };
        };
        step.data = { incoming: original(0, streamIds[route.peerIndex].data, route.peerIndex, step.sourceIndex),
            outgoing: original(1, step.dataId, step.sourceIndex, receiver.index) };
    }
    const originals = plan.flatMap(step => [step.operationId, step.subscriptionId,
        ...(step.data ? Object.values(step.data).map(original => original.operation_id) : [])]);
    originals.push(...currentEnds.map(end => end.operationId));
    assert.equal(new Set(originals).size, originals.length);
    if (dataWorkload) {
        const messages = plan.flatMap(step => Object.values(step.data));
        assert.equal(new Set(messages.map(original => original.message_id)).size, TOTAL_MOVES * 2);
        assert.equal(new Set(messages.map(original => original.body_base64)).size, TOTAL_MOVES * 2);
    }
    await save(join(directory, "plan.json"), plan);
    await save(join(directory, "current-end-plan.json"), {
        conversation_id: conversationId, session_ids: sessions.map(session => session.id), ends: currentEnds,
        original_program_start_unix_us: workload.programStartUnixUs,
        original_program_clock: workload.programClock,
        programme_end_monotonic_ms: workload.startedAt + 62000,
        original_cutoff_monotonic_ms: workload.startedAt + 67000,
    });
    const owners = new Set(), completed = [], segments = [], physicalEnds = [], dataExchanges = [], earlyJoins = new Map();
    const blocked = [], failedSlots = [];
    let isolatedFailure;
    const stopOwners = () => {
        for (const owner of owners) if (!earlyJoins.has(owner)) {
            const joining = Promise.resolve().then(() => owner.stopAndJoin());
            joining.catch(() => {});
            earlyJoins.set(owner, joining);
        }
    };
    let firstFailure, firstFailureStage;
    const stages = new Map();
    const stop = new AbortController(), abort = reason => { stop.abort(reason); stopOwners(); };
    const parentAbort = () => {
        const reason = workload.signal.reason;
        // A packet reader may observe the original control refusal before its
        // movement chain settles. Preserve that cause before cancelling the
        // retained-end wait or a sibling can report derivative cancellation.
        if (reason && reason.name !== "AbortError" && !firstFailure) {
            firstFailure = reason;
            firstFailureStage = { stage: "media-workload" };
        }
        abort(reason);
    };
    workload.signal.addEventListener("abort", parentAbort, { once: true });
    if (workload.signal.aborted) parentAbort();
    const assertCurrent = () => {
        if (stop.signal.aborted) throw firstFailure ?? Object.assign(Error("original movement workload cancelled"), { kind: "cancelled", certainty: "read-only" });
    };
    const collect = async (owner, sourceIndex, moveDirectory, natural = false) => {
        const stage = await (natural ? owner.join() : owner.stopAndJoin());
        assert.equal(stage.passed, true);
        if (dataWorkload) assert.equal(stage.physical_data_exchange_verified, true);
        owners.delete(owner);
        const observation = join(moveDirectory, "target-observation.json");
        const inspected = await inspectReplacementCapture(observation, 80 * 1024 * 1024, 16000);
        const captured = inspected.observation;
        assert.equal(captured.program_start_unix_us, workload.programStartUnixUs);
        assert.deepEqual(captured.program_clock, workload.programClock);
        assert.ok(captured.received.every(row => typeof row.original_root_completed_observed === "boolean"),
            "each physical packet must retain its actual original-root observation boundary");
        segments.push({ sourceIndex, observation, sha256: inspected.sha256 });
        if (natural) {
            const end = stage.final_current_owner_end;
            assert.equal(end?.operation_id, currentEnds.find(end => end.index === sourceIndex).operationId);
            assert.equal(end.connection_id, owner.connection.id);
            assert.equal(end.mutation_submissions, 1);
            assert.equal(end.original_cutoff_ms, 67000);
            assert.ok(end.elapsed_since_programme_end_ms >= 0 && end.elapsed_since_programme_end_ms < 5000);
            const generation = owner.movement.binding.generation;
            assert.ok(Number.isSafeInteger(generation) && generation > 0);
            const remaining = () => {
                assertCurrent(); const remaining = Math.floor(workload.startedAt + 67000 - now());
                assert.ok(remaining > 0 && remaining <= 5000, "original-physical-owner-retirement-cutoff"); return remaining;
            };
            let retired;
            for (;;) {
                try { retired = await owner.connection.refresh({ timeoutMs: remaining() }); }
                catch (failure) {
                    if (!(failure.problem?.code === 503 && failure.problem?.reason === "storage-unavailable")) throw failure;
                }
                if (retired) {
                    assert.equal(retired.id, owner.connection.id);
                    if (retired.state === "retired" && retired.generation > generation) break;
                }
                await new Promise(done => setTimeout(done, Math.min(50, remaining())));
            }
            remaining();
            const receipt = { index: sourceIndex, ...end, admitted_generation: generation,
                retired_generation: retired.generation, retirement_observed_ms: now() - workload.startedAt - 62000 };
            await save(join(moveDirectory, "final-current-retirement.json"), receipt); remaining();
            physicalEnds.push(receipt);
        }
        return stage;
    };
    async function chain(sourceIndex) {
        const route = workload.routes[sourceIndex], incoming = route.peerIndex;
        const receiver = workload.routes.find(v => v.peerIndex === sourceIndex);
        assert.ok(receiver);
        const sid = route.sessionIndex, source = sourcePrograms[sourceIndex], controller = controllers[sid];
        let current = connections[sourceIndex].connection, previous, previousDirectory;
        let currentCamera = streamIds[sourceIndex].video;
        for (const step of plan.filter(v => v.sourceIndex === sourceIndex)) {
            const stage = name => stages.set(sourceIndex, {
                source_index: sourceIndex, session_index: sid, ordinal: step.ordinal, stage: name,
            });
            stage("current-workload");
            assertCurrent();
            stage("scheduled-move");
            const due = workload.startedAt + step.scheduled_start_ms;
            while (now() < due) {
                const before = now();
                try { await wait(Math.max(1, Math.ceil(due - before)), stop.signal); }
                catch (failure) { assertCurrent(); throw failure; }
                assertCurrent();
                assert.ok(now() > before, "scheduled-movement-wait-must-advance-original-clock");
            }
            assertCurrent();
            assert.ok(now() - workload.startedAt >= step.scheduled_start_ms, "movement-cannot-start-before-scheduled-cadence");
            stage("session-refresh");
            const session = await sessions[sid].refresh({ timeoutMs: 8000 });
            stage("source-connection-refresh");
            const original = await current.refresh({ timeoutMs: 8000 });
            stage("retained-connection-refresh");
            const retained = await connections[incoming].connection.refresh({ timeoutMs: 8000 });
            const confirm = previous ? undefined : route.expectRetirement(step.operationId);
            const moveDirectory = join(directory, sourceIndex + "-" + step.ordinal);
            const began = now();
            assert.ok(began - workload.startedAt < 62000, "movement-must-start-inside-original-programme");
            assertCurrent();
            stage("physical-target");
            const owner = await startTarget({ controller, privateRoot, directory: moveDirectory,
                executable, executableSha256, operationId: step.operationId,
                conversationId, sessionId: sessions[sid].id, connectionId: current.id,
                retainedConnectionId: connections[incoming].connection.id,
                expectedTopologyRevision: session.topologyRevision,
                originalGeneration: original.generation, retainedGeneration: retained.generation,
                target: targets[step.targetIndex], streamId: step.audioId,
                ...(dataWorkload ? { dataStreamId: step.dataId } : {}),
                source: { seconds: 62, ssrc: 0x47000001 + sourceIndex * 256 + step.ordinal * 2,
                    audioPackets: source.audioPackets, programStartUnixUs: workload.programStartUnixUs, programClock: workload.programClock,
                    ...(step.ordinal === MOVES_PER_SESSION - 1 ? { terminalEndOperationId: currentEnds.find(end => end.index === sourceIndex).operationId } : {}),
                    ...(dataWorkload ? { data: step.data } : {}),
                    video: { streamId: step.videoId, previousStreamId: currentCamera,
                        previousReceiveVideoStreamId: streamIds[incoming].video, receiveAudioStreamId: streamIds[incoming].audio,
                        receiveVideoStreamId: streamIds[incoming].video,
                        ...(dataWorkload ? { receiveDataStreamId: streamIds[incoming].data } : {}),
                        ssrc: 0x47000002 + sourceIndex * 256 + step.ordinal * 2, program: source.videoProgram } },
                retireOriginal: async proof => {
                    stage("previous-owner-join");
                    if (previous) await collect(previous, sourceIndex, previousDirectory);
                    else { confirm(proof); await route.stopAndJoin(); }
                    stage("physical-target-readiness");
                },
            });
            owners.add(owner);
            current = owner.connection;
            currentCamera = step.videoId;
            previous = owner; previousDirectory = moveDirectory;
            const observedMove = { source_index: sourceIndex, session_index: sid, ordinal: step.ordinal,
                original_reference_preserved: owner.reference.operation_id === step.operationId,
                original_retired: owner.movement.retired.includes(original.id),
                camera_transfer_admitted: false, elapsed_ms: now() - began,
                scheduled_start_ms: step.scheduled_start_ms, actual_start_ms: began - workload.startedAt,
                data_exchange_verified: false,
                stage_directory: moveDirectory };
            completed.push(observedMove);
            assertCurrent();
            // The receiver's original video handle was retargeted by replacement.
            // Keep its original reader; only Data adds a separately authorized selection.
            if (dataWorkload) {
                stage("incoming-data-subscription");
                const currentReceiver = await receiver.connection.refresh({ timeoutMs: 8000 });
                const dataSubscription = await receiver.connection.streams.subscribe({ selectors: {
                    first: { stream: step.dataId }, rest: [],
                }, expectedStreamRevision: currentReceiver.streamRevision },
                { operationId: step.subscriptionId, timeoutMs: 8000 });
                await dataSubscription.wait({ timeoutMs: 8000 });
            }
            stage("live-session-refresh");
            const live = await sessions[sid].refresh({ timeoutMs: 8000 });
            assert.ok(!["ended", "failed", "cancelled"].includes(live.state));
            observedMove.camera_transfer_admitted = true;
            if (dataWorkload) {
                stage("original-physical-data-exchange");
                const exchange = await exchangeData({ workload, owner, rootOperationId: step.operationId,
                    contract: step.data, directory: moveDirectory, conversationId, sessionId: sessions[sid].id,
                    upstream: connections[incoming].connection, upstreamClient: dataClients[incoming],
                    downstream: receiver.connection, signal: stop.signal, now });
                dataExchanges.push(exchange); observedMove.data_exchange_verified = true;
            }
            observedMove.elapsed_ms = now() - began;
        }
        // The actual final peer ends its own admitted Connection at the
        // original62s boundary. Its joined receipt proves the current owner.
        stages.set(sourceIndex, { source_index: sourceIndex, session_index: sid, ordinal: MOVES_PER_SESSION - 1,
            stage: "final-natural-owner-join" });
        await collect(previous, sourceIndex, previousDirectory, true);
        return { sourceIndex, connection: current };
    }
    try {
        const retained = () => endRetained({ routes: workload.routes,
            ends: currentEnds.filter(end => !sources.includes(end.index)), conversationId, sessions,
            startedAt: workload.startedAt, signal: stop.signal, now,
            persist: (index, kind, value) => save(join(directory, "retained-" + index + "-" + kind + ".json"), value),
        });
        const tasks = [...sources.map(index => () => chain(index)), retained];
        const results = await Promise.allSettled(tasks.map((task, index) => Promise.resolve().then(task).catch(failure => {
            if (!firstFailure) {
                firstFailure = failure;
                firstFailureStage = stages.get(sources[index]) ?? { stage: "retained-current-end" };
                workload.reportFailure?.(firstFailure);
            }
            const failedStage = stages.get(sources[index]);
            if (index < sources.length && failedStage?.stage === "physical-target" && isolatedPrivateTargetFailure(failure)) {
                isolatedFailure ??= failure;
                const originalStep = plan.find(step => step.sourceIndex === sources[index] && step.ordinal === failedStage.ordinal);
                failedSlots.push({ ...failedStage, operation_id: originalStep.operationId,
                    scheduled_start_ms: originalStep.scheduled_start_ms, status: "failed",
                    failure: { name: failure.name, kind: failure.kind ?? null, certainty: failure.certainty ?? null,
                        ...operationFailure(failure) }, original_error_preserved: true,
                    cleanup: privateTargetCleanup(failure).facts });
                blocked.push(...blockedMovementSlots(plan, sources[index], failedStage.ordinal, now() - workload.startedAt));
                // No new root can follow a missing actual admitted predecessor.
                // The other Session and original retained62s owners stay live.
            } else {
                if (firstFailure !== failure) workload.reportFailure?.(failure, { fatal: true });
                abort(failure);
            }
            throw failure;
        })));
        if (firstFailure) throw firstFailure;
        assertCurrent();
        assert.ok(results.every(v => v.status === "fulfilled"));
        assert.equal(completed.length, TOTAL_MOVES);
        assert.equal(segments.length, TOTAL_MOVES);
        assert.equal(physicalEnds.length, 2);
        assert.equal(results[2].value.retained_connections_retired, 4);
        if (dataWorkload) {
            assert.equal(dataExchanges.length, TOTAL_MOVES);
            assert.ok(completed.every(move => move.data_exchange_verified));
        }
        assert.ok(completed.every(v => v.original_reference_preserved && v.original_retired && v.camera_transfer_admitted));
        const observation = { original_moves_completed: TOTAL_MOVES, concurrent_session_chains: 2,
            movement_profile: MOVEMENT_PROFILE, movement_profile_sha256: MOVEMENT_PROFILE_SHA256,
            scheduled_start_offsets_ms: MOVE_START_OFFSETS_MS,
            data_resource_contract_requested: dataWorkload,
            replacement_data_io_verified: dataWorkload,
            replacement_data_exchanges: dataExchanges,
            moves_per_source: [MOVES_PER_SESSION, MOVES_PER_SESSION], active_av_routes: 6, maximum_pending_candidates: 2,
            planned_peak_connection_slots: 8, shared_sdk_closed_during_replacement: false,
            original_program_start_unix_us: workload.programStartUnixUs,
        original_program_clock: workload.programClock,
            current_owner_end: { physical: physicalEnds, retained: results[2].value,
                current_connections_ended: 6, original_cutoff_monotonic_ms: workload.startedAt + 67000 },
            moves: completed, physical_segments: segments,
            full_timeline_decoded: false, cell_qualified: false, task_complete: false, whole_gate_qualified: false };
        await save(join(directory, "movement.json"), observation);
        return { observation, currentConnections: results.slice(0, 2).map(v => v.value) };
    } catch (failure) { firstFailure ??= failure; throw failure; }
    finally {
        workload.signal.removeEventListener("abort", parentAbort);
        stopOwners();
        const joins = await Promise.allSettled([...earlyJoins.values()]);
        const failure = joins.find(v => v.status === "rejected");
        if (failure) { workload.reportFailure?.(failure.reason, { fatal: true }); abort(failure.reason); }
        if (firstFailure) {
        const failureObservation = {
            passed: false, original_moves_completed: completed.length, moves: completed,
            movement_profile: MOVEMENT_PROFILE, movement_profile_sha256: MOVEMENT_PROFILE_SHA256,
            scheduled_start_offsets_ms: MOVE_START_OFFSETS_MS,
            physical_segments: segments, failure: { name: firstFailure.name,
                kind: firstFailure.kind ?? null, certainty: firstFailure.certainty ?? null,
                observation: firstFailureStage ?? null, ...operationFailure(firstFailure) },
            target_joins_failed: joins.filter(v => v.status === "rejected").length,
            failed_slots: failedSlots, blocked_moves: blocked,
            failure_isolated: Boolean(isolatedFailure && !stop.signal.aborted),
            original_cutoff_monotonic_ms: workload.startedAt + 67000,
            task_complete: false, whole_gate_qualified: false,
        };
        await save(join(directory, "movement-failure.json"), failureObservation);
        const emitted = emitOriginalObservation(observe, [Object.freeze(failureObservation)], firstFailure);
        if (!emitted.emitted) { workload.reportFailure?.(emitted.error, { fatal: true }); abort(emitted.error); throw emitted.error; }
        }
        if (failure && !firstFailure) throw failure.reason;
    }
}
