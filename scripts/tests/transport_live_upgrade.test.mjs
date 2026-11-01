import assert from "node:assert/strict";
import test from "node:test";
import { liveSessionUpgrade } from "../lib/transport_live_upgrade.mjs";

function fixture({ corruptAfter = false, changeGeneration = false, refusal, receiveRefusal } = {}) {
    const roles = ["participant-1", "participant-2"], participants = Object.fromEntries(roles.map(v => [v, "member-" + v]));
    const plan = { upgrade: "original-upgrade", unsupported: "original-unsupported" }, queues = new Map(), connections = [], calls = [], observations = [];
    for (const sid of [0, 1]) for (const role of roles) for (const field of ["connection", "stream", "offer", "select", "before", "after", "before-message", "after-message", "end"])
        plan["live-" + sid + "-" + role + "-" + field] = "original-" + sid + "-" + role + "-" + field;
    const operation = (value, action = () => {}) => ({ wait: async () => {
        action(); return { value, snapshot: { state: "completed" } };
    } });
    const states = [0, 1].map(sid => ({ id: "session-" + sid, conversationId: "conversation-original",
        media: sid ? ["data"] : ["data", "audio"], topologyRevision: 1, state: "active" }));
    const sessions = states.map((state, sid) => ({ id: state.id, conversationId: state.conversationId,
        refresh: async () => structuredClone(state), update: async (input, expected, options) => {
            calls.push({ type: "upgrade", sid, input, options }); assert.equal(expected, state.topologyRevision);
            if (input.media.first === "screen") throw Object.assign(Error(), { problem: { code: 501, reason: "unsupported_feature" } });
            state.media = ["data", "audio", "video"]; state.topologyRevision++;
            if (changeGeneration && sid === 1) connections[0].generation++;
            return operation({});
        } }));
    const clients = new Map(roles.map(role => [role, { conversations: { byId: cid => {
        assert.equal(cid, "conversation-original"); return { sessions: { byId: sid => ({ connections: { offer: async (input, options) => {
            calls.push({ type: "offer", options }); assert.deepEqual(input.streams.first.kind, "data");
            const connection = { id: input.connectionId, generation: 1, state: "active", readsAfterEnd: 0 };
            const ownStream = input.streams.first.id;
            connection.refresh = async () => {
                if (connection.state === "ending" && ++connection.readsAfterEnd > 1) connection.state = "retired";
                return { id: connection.id, generation: connection.generation, state: connection.state, streamRevision: 1 };
            };
            connection.streams = { subscribe: async (_, o) => { calls.push({ type: "subscribe", options: o }); return operation({}); },
                byId: stream => ({ media: { sendData: async (input, o) => {
                    assert.equal(stream, ownStream); calls.push({ type: "send", options: o });
                    if (refusal) throw refusal;
                    const target = input.to.first, key = target + ":" + stream, bytes = Buffer.from(input.payload.value);
                    if (corruptAfter && o.operationId.endsWith("-after")) bytes[0] ^= 1;
                    queues.set(key, { messageId: input.messageId, operationId: o.operationId, fromParticipant: participants[role],
                        payload: { kind: "binary", value: bytes } });
                    return operation({ boundary: "core-queue-admission", connectionId: connection.id });
                }, receiveData: async () => {
                    if (receiveRefusal) throw receiveRefusal;
                    const key = participants[role] + ":" + stream, value = queues.get(key); assert.ok(value);
                    queues.delete(key); return value;
                } } }) };
            connection.end = async (_, o) => { calls.push({ type: "end", options: o });
                return operation({}, () => { connection.state = "ending"; connection.generation++; }); };
            connections.push(connection); return operation(connection);
        } } }) } };
    } } }]));
    const complete = async work => (await (await work).wait({ timeoutMs: 8000 })).value;
    const refused = async (call, code, reason) => assert.rejects(call, error => error.problem.code === code && error.problem.reason === reason);
    return { calls, connections, queues, observations, run: () => liveSessionUpgrade({ clients, sessions, plan, participants,
        options: { timeoutMs: 8000 }, complete, refused, observe: v => observations.push(v) }) };
}

test("original data Connections and unique payloads span both upgrades; retirement is observed after each end", async () => {
    const f = fixture(), result = await f.run();
    assert.equal(result.messages.length, 8); assert.equal(new Set(result.messages.map(v => v.messageId)).size, 8);
    assert.equal(f.calls.filter(v => v.type === "send").length, 8);
    assert.ok(f.connections.every(v => v.state === "retired" && v.readsAfterEnd === 2));
    assert.equal(f.queues.size, 0); assert.equal(result.observation.current_unoffered_screen_refusals, 2);
    const firstUpgrade = f.calls.findIndex(v => v.type === "upgrade");
    assert.equal(f.calls.slice(0, firstUpgrade).filter(v => v.type === "send").length, 4);
    assert.equal(f.calls.slice(firstUpgrade).filter(v => v.type === "send").length, 4);
    assert.ok(f.calls.every(v => v.options.operationId.startsWith("original-")));
    assert.equal(f.observations.length, 1);
    assert.equal(f.observations[0].passed, true);
    assert.equal(f.observations[0].failure, null);
    assert.equal(f.observations[0].data_received, 8);
    assert.equal(f.observations[0].retirements_observed, 4);
});
test("a changed original generation cannot count as live continuity", async () => {
    const f = fixture({ changeGeneration: true }); await assert.rejects(f.run());
    assert.equal(f.calls.filter(v => v.type === "send").length, 4);
});
test("post-upgrade received data must match the original addressed binary payload", async () => {
    const f = fixture({ corruptAfter: true }); await assert.rejects(f.run());
    assert.equal(f.calls.filter(v => v.type === "send").length, 5);
});
test("current authority refusal propagates without another mutation or replacement operation ID", async () => {
    const refusal = Error("current-authority-refused"), f = fixture({ refusal });
    await assert.rejects(f.run(), error => error === refusal);
    assert.equal(f.calls.filter(v => v.type === "send").length, 1);
    assert.equal(f.calls.filter(v => v.type === "upgrade").length, 0);
    assert.deepEqual(f.observations[0].failure,
        { stage: "data-send", session_index: 0, role_index: 0, round: "before" });
    assert.equal(f.observations[0].data_completed, 0);
});

test("an original receive deadline is located after one completed send without replay or invented receipt", async () => {
    const receiveRefusal = Object.assign(Error("original-read-deadline"), { kind: "deadline", certainty: "read-only" });
    const f = fixture({ receiveRefusal });
    await assert.rejects(f.run(), error => error === receiveRefusal);
    assert.equal(f.calls.filter(v => v.type === "send").length, 1);
    assert.equal(f.calls.filter(v => v.type === "upgrade").length, 0);
    assert.equal(f.observations.length, 1);
    assert.equal(f.observations[0].passed, false);
    assert.deepEqual(f.observations[0].failure,
        { stage: "data-receive", session_index: 0, role_index: 0, round: "before" });
    assert.equal(f.observations[0].data_completed, 1);
    assert.equal(f.observations[0].data_received, 0);
    assert.equal(f.observations[0].retirements_observed, 0);
});
