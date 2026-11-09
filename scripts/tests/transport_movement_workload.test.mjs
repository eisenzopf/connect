import { recordPrivateTargetCleanup } from "../lib/transport_failure_isolation.mjs";
import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
import { deviceMovementWorkload } from "../lib/transport_movement_workload.mjs";
import { MOVE_START_OFFSETS_MS, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256 } from "../lib/transport_movement_profile.mjs";

async function scenario(mode) {
    const root = resolve(".local"); await mkdir(root, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(root, "movement-controller-"));
    const signal = new AbortController(), starts = [], admitted = [], subscriptions = [], stopped = [], joined = [], retired = [], finalWaits = new Map();
    let clock = 0;
    const clockWaits = [];
    const roles = ["owner", "two", "three", "four", "owner", "two"], peers = [3, 0, 1, 2, 5, 4];
    const revisions = [1, 1];
    const physicalRetired = new Set();
    const controllers = [{ owner: "existing-original-helper" }, { owner: "existing-controller-helper" }];
    const connection = (id, generation) => ({ id, async refresh() {
        if (mode === "retirement-refusal" && physicalRetired.has(id)) throw Object.assign(Error("current metadata refused"), { kind: "authorization", certainty: "read-only" });
        return { id, generation: generation + Number(physicalRetired.has(id)), state: physicalRetired.has(id) ? "retired" : "active", streamRevision: 2 }; },
        streams: { async subscribe(intent, options) {
            subscriptions.push({ intent, options });
            return { async wait() { return { snapshot: { state: "completed" } }; } };
        } } });
    const connections = roles.map((role, i) => ({ connection: connection("original-" + i, 1) }));
    const dataProfile = ["data", "partial-data", "data-failure", "prepared-video-data"].includes(mode);
    const streamIds = roles.map((_, i) => ({ audio: "audio-" + i, video: "video-" + i,
        ...(dataProfile && !(mode === "partial-data" && i === 5) ? { data: "data-" + i } : {}) }));
    const routes = roles.map((role, i) => ({ index: i, peerIndex: peers[i], sessionIndex: Number(i >= 4), role,
        connection: connections[i].connection,
        expectRetirement(operationId) { return proof => {
            assert.equal(proof.reference.operation_id, operationId); retired.push(i);
        }; },
        async stopAndJoin() { assert.ok(retired.includes(i)); },
        async switchIncomingVideo() { assert.fail("replacement must preserve the existing video reader"); },
    }));
    const sessions = [0, 1].map(i => ({ id: "session-" + i,
        async refresh() { return { state: "accepted", topologyRevision: revisions[i] }; } }));
    const original = Object.assign(Error("original unknown move"), mode === "isolated-outbound" ?
        { kind: "authorization", certainty: "refused" } : { kind: "deadline", certainty: "uncertain" });
    const reported = [];
    const workload = { routes, signal: signal.signal, reportFailure(failure) {
        reported.push({ failure, stopped: stopped.length });
    }, programStartUnixUs: 1793600000000000,
        programClock: {kind: "linux-monotonic-v1", domain: "fixture", origin_ns: "1"}, startedAt: 0 };
    const currentEnds = roles.map((_, index) => ({ index, operationId: "end-" + index }));
    let retainedJoined = false;
    const endRetained = async args => {
        const saved = JSON.parse(await readFile(join(directory, "run/current-end-plan.json"), "utf8"));
        assert.deepEqual(saved.ends, currentEnds);
        assert.deepEqual(args.ends.map(end => end.index), [1, 2, 3, 5]);
        assert.equal(args.startedAt, 0);
        retainedJoined = true;
        if (mode === "external-failure") return new Promise((_, reject) => {
            if (args.signal.aborted) reject(args.signal.reason);
            else args.signal.addEventListener("abort", () => reject(args.signal.reason), { once: true });
        });
        if (mode === "retained-failure") throw original;
        return { retained_connections_retired: 4, original_end_mutations: 4 };
    };
    if (mode === "cancelled") signal.abort();
    const startTarget = async options => {
        const sid = Number(options.sessionId.at(-1)), ordinal = starts.filter(v => v.sessionId === options.sessionId).length;
        assert.ok(clock >= MOVE_START_OFFSETS_MS[ordinal] && clock < 62000);
        assert.equal(options.controller, controllers[sid]);
        starts.push(options);
        if (mode === "external-failure" && sid === 0) {
            signal.abort(original);
            await new Promise(setImmediate);
            throw original;
        }
        const saved = JSON.parse(await readFile(join(directory, "run/plan.json"), "utf8"));
        assert.equal(saved.length, 4);
        assert.equal(saved.filter(v => v.operationId === options.operationId).length, 1);
        assert.equal(options.expectedTopologyRevision, revisions[sid]);
        assert.equal(options.source.programStartUnixUs, workload.programStartUnixUs);
        const sourceIndex = sid === 0 ? 0 : 4;
        const prior = starts.filter(v => v.sessionId === options.sessionId).at(-2);
        assert.equal(options.source.video.previousStreamId, prior?.source.video.streamId ?? streamIds[sourceIndex].video);
        assert.equal(options.source.video.previousReceiveVideoStreamId, streamIds[peers[sourceIndex]].video);
        if (dataProfile) {
            assert.ok(options.dataStreamId.startsWith("data_"));
            const sourceIndex = sid === 0 ? 0 : 4;
            assert.equal(options.source.video.receiveDataStreamId, streamIds[peers[sourceIndex]].data);
            assert.equal(saved.find(step => step.operationId === options.operationId).dataId, options.dataStreamId);
        }
        assert.equal(options.target.deviceId, ordinal % 2 ? "original-device" : "replacement-device");
        if (mode === "isolated-outbound" && sid === 0 && ordinal === 0) {
            recordPrivateTargetCleanup(original, { format: "connect/original-private-target-cleanup/1", logical_outcome: "not-submitted", submission_definitively_refused: true, completion_observed: false,
                transferred: false, original_root_completed: false, original_source_owner_joined: false,
                worker_started: false, target_worker_joined: true, target_worker_forced: false,
                target_executable_unchanged: true, target_config_unchanged: true });
            throw original;
        }
        if (mode === "unknown" && sid === 0 && ordinal === 1) throw original;
        revisions[sid]++;
        const moved = { retired: [options.connectionId], binding: { generation: options.originalGeneration + 1 } };
        const reference = { command: "connection.replace", operation_id: options.operationId,
            scope: { connection_id: options.connectionId } };
        await options.retireOriginal({ reference, original: { id: options.connectionId,
            state: "retired", generation: options.originalGeneration + 1 }, movement: moved });
        await mkdir(options.directory);
        await writeFile(join(options.directory, "target-observation.json"), JSON.stringify({
            program_start_unix_us: workload.programStartUnixUs,
            program_clock: workload.programClock,
            received: [{ original_root_completed_observed: false }, { original_root_completed_observed: true }],
        }), { mode: 0o600 });
        const capturedBytes = await readFile(join(options.directory, "target-observation.json"));
        const stage = { passed: true, capture_inspection_worker_joined: true,
            capture_root_boundaries_verified: true,
            target_observation_sha256: createHash("sha256").update(capturedBytes).digest("hex"),
            original_program_start_unix_us: workload.programStartUnixUs,
            original_program_clock: structuredClone(workload.programClock) };
        if (mode === "reuse-inspection") await rm(join(options.directory, "target-observation.json"));
        if (mode === "missing-root-verification") delete stage.capture_root_boundaries_verified;
        if (mode === "unjoined-inspector") stage.capture_inspection_worker_joined = false;
        if (mode === "foreign-inspected-clock") stage.original_program_clock.domain = "foreign";
        if (mode === "invalid-inspected-hash") stage.target_observation_sha256 = "not-a-hash";
        if (dataProfile) stage.physical_data_exchange_verified = true;
        if (ordinal === 1) {
            assert.equal(options.source.terminalEndOperationId, "end-" + (sid === 0 ? 0 : 4));
            stage.final_current_owner_end = { operation_id: options.source.terminalEndOperationId,
                connection_id: "candidate-" + sid + "-" + ordinal, mutation_submissions: 1,
                original_cutoff_ms: 67000, elapsed_since_programme_end_ms: 200 };
            if (mode === "foreign-final-end" && sid === 0) stage.final_current_owner_end.connection_id = "foreign";
        } else assert.equal(options.source.terminalEndOperationId, undefined);
        let result;
        admitted.push(options.operationId);
        return { reference, movement: moved, connection: connection("candidate-" + sid + "-" + ordinal, options.originalGeneration + 1),
            async stopAndJoin() {
                if (!result) { stopped.push(options.operationId); result = stage; }
                finalWaits.get(options.operationId)?.(result);
                return result;
            },
            async join() {
                if (mode === "cancel-after4") return new Promise(done => {
                    joined.push(options.operationId); finalWaits.set(options.operationId, done);
                    if (finalWaits.size === 2) setImmediate(() => signal.abort());
                });
                if (!result) { joined.push(options.operationId); result = stage; }
                return new Promise(done => {
                    finalWaits.set(options.operationId, () => { physicalRetired.add("candidate-" + sid + "-" + ordinal); done(result); });
                    if (finalWaits.size === (mode === "isolated-outbound" ? 1 : 2)) {
                        clock = 62500; for (const finish of finalWaits.values()) finish();
                    }
                });
            } };
    };
    try {
        const run = deviceMovementWorkload({ workload, controllers, conversationId: "original-room", sessions,
            connections, streamIds,
            participantIds: roles.map(v => "participant-" + v),
            targets: [{ deviceId: "original-device" }, { deviceId: "replacement-device" }],
            sourcePrograms: Object.fromEntries([0, 4].map(i => [i, { audioPackets: Array(3100).fill([1]), videoProgram: { executable: "/private/.local/encoder", executable_sha256: "a".repeat(64), raw_path: "/private/.local/source.i420", raw_sha256: "b".repeat(64), frame_count: 1860 } }])),
            privateRoot: root, directory: join(directory, "run"), executable: "unexecuted-test-double",
            executableSha256: "0".repeat(64), currentEnds, startTarget, endRetained, now: () => clock,
            wait: async (ms, abort) => {
                assert.equal(abort.aborted, false); clockWaits.push(ms);
                if (mode === "cancel-during-wait") signal.abort();
                else if (mode !== "no-progress-wait") clock += ms - Number(mode === "early-wake" && clockWaits.length === 1) * 0.25;
                await Promise.resolve();
            },
            dataClients: connections.map(() => ({})),
            exchangeData: async args => {
                const saved = JSON.parse(await readFile(join(directory, "run/plan.json"), "utf8"));
                const step = saved.find(step => step.operationId === args.rootOperationId);
                assert.ok(subscriptions.some(subscription => subscription.options.operationId === step.subscriptionId));
                assert.equal(args.contract.incoming.stream_id, streamIds[args.upstream.id === "original-3" ? 3 : 5].data);
                assert.ok(["original-1", "original-5"].includes(args.downstream.id));
                assert.equal(Buffer.from(args.contract.incoming.body_base64, "base64").length, 65536);
                assert.notEqual(args.contract.incoming.body_base64, args.contract.outgoing.body_base64);
                if (mode === "data-failure") throw original;
                return { messages: Object.values(args.contract).map(message => ({ message_id: message.message_id })) };
            } });
        if (mode === "isolated-outbound") {
            await assert.rejects(run, failure => failure === original);
            assert.equal(signal.signal.aborted, false);
            assert.deepEqual(starts.map(value => value.sessionId).sort(), ["session-0", "session-1", "session-1"]);
            assert.equal(starts.filter(value => value.sessionId === "session-0").length, 1,
                "the failed outbound must never submit its prewritten return");
            const plan = JSON.parse(await readFile(join(directory, "run/plan.json"), "utf8"));
            const failure = JSON.parse(await readFile(join(directory, "run/movement-failure.json"), "utf8"));
            assert.equal(failure.passed, false); assert.equal(failure.failure_isolated, true);
            assert.equal(failure.original_moves_completed, 2);
            assert.equal(failure.failed_slots.length, 1); assert.equal(failure.failed_slots[0].ordinal, 0);
            assert.deepEqual(failure.blocked_moves.map(slot => slot.operation_id),
                [plan.find(step => step.sourceIndex === 0 && step.ordinal === 1).operationId]);
            assert.equal(failure.blocked_moves[0].mutation_submissions, 0);
            assert.equal(failure.original_cutoff_monotonic_ms, 67000);
            assert.equal(clock, 62500); assert.equal(retainedJoined, true);
            assert.equal(reported[0].failure, original);
            assert.equal(new Set(starts.map(value => value.operationId)).size, 3);
            assert.equal(new Set([...stopped, ...joined]).size, admitted.length);
            return;
        }
        if (mode === "partial-data") {
            await assert.rejects(run, { code: "ERR_ASSERTION" });
            assert.equal(starts.length, 0); return;
        }
        if (["missing-root-verification", "unjoined-inspector", "foreign-inspected-clock", "invalid-inspected-hash"].includes(mode)) {
            await assert.rejects(run, error => error.code === "ERR_ASSERTION");
            assert.equal(new Set([...stopped, ...joined]).size, admitted.length);
            return;
        }
        if (!["normal", "data", "early-wake", "prepared-video", "prepared-video-data", "reuse-inspection"].includes(mode)) {
            await assert.rejects(run, error => ["unknown", "retained-failure", "data-failure", "external-failure"].includes(mode) ? error === original :
                ["foreign-final-end", "no-progress-wait"].includes(mode) ? error.code === "ERR_ASSERTION" : mode === "retirement-refusal" ? error.certainty === "read-only" : error.kind === "cancelled");
            assert.equal(retainedJoined, true);
            if (["unknown", "data-failure", "retained-failure"].includes(mode)) {
                assert.equal(reported.length, 1);
                assert.equal(reported[0].failure, original);
                if (mode !== "retained-failure") assert.ok(reported[0].stopped < stopped.length,
                    "the first control failure must be reported before cleanup joins admitted owners");
            }
            assert.equal(new Set(starts.map(v => v.operationId)).size, starts.length);
            if (mode === "retirement-refusal") assert.ok(joined.length >= 1 && joined.length <= 2);
            else assert.equal(joined.length, ["cancel-after4", "foreign-final-end"].includes(mode) ? 2 : 0);
            assert.equal(new Set([...stopped, ...joined]).size, admitted.length);
            assert.ok(subscriptions.length <= admitted.length);
            const failure = JSON.parse(await readFile(join(directory, "run/movement-failure.json"), "utf8"));
            assert.equal(failure.passed, false); assert.equal(failure.target_joins_failed, 0);
            if (["cancelled", "cancel-during-wait", "no-progress-wait"].includes(mode)) {
                assert.equal(starts.length, 0);
                if (mode === "no-progress-wait") assert.equal(failure.failure.observation.stage, "scheduled-move");
                if (mode === "cancel-during-wait") {
                    assert.ok(clockWaits.length > 0);
                    assert.ok(["current-workload", "scheduled-move"].includes(failure.failure.observation.stage));
                }
            }
            else if (mode === "unknown") assert.equal(failure.failure.certainty, "uncertain");
            else if (mode === "data-failure") {
                assert.ok(starts.length >= 1 && starts.length <= 2);
                assert.equal(failure.failure.observation.stage, "original-physical-data-exchange");
            } else if (mode === "external-failure") {
                assert.equal(failure.failure.kind, original.kind);
                assert.equal(failure.failure.certainty, original.certainty);
                assert.equal(failure.failure.observation.stage, "media-workload");
            } else if (mode === "retirement-refusal") {
                assert.ok(starts.length >= 2 && starts.length <= 4);
                assert.equal(failure.failure.certainty, "read-only");
            } else if (mode !== "retained-failure") { assert.equal(starts.length, 4); assert.equal(failure.original_moves_completed, 4); }
            return;
        }
        const { observation, currentConnections } = await run;
        assert.equal(starts.length, 4); assert.equal(subscriptions.length, dataProfile ? 4 : 0);
        assert.equal(new Set(starts.map(v => v.operationId)).size, 4);
        assert.equal(new Set(subscriptions.map(v => v.options.operationId)).size, dataProfile ? 4 : 0);
        assert.equal(stopped.length, 2); assert.equal(joined.length, 2);
        assert.deepEqual(retired.sort(), [0, 4]);
        assert.equal(observation.original_moves_completed, 4);
        assert.equal(observation.movement_profile, MOVEMENT_PROFILE);
        assert.equal(observation.movement_profile_sha256, MOVEMENT_PROFILE_SHA256);
        assert.deepEqual(observation.scheduled_start_offsets_ms, [15000, 40000]);
        assert.ok(clockWaits.length >= (mode === "early-wake" ? 3 : 2));
        for (const move of observation.moves) {
            assert.equal(move.scheduled_start_ms, MOVE_START_OFFSETS_MS[move.ordinal]);
            assert.ok(move.actual_start_ms >= move.scheduled_start_ms && move.actual_start_ms < 62000);
        }
        assert.equal(observation.data_resource_contract_requested, dataProfile);
        assert.equal(observation.replacement_data_io_verified, dataProfile);
        assert.equal(observation.replacement_data_exchanges.length, dataProfile ? 4 : 0);
        if (dataProfile) for (const selected of subscriptions) {
            assert.deepEqual(selected.intent.selectors.rest, []);
            const original = starts.find(start => start.dataStreamId === selected.intent.selectors.first.stream);
            assert.ok(original);
        }
        assert.equal(observation.physical_segments.length, 4);
        if (mode === "reuse-inspection") {
            // Movement consumes the exact already-owned stage receipt, not a
            // second file read. The separate post-idle interpreter still requires
            // every original file and cannot qualify this missing-file fixture.
            for (const segment of observation.physical_segments) {
                assert.match(segment.sha256, /^[0-9a-f]{64}$/);
                await assert.rejects(readFile(segment.observation), {code: "ENOENT"});
            }
        }
        assert.equal(observation.planned_peak_connection_slots, 8);
        assert.equal(observation.shared_sdk_closed_during_replacement, false);
        assert.equal(observation.full_timeline_decoded, false);
        assert.equal(observation.task_complete, false);
        assert.equal(retainedJoined, true);
        assert.equal(observation.current_owner_end.current_connections_ended, 6);
        assert.deepEqual(observation.current_owner_end.physical.map(end => end.index).sort(), [0, 4]);
        assert.ok(currentConnections.every(v => [0, 4].includes(v.sourceIndex) && v.connection.id.endsWith("-1")));
    } finally { await rm(directory, { recursive: true, force: true }); }
}

test("four distinct scheduled moves retain one source clock across two switch-and-return Session chains", () => scenario("normal"));
test("an early timer wake waits for the remaining original-clock interval before any move", () => scenario("early-wake"));
test("cancellation during the scheduled wait starts no original mutation", () => scenario("cancel-during-wait"));
test("a scheduled wait that does not advance the clock fails instead of starting a move or spinning", () => scenario("no-progress-wait"));
test("the Data workload carries all original incoming and new target Data identities through four moves", () => scenario("data"));
test("a partly declared six-Connection Data workload refuses before any movement", () => scenario("partial-data"));
test("a failed original Data exchange stops both movement chains and joins admitted owners without new roots", () => scenario("data-failure"));
test("an unknown original prevents further moves and joins every already admitted physical owner", () => scenario("unknown"));
test("parent cancellation before the workload attempts no original move", () => scenario("cancelled"));
test("parent cancellation stops both final natural source waits and cannot promote four completed roots to a workload pass", () => scenario("cancel-after4"));
test("a failed retained end cancels movement and joins every already admitted physical owner", () => scenario("retained-failure"));
test("an original external media error survives an earlier settling retained-end cancellation", () => scenario("external-failure"));
test("a final end receipt for a foreign Connection cannot pass four successful roots", () => scenario("foreign-final-end"));
test("a completed final end cannot replace current-authorized retirement metadata for that same owner", () => scenario("retirement-refusal"));
test("two Session chains cannot silently share one serialized ordinary control helper", async () => {
    const shared = {};
    await assert.rejects(deviceMovementWorkload({ workload: { routes: Array(6) }, connections: Array(6),
        sessions: Array(2), controllers: [shared, shared] }), error => error.code === "ERR_ASSERTION");
});

 test("prepared video mappings follow both original owners without a post-move video command", () => scenario("prepared-video"));
 test("prepared video keeps its receiver while separately authorized Data subscriptions complete", () => scenario("prepared-video-data"));

 test("a definitively refused original outbound blocks only its own return while the original sibling reaches40s and62s cleanup", () => scenario("isolated-outbound"));


test("movement reuses the joined stage inspection without rereading its large capture before original retirement", () => scenario("reuse-inspection"));
test("movement refuses missing root-boundary proof, unjoined inspection and foreign clock or capture hash", async () => {
    for (const mode of ["missing-root-verification", "unjoined-inspector", "foreign-inspected-clock", "invalid-inspected-hash"])
        await scenario(mode);
});
