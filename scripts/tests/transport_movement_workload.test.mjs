import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
import { deviceMovementWorkload } from "../lib/transport_movement_workload.mjs";

async function scenario(mode) {
    const root = resolve(".local"); await mkdir(root, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(root, "movement-controller-"));
    const signal = new AbortController(), starts = [], admitted = [], subscriptions = [], stopped = [], joined = [], retired = [], finalWaits = new Map();
    const roles = ["owner", "two", "three", "four", "owner", "two"], peers = [3, 0, 1, 2, 5, 4];
    const revisions = [1, 1];
    const controllers = [{ owner: "existing-original-helper" }, { owner: "existing-controller-helper" }];
    const connection = (id, generation) => ({ id, async refresh() { return { id, generation, streamRevision: 2 }; },
        streams: { async subscribe(intent, options) {
            subscriptions.push({ intent, options });
            return { async wait() { return { snapshot: { state: "completed" } }; } };
        } } });
    const connections = roles.map((role, i) => ({ connection: connection("original-" + i, 1) }));
    const routes = roles.map((role, i) => ({ index: i, peerIndex: peers[i], sessionIndex: Number(i >= 4), role,
        connection: connections[i].connection,
        expectRetirement(operationId) { return proof => {
            assert.equal(proof.reference.operation_id, operationId); retired.push(i);
        }; },
        async stopAndJoin() { assert.ok(retired.includes(i)); },
        async switchIncomingVideo(operationId, videoId, select) {
            assert.ok(videoId.startsWith("video_"));
            const operation = await select(); await operation.wait();
            assert.equal(subscriptions.at(-1).options.operationId, operationId);
        },
    }));
    const sessions = [0, 1].map(i => ({ id: "session-" + i,
        async refresh() { return { state: "accepted", topologyRevision: revisions[i] }; } }));
    const original = Object.assign(Error("original unknown move"), { kind: "deadline", certainty: "uncertain" });
    const workload = { routes, signal: signal.signal, programStartUnixUs: 1793600000000000 };
    if (mode === "cancelled") signal.abort();
    const startTarget = async options => {
        const sid = Number(options.sessionId.at(-1)), ordinal = starts.filter(v => v.sessionId === options.sessionId).length;
        assert.equal(options.controller, controllers[sid]);
        starts.push(options);
        const saved = JSON.parse(await readFile(join(directory, "run/plan.json"), "utf8"));
        assert.equal(saved.length, 20);
        assert.equal(saved.filter(v => v.operationId === options.operationId).length, 1);
        assert.equal(options.expectedTopologyRevision, revisions[sid]);
        assert.equal(options.source.programStartUnixUs, workload.programStartUnixUs);
        assert.equal(options.target.deviceId, ordinal % 2 ? "original-device" : "replacement-device");
        if (mode === "unknown" && sid === 0 && ordinal === 2) throw original;
        revisions[sid]++;
        const moved = { retired: [options.connectionId] };
        const reference = { command: "connection.replace", operation_id: options.operationId,
            scope: { connection_id: options.connectionId } };
        await options.retireOriginal({ reference, original: { id: options.connectionId,
            state: "retired", generation: options.originalGeneration + 1 }, movement: moved });
        await mkdir(options.directory);
        await writeFile(join(options.directory, "target-observation.json"), JSON.stringify({
            program_start_unix_us: workload.programStartUnixUs,
            received: [{ original_root_completed_observed: false }, { original_root_completed_observed: true }],
        }));
        const stage = { passed: true };
        let result;
        admitted.push(options.operationId);
        return { reference, movement: moved, connection: connection("candidate-" + sid + "-" + ordinal, options.originalGeneration + 1),
            async stopAndJoin() {
                if (!result) { stopped.push(options.operationId); result = stage; }
                finalWaits.get(options.operationId)?.(result);
                return result;
            },
            async join() {
                if (mode === "cancel-after20") return new Promise(done => {
                    joined.push(options.operationId); finalWaits.set(options.operationId, done);
                    if (finalWaits.size === 2) setImmediate(() => signal.abort());
                });
                if (!result) { joined.push(options.operationId); result = stage; } return result;
            } };
    };
    try {
        const run = deviceMovementWorkload({ workload, controllers, conversationId: "original-room", sessions,
            connections, streamIds: roles.map((_, i) => ({ audio: "audio-" + i, video: "video-" + i })),
            participantIds: roles.map(v => "participant-" + v),
            targets: [{ deviceId: "original-device" }, { deviceId: "replacement-device" }],
            sourcePrograms: Object.fromEntries([0, 4].map(i => [i, { audioPackets: Array(3100).fill([1]), videoFrames: Array(1860).fill([1]) }])),
            privateRoot: root, directory: join(directory, "run"), executable: "unexecuted-test-double",
            executableSha256: "0".repeat(64), startTarget });
        if (mode !== "normal") {
            await assert.rejects(run, error => mode === "unknown" ? error === original : error.kind === "cancelled");
            assert.equal(new Set(starts.map(v => v.operationId)).size, starts.length);
            assert.equal(joined.length, mode === "cancel-after20" ? 2 : 0);
            assert.equal(stopped.length, admitted.length);
            assert.ok(subscriptions.length <= admitted.length);
            const failure = JSON.parse(await readFile(join(directory, "run/movement-failure.json"), "utf8"));
            assert.equal(failure.passed, false); assert.equal(failure.target_joins_failed, 0);
            if (mode === "cancelled") assert.equal(starts.length, 0);
            else if (mode === "unknown") assert.equal(failure.failure.certainty, "uncertain");
            else { assert.equal(starts.length, 20); assert.equal(failure.original_moves_completed, 20); assert.equal(stopped.length, 20); }
            return;
        }
        const { observation, currentConnections } = await run;
        assert.equal(starts.length, 20); assert.equal(subscriptions.length, 20);
        assert.equal(new Set(starts.map(v => v.operationId)).size, 20);
        assert.equal(new Set(subscriptions.map(v => v.options.operationId)).size, 20);
        assert.equal(stopped.length, 18); assert.equal(joined.length, 2);
        assert.deepEqual(retired.sort(), [0, 4]);
        assert.equal(observation.original_moves_completed, 20);
        assert.equal(observation.physical_segments.length, 20);
        assert.equal(observation.planned_peak_connection_slots, 8);
        assert.equal(observation.shared_sdk_closed_during_replacement, false);
        assert.equal(observation.full_timeline_decoded, false);
        assert.equal(observation.task_complete, false);
        assert.ok(currentConnections.every(v => [0, 4].includes(v.sourceIndex) && v.connection.id.endsWith("-9")));
    } finally { await rm(directory, { recursive: true, force: true }); }
}

test("twenty distinct original moves retain one source clock across two ten-move Session chains", () => scenario("normal"));
test("an unknown original prevents further moves and joins every already admitted physical owner", () => scenario("unknown"));
test("parent cancellation before the workload attempts no original move", () => scenario("cancelled"));
test("parent cancellation stops both final natural source waits and cannot promote twenty completed roots to a workload pass", () => scenario("cancel-after20"));
test("two Session chains cannot silently share one serialized ordinary control helper", async () => {
    const shared = {};
    await assert.rejects(deviceMovementWorkload({ workload: { routes: Array(6) }, connections: Array(6),
        sessions: Array(2), controllers: [shared, shared] }), error => error.code === "ERR_ASSERTION");
});
