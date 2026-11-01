import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { replacementInput, physicalReplacementTarget } from "../lib/transport_replacement_target.mjs";

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
