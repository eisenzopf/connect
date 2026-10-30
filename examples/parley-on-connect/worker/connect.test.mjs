import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { NodeClient } from "../../../sdk/typescript/dist/index.js";
import { ConnectWorkerGateway } from "./connect.mjs";
const executable = fileURLToPath(new URL("../../../sdk/tests/fixtures/sdk_helper.mjs", import.meta.url));
const base = new URL("../../../.local/gate-6/decision-worker-sdk-tests/", import.meta.url);
const calls = async dir => (await readFile(join(dir, "requests.jsonl"), "utf8").catch(() => ""))
    .trim().split("\n").filter(Boolean).map(JSON.parse);
async function fixture(config, work) {
    await mkdir(base, { recursive: true, mode: 0o700 });
    const dir = await mkdtemp(new URL("sdk-", base).pathname), path = join(dir, "config.json");
    await writeFile(path, JSON.stringify(config), { mode: 0o600 });
    const client = new NodeClient({ config: path, executable });
    const gateway = new ConnectWorkerGateway({ client, conversationId: "room", timeoutMs: 1500 });
    try { await work(gateway, dir, client); }
    finally {
        await client.close({ timeoutMs: 1500 });
        const pid = Number(await readFile(join(dir, "pid"), "utf8").catch(() => 0));
        if (pid) assert.throws(() => process.kill(pid, 0), error => error.code === "ESRCH");
        await rm(dir, { recursive: true, force: true });
    }
}
const originalConsultation = () => ({ schema: "connect-operation-reference/1", operation_id: "consult-original",
    command: "session.consult", scope: { conversation_id: "room", session_id: "session", connection_id: "via" },
    continuation: { kind: "consultation", retained_connection_id: "retained", record_id: "record-original" } });
const cases = () => [
    { command: "message.send", scope: { conversationId: "room" }, input: { messageId: "message", to: { first: "owner", rest: [] }, text: "bounded reply" } },
    { command: "session.invite", inviteKind: "new", scope: { conversationId: "room", sessionId: "session" }, input: { sessionId: "session", to: ["owner"], medium: "voice", intent: "bounded", capabilities: {} } },
    { command: "session.invite", inviteKind: "participants", scope: { conversationId: "room", sessionId: "session" }, input: { to: { first: "owner", rest: [] }, medium: "voice", intent: "bounded", capabilities: {}, expectedTopologyRevision: 2 } },
    { command: "session.end", scope: { conversationId: "room", sessionId: "session" }, input: { reasonCode: 200, reason: "owned closing complete", expectedTopologyRevision: 2 } },
    { command: "session.consult", scope: { conversationId: "room", sessionId: "session", connectionId: "via" }, input: { via: "via", retain: "retained", with: { first: "owner", rest: [] }, hold: "silence", expectedTopologyRevision: 2 } },
    { command: "session.consult.return", scope: { conversationId: "room", sessionId: "session", connectionId: "via" }, input: { consultation: originalConsultation(), expectedTopologyRevision: 2 } },
    { command: "session.handoff", scope: { conversationId: "room", sessionId: "session", connectionId: "via" }, input: { via: "via", retain: "retained", from: "assistant", to: "owner", outgoing: "leave", role: { scope: "session" }, consultation: originalConsultation(), expectedTopologyRevision: 2 } },
    { command: "connection.replace", scope: { conversationId: "room", sessionId: "session", connectionId: "via" }, input: { retain: "retained", target: { endpoint: "owner-device" }, expectedTopologyRevision: 2 } },
];
test("public SDK validates all original worker references without starting a helper", async () => {
    await fixture({}, async (gateway, dir) => {
        for (const [index, input] of cases().entries()) {
            const intent = { ...input, operationId: "original-" + index }, saved = gateway.prepare(intent);
            assert.equal(saved.operation_id, intent.operationId); assert.equal(saved.command, intent.command);
            assert.equal(gateway.sameReference(saved, structuredClone(saved)), true);
            assert.equal(gateway.sameReference(saved, { ...saved, operation_id: "different" }), false);
            assert.equal(gateway.sameReference(saved, { ...saved, scope: { conversation_id: "foreign" } }), false);
        }
        assert.equal((await calls(dir)).length, 0);
    });
});
test("closed command adapter submits through public typed methods and observes the exact original", async () => {
    for (const [index, input] of cases().entries()) await fixture({}, async (gateway, dir) => {
        const intent = { ...input, operationId: "original-" + index }, before = gateway.prepare(intent);
        const returned = await gateway.submit(intent); assert.equal(gateway.sameReference(before, returned.reference), true);
        const observed = await gateway.observe(returned.reference, intent);
        assert.equal(observed.state, "completed"); assert.equal(observed.key.operationId, intent.operationId);
        const requests = await calls(dir); assert.equal(requests.length, 2);
        assert.equal(requests[1].request.action.action.type, "status");
        assert.equal(requests[1].request.action.reference.operation_id, intent.operationId);
        assert.equal(requests[1].request.action.reference.command, intent.command);
    });
});
test("wrong resource and continuation references refuse before helper IO", async () => {
    await fixture({}, async (gateway, dir) => {
        const all = cases();
        for (const input of all) {
            assert.throws(() => gateway.prepare({ ...input, operationId: "original", scope: { ...input.scope, conversationId: "foreign" } }), { code: "different-action-scope" });
        }
        assert.throws(() => gateway.prepare({ ...all[1], operationId: "original", input: { ...all[1].input, sessionId: "foreign" } }), { code: "different-invitation-scope" });
        assert.throws(() => gateway.prepare({ ...all[5], operationId: "original", input: { ...all[5].input, consultation: { ...originalConsultation(), scope: { conversation_id: "foreign" } } } }), { code: "missing-original-consultation" });
        assert.throws(() => gateway.prepare({ ...all[6], operationId: "original", input: { ...all[6].input, via: "foreign" } }), { code: "different-controller-scope" });
        const intent = { ...all[0], operationId: "original" };
        await assert.rejects(gateway.observe({ ...gateway.prepare(intent), operation_id: "foreign" }, intent), { code: "different-original-operation" });
        assert.equal((await calls(dir)).length, 0);
    });
});
test("event replay uses a public authorized cursor reader and joins it on exact match", async () => {
    await fixture({}, async (gateway, dir) => {
        const first = gateway.events(null), previousCursor = await first.ready(), next = await first.next();
        await first.close();
        const fact = await gateway.readEvent({ conversationId: "room", previousCursor,
            eventId: next.value.eventId, cursor: next.value.cursor });
        assert.equal(fact.eventId, next.value.eventId);
        const requests = await calls(dir);
        assert.equal(requests.filter(x => x.request.action.type === "events_close").length, 2);
        assert.ok(requests.every(x => ["events_poll", "events_close"].includes(x.request.action.type)));
    });
});
test("fresh context comes from public snapshots while owned speech remains an app fact", async () => {
    await fixture({}, async (gateway, dir) => {
        const context = await gateway.context({ stage: "closing", planId: "plan", planVersion: 1 },
            { sessionId: "session", speech: { sessionId: "session", sourceId: "owned", complete: false }, approverIds: ["owner"] });
        assert.equal(context.sessionId, "session"); assert.equal(context.speech.complete, false);
        assert.deepEqual(context.approvers, []); // No fabricated owner from an incoming body.
        assert.deepEqual((await calls(dir)).map(x => x.request.action.type), ["conversation_refresh", "session"]);
    });
});
