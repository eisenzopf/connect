// Live public SDK data Connections across an additive Session capability change.
// Original intent IDs are allocated and persisted by the parent before any IO.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { SdkBinaryData } from "../../sdk/typescript/dist/index.js";

export async function liveSessionUpgrade({ clients, sessions, plan, participants, options,
    complete, refused, observe = () => {} }) {
    assert.equal(sessions.length, 2);
    const roles = ["participant-1", "participant-2"], connections = [], messages = [];
    const mutation = key => ({ ...options, operationId: plan[key] });
    const began = performance.now();
    let context = { stage: "offer", session_index: null, role_index: null, round: null }, passed = false;
    const counters = { offers_completed: 0, selections_completed: 0, data_completed: 0,
        data_received: 0, upgrades_completed: 0, ends_completed: 0, retirements_observed: 0 };
    const mark = (stage, sid = null, role = null, round = null) => {
        context = { stage, session_index: sid, role_index: role === null ? null : roles.indexOf(role), round };
    };
    try {
        for (const [sid, session] of sessions.entries()) for (const role of roles) {
            const prefix = "live-" + sid + "-" + role;
            const source = clients.get(role).conversations.byId(session.conversationId).sessions.byId(session.id);
            mark("offer", sid, role);
            const connection = await complete(source.connections.offer({ connectionId: plan[prefix + "-connection"],
                streams: { first: { id: plan[prefix + "-stream"], kind: "data", direction: "sendrecv",
                    codecs: { first: "binary", rest: [] } }, rest: [] } }, mutation(prefix + "-offer")));
            counters.offers_completed++;
            connections.push({ sid, role, prefix, connection });
        }
        for (const target of connections) {
            const peer = connections.find(v => v.sid === target.sid && v.role !== target.role);
            mark("selection-snapshot", target.sid, target.role);
            const current = await target.connection.refresh(options);
            mark("selection", target.sid, target.role);
            await complete(target.connection.streams.subscribe({ selectors: {
                first: { stream: plan[peer.prefix + "-stream"] }, rest: [] },
            expectedStreamRevision: current.streamRevision }, mutation(target.prefix + "-select")));
            counters.selections_completed++;
        }
        mark("original-snapshots");
        const originals = await Promise.all(connections.map(v => v.connection.refresh(options)));
        assert.ok(originals.every(v => ["ready", "active"].includes(v.state)));
        const payloadDigests = [];
        const exchange = async round => {
            for (const [index, source] of connections.entries()) {
                const target = connections.find(v => v.sid === source.sid && v.role !== source.role);
                const bytes = Buffer.from(Array.from({ length: 4096 }, (_, at) => (at * 17 + index * 31 + (round === "after" ? 71 : 0)) % 256));
                const messageId = plan[source.prefix + "-" + round + "-message"];
                const operationId = plan[source.prefix + "-" + round];
                mark("data-send", source.sid, source.role, round);
                const sent = await complete(source.connection.streams.byId(plan[source.prefix + "-stream"]).media.sendData({
                    messageId, to: { first: participants[target.role], rest: [] }, payload: new SdkBinaryData(bytes),
                }, { ...options, operationId }));
                counters.data_completed++;
                assert.equal(sent.boundary, "core-queue-admission");
                assert.equal(sent.connectionId, source.connection.id);
                mark("data-receive", source.sid, source.role, round);
                const got = await target.connection.streams.byId(plan[source.prefix + "-stream"]).media.receiveData(options);
                counters.data_received++;
                mark("data-identity", source.sid, source.role, round);
                assert.equal(got.messageId, messageId); assert.equal(got.operationId, operationId);
                assert.equal(got.fromParticipant, participants[source.role]);
                assert.equal(got.payload.kind, "binary"); assert.deepEqual(Buffer.from(got.payload.value), bytes);
                messages.push({ source: source.role, target: target.role, messageId });
                payloadDigests.push(createHash("sha256").update(bytes).digest("hex"));
            }
        };
        await exchange("before");
        const upgraded = [];
        for (const [sid, session] of sessions.entries()) {
            mark("upgrade-snapshot", sid);
            const current = await session.refresh(options);
            assert.ok(!current.media.includes("video"));
            mark("upgrade", sid);
            await complete(session.update({ action: "medium-upgrade", media: { first: "audio", rest: ["video"] } },
                current.topologyRevision, { ...options, operationId: plan.upgrade + "-av-" + session.id }));
            counters.upgrades_completed++;
            mark("upgraded-snapshot", sid);
            const next = await session.refresh(options);
            assert.equal(next.id, current.id);
            assert.deepEqual([...next.media].sort(), ["audio", "data", "video"]);
            assert.ok(next.topologyRevision > current.topologyRevision);
            mark("unsupported-screen", sid);
            await refused(() => complete(session.update({ action: "medium-upgrade", media: { first: "screen", rest: [] } },
                next.topologyRevision, { ...options, operationId: plan.unsupported + "-live-" + sid })), 501, "unsupported_feature");
            assert.deepEqual(await session.refresh(options), next);
            upgraded.push({ session_index: sid, before: [...current.media].sort(), after: [...next.media].sort() });
        }
        mark("post-upgrade-original-snapshots");
        const after = await Promise.all(connections.map(v => v.connection.refresh(options)));
        assert.ok(after.every((v, index) => ["ready", "active"].includes(v.state) &&
            v.id === originals[index].id && v.generation === originals[index].generation));
        await exchange("after");
        // Release these original slots before the declared six-Connection AV scene.
        // An end receipt alone does not establish the durable retirement observation.
        for (const [index, item] of connections.entries()) {
            mark("end", item.sid, item.role);
            const began = performance.now();
            const ended = await item.connection.end({ reasonCode: 200, reason: "Synthetic upgrade stage complete" },
                { ...mutation(item.prefix + "-end"), timeoutMs: 5000 });
            let remaining = Math.floor(5000 - (performance.now() - began));
            assert.ok(remaining > 0);
            mark("end-completion", item.sid, item.role);
            const result = await ended.wait({ timeoutMs: remaining }); assert.equal(result.snapshot.state, "completed");
            counters.ends_completed++;
            mark("retirement-snapshot", item.sid, item.role);
            for (;;) {
                remaining = Math.floor(5000 - (performance.now() - began)); assert.ok(remaining > 0);
                const state = await item.connection.refresh({ ...options, timeoutMs: remaining });
                assert.equal(state.id, originals[index].id);
                if (state.state === "retired" && state.generation > originals[index].generation) { counters.retirements_observed++; break; }
                await new Promise(done => setTimeout(done, Math.min(50, remaining)));
            }
        }
        passed = true;
        return { messages, observation: { original_connections: 4, original_sessions: 2,
            verified_payloads_before: 4, verified_payloads_after: 4, bytes_per_payload: 4096,
            payload_sha256: payloadDigests, original_ids_and_generations_preserved_across_upgrade: true,
            current_unoffered_screen_refusals: 2, upgraded_sessions: upgraded, original_connections_retired: 4,
            scope: "Actual data packets before/after capability addition on the same original live Connections. Subsequent AV decoding is separate; no claim of uninterrupted preexisting audio, physical Device replacement or complete cell qualification." } };
    } finally {
        observe({ format: "connect/live-session-upgrade-work-observation/1", passed,
            elapsed_ms: performance.now() - began, failure: passed ? null : context, ...counters,
            scope: "Original awaited stage and bounded completed counters. No retry, payload, identity, media-readiness or full-cell qualification." });
    }
}
