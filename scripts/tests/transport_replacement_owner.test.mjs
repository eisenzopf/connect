import { isolatedPrivateTargetFailure, privateTargetCleanup } from "../lib/transport_failure_isolation.mjs";
import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { startPhysicalReplacementTarget } from "../lib/transport_replacement_target.mjs";

// A real separately owned process exercises controller lifecycle only. It is
// explicitly synthetic and performs no native negotiation or media IO.
async function fixture(body, failBeforeCompletion = false, preparationStates = null, malformedCompleted = null) {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const root = await mkdtemp(join(parent, "replacement-owner-test-"));
    const executable = join(root, "synthetic-target"), config = join(root, "client.toml");
    await writeFile(config, "synthetic-config", { mode: 0o600 });
    await writeFile(executable, `#!${process.execPath}\nif (${failBeforeCompletion}) process.exit(23);\n` + String.raw`
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (input.media.owner_control !== true) process.exit(9);
const stem = input.output.replace(/\.json$/, "");
const ready = { format: "connect/native-replacement-owner-ready/1", operation_id: input.command.operation_id,
 connection_id: "destination", video_enabled: false, decoded_media_verified: false,
 operation_completed_inferred: false, negotiated_streams: [{ strm_id: input.offer.streams_offered[0].id }] };
fs.writeFileSync(stem + ".owner-ready.tmp", JSON.stringify(ready), { mode: 0o600, flag: "wx" });
fs.linkSync(stem + ".owner-ready.tmp", stem + ".owner-ready.json");
fs.writeFileSync(stem + ".pid", String(process.pid), { mode: 0o600, flag: "wx" });
const deadline = setTimeout(() => process.exit(8), 5000);
const poll = setInterval(() => {
 if (!fs.existsSync(stem + ".owner-stop.json")) return;
 const stop = JSON.parse(fs.readFileSync(stem + ".owner-stop.json", "utf8"));
 if (stop.command !== "stop" || stop.operation_id !== input.command.operation_id) process.exit(10);
 clearTimeout(deadline); clearInterval(poll);
 fs.writeFileSync(input.output, JSON.stringify({ format: "connect/native-replacement-peer-observation/1",
  passed: true, original_client_joined: true, operation_completed_inferred: false, decoded_media_verified: false,
  sent_audio_packets: 1, skipped_original_frames: 0, video_enabled: false, received: [],
  owner_control: true, owner_ready: true, stop_requested: true }), { mode: 0o600 });
}, 10);
`, { mode: 0o700 });
    let submitted = 0, retired = 0, restored = 0, waitAborted = false;
    const reference = { command: "connection.replace", operation_id: "original",
        scope: { conversation_id: "conversation", session_id: "session", connection_id: "source" } };
    const moved = { connection: { id: "destination" }, admitted: [{ id: "destination" }], retired: ["source"],
        retained: [{ id: "retained", refresh: async () => ({ id: "retained", generation: 1 }) }],
        topologyRevision: 2, conversationRevision: 3, binding: { deviceId: "device" } };
    const operation = { referenceJson: () => reference, wait: async options => {
        if (!failBeforeCompletion) return { snapshot: { state: "completed" }, value: malformedCompleted ?
            { get connection() { throw malformedCompleted; } } : moved };
        return new Promise((_, reject) => {
            const timer = setTimeout(() => reject(Error("synthetic-read-deadline")), 1000);
            options.signal?.addEventListener("abort", () => {
                clearTimeout(timer); waitAborted = true; reject(Error("synthetic-read-cancelled"));
            }, { once: true });
        });
    } };
    let preparationReads = 0, preparationBudget = 8001;
    const controller = { conversations: { byId: () => ({ sessions: { byId: () => ({ connections: { byId: () => ({
        replaceDevice: async () => { submitted++; return operation; },
        refresh: async () => ({ id: "source", state: "retired", generation: 2 }),
    }) } }) } }) }, operationKinds: { connection_replace: "kind" }, operations: { restore: saved => {
        assert.deepEqual(saved, reference); restored++; return operation;
    } }, raw: { validate: v => v, request: async (frame, options) => {
        assert.equal(frame.payload.operation_id, "original");
        assert.ok(options.timeoutMs < preparationBudget); preparationBudget = options.timeoutMs;
        if (preparationStates) {
            // A real target executable must not even be launched during an
            // unconfirmed reservation or malformed confirmation observation.
            await assert.rejects(stat(join(root, "stage/target-input.json")), { code: "ENOENT" });
            assert.ok(preparationReads < preparationStates.length);
        }
        const preparation = preparationStates?.[preparationReads] ?? { native_replacement_prepared: true };
        preparationReads++;
        return [{ value: { type: "operation.updated", payload: {
            operation_id: "original", record_id: "record", revision: preparationReads,
            command_type: "connection.replace", expected_revision: 1, state: "preparing", ...preparation,
        } } }];
    } } };
    const directory = join(root, "stage");
    let owner;
    try {
        owner = await startPhysicalReplacementTarget({ controller, retireOriginal: async () => { retired++; },
            privateRoot: parent, directory, executable,
            executableSha256: createHash("sha256").update(await readFile(executable)).digest("hex"),
            operationId: "original", conversationId: "conversation", sessionId: "session", connectionId: "source",
            retainedConnectionId: "retained", expectedTopologyRevision: 1, originalGeneration: 1, retainedGeneration: 1,
            streamId: "target-audio", target: { endpointId: "endpoint", deviceId: "device", nativeConfig: config },
            source: { seconds: 1, ssrc: 7, audioPackets: Array.from({ length: 50 }, () => [1]) } });
        assert.equal(failBeforeCompletion, false, "failed target unexpectedly admitted");
        await body({ owner, directory, preparationReads, counters: () => ({ submitted, retired, restored }) });
    } catch (error) {
        if (!failBeforeCompletion && !malformedCompleted) throw error;
        await body({ error, directory, waitAborted, counters: () => ({ submitted, retired, restored }) });
    } finally {
        await owner?.stopAndJoin().catch(() => {});
        await rm(root, { recursive: true, force: true });
    }
}

test("early physical failure cancels only the original read wait and preserves the target cause", async () => {
    await fixture(async ({ error, directory, waitAborted, counters }) => {
        assert.equal(error.message, "original-physical-target-exited-before-completion");
        assert.equal(waitAborted, true);
        const stage = JSON.parse(await readFile(join(directory, "stage.json"), "utf8"));
        assert.equal(stage.failure.phase, "independent-physical-target");
        assert.deepEqual(stage.target_early_exit, { code: 23, signal: null, error: null });
        assert.equal(stage.target_worker_joined, true);
        assert.equal(stage.target_worker_forced, false);
        assert.equal(stage.original_root_completed, false);
        assert.equal(stage.passed, false);
        assert.deepEqual(counters(), { submitted: 1, retired: 0, restored: 0 });
    }, true);
});

test("admission retains a live owner; concurrent join and repeated stop share one physical cleanup", async () => {
    await fixture(async ({ owner, directory, counters }) => {
        const pid = Number(await readFile(join(directory, "target-observation.pid"), "utf8"));
        process.kill(pid, 0);
        assert.equal(owner.connection.id, "destination");
        assert.equal(owner.reference.operation_id, "original");
        const admission = JSON.parse(await readFile(join(directory, "admission.json"), "utf8"));
        assert.equal(admission.passed, false); assert.equal(admission.target_worker_joined, false);
        await assert.rejects(stat(join(directory, "stage.json")), { code: "ENOENT" });
        const joined = owner.join();
        const [a, b, c] = await Promise.all([joined, owner.stopAndJoin(), owner.stopAndJoin()]);
        assert.deepEqual(a, b); assert.deepEqual(b, c);
        assert.equal(a.passed, true); assert.equal(a.target_worker_joined, true);
        assert.equal(a.target_worker_forced, false); assert.equal(a.target_stop_requested, true);
        assert.deepEqual(counters(), { submitted: 1, retired: 1, restored: 1 });
        assert.deepEqual(JSON.parse(await readFile(join(directory, "target-observation.owner-stop.json"), "utf8")),
            { command: "stop", operation_id: "original" });
        assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
    });
});

test("a failed physical owner cannot turn completed admission into a passing final report", async () => {
    await fixture(async ({ owner, directory, counters }) => {
        const pid = Number(await readFile(join(directory, "target-observation.pid"), "utf8"));
        process.kill(pid, "SIGTERM");
        await assert.rejects(owner.join());
        const report = JSON.parse(await readFile(join(directory, "stage.json"), "utf8"));
        assert.equal(report.passed, false); assert.equal(report.target_worker_joined, true);
        assert.equal(report.failure.phase, "target-worker-join");
        assert.deepEqual(counters(), { submitted: 1, retired: 1, restored: 1 });
    });
});

// New focused counterexample: reservation is not the original captured ticket.
test("preparation confirmation precedes target spawn without retaining allocation authority", async () => {
    const observations = [{}, { native_replacement_prepared: false },
        { native_replacement_prepared: 1 }, { native_replacement_prepared: "true" },
        { effects: [{ provider: "native-replacement-prepare", state: "confirmed", attempt: 1 }] },
        { native_replacement_prepared: true }];
    await fixture(async ({ owner, directory, preparationReads, counters }) => {
        assert.equal(preparationReads, observations.length);
        assert.deepEqual(counters(), { submitted: 1, retired: 1, restored: 1 });
        const input = JSON.parse(await readFile(join(directory, "target-input.json"), "utf8"));
        assert.equal(input.root.revision, observations.length);
        assert.equal(input.root.native_replacement_prepared, true);
        const result = await owner.stopAndJoin();
        assert.equal(result.passed, true);
        assert.equal(result.target_worker_joined, true);
        assert.equal(result.target_worker_forced, false);
    }, false, observations);
});

test("a verified completed snapshot remains completion-observed before malformed result access throws", async () => {
    const original=Error("original malformed completed result");
    await fixture(async ({error,directory,counters}) => {
        assert.equal(error,original);
        const stage=JSON.parse(await readFile(join(directory,"stage.json"),"utf8"));
        assert.equal(stage.original_root_completed,true);assert.equal(stage.passed,false);
        const proof=privateTargetCleanup(error).facts;
        assert.equal(proof.completion_observed,true);assert.equal(proof.logical_outcome,"completed-observed");
        assert.equal(proof.target_worker_joined,true);assert.equal(isolatedPrivateTargetFailure(error),false);
        assert.deepEqual(counters(),{submitted:1,retired:0,restored:0});
    },false,null,original);
});
