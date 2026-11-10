// Current retained SDK routes end alongside the final two physical owners.
// No helper/client closes here; each actual route retains its original scope.
import assert from "node:assert/strict";
import { connectionSnapshots } from "./transport_snapshot.mjs";
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
async function delay(milliseconds, signal) {
    if (signal.aborted) throw signal.reason;
    await new Promise((done, reject) => {
        const abort = () => { clearTimeout(timer); reject(signal.reason); };
        const timer = setTimeout(() => { signal.removeEventListener("abort", abort); done(); }, Math.max(0, milliseconds));
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) { signal.removeEventListener("abort", abort); abort(); }
    });
}
export async function endRetainedConnections({ routes, clients, ends, conversationId, sessions, startedAt,
    signal, persist, now = () => performance.now(), wait = delay }) {
    assert.equal(routes.length, 6); assert.equal(sessions.length, 2);
    assert.equal(clients.length, 6);
    assert.ok(clients.every(client => client && typeof client.raw?.read === "function"));
    assert.ok(id(conversationId) && Number.isFinite(startedAt) && startedAt >= 0);
    assert.deepEqual(ends.map(end => end.index).sort((a, b) => a - b), [1, 2, 3, 5]);
    assert.ok(ends.every(end => id(end.operationId)));
    assert.equal(new Set(ends.map(end => end.operationId)).size, 4);
    assert.equal(new Set(routes.map(route => route.connection.id)).size, 6);
    const endAt = startedAt + 62000, cutoff = startedAt + 67000;
    const current = () => { if (signal.aborted) throw signal.reason; };
    const remaining = () => {
        current(); const value = Math.floor(cutoff - now());
        assert.ok(value > 0 && value <= 5000, "original-current-end-cutoff"); return value;
    };
    current();
    const originals = ends.map(end => {
        const route = routes[end.index];
        assert.ok(id(route.connection.id) && Number.isSafeInteger(route.generation) && route.generation > 0);
        assert.ok(route.sessionIndex === 0 || route.sessionIndex === 1);
        assert.ok(id(sessions[route.sessionIndex].id));
        return { end, route, intent: {
            format: "connect/retained-current-original-end/1", type: "connection.end",
            scope: { conversation_id: conversationId, session_id: sessions[route.sessionIndex].id,
                connection_id: route.connection.id, operation_id: end.operationId },
            payload: { reason_code: 200, reason: "Synthetic workload complete" },
            programme_end_monotonic_ms: endAt, original_cutoff_monotonic_ms: cutoff,
        } };
    });
    // Save all original keys/scopes before any possible submission or wait.
    for (const { end, intent } of originals) await persist(end.index, "intent", intent);
    // Timers may wake before a fractional monotonic deadline. Recheck the
    // same programme boundary; no Connection may end early or gain more time.
    while (now() < endAt) {
        current();
        const before = now();
        await wait(Math.max(1, Math.ceil(endAt - before)), signal);
        current();
        assert.ok(now() > before, "original-current-end-wait-must-advance-clock");
    }
    current();
    assert.ok(now() >= endAt, "original-media-programme-ended-before-current-end");
    const submissions = originals.map(async ({ end, route, intent }) => {
        // Cleanup remains owned even if a sibling consumes the original budget.
        // Deadline/cancellation can forbid mutation, never abandon packet joins.
        await route.stopAndJoin();
        remaining();
        const operation = await route.connection.end({ reasonCode: 200, reason: "Synthetic workload complete" },
            { operationId: end.operationId, timeoutMs: remaining() });
        const reference = operation.referenceJson();
        assert.equal(reference.operation_id, end.operationId);
        assert.equal(reference.command, "connection.end");
        assert.deepEqual(JSON.parse(JSON.stringify(reference.scope)), { conversation_id: conversationId,
            session_id: sessions[route.sessionIndex].id, connection_id: route.connection.id });
        await persist(end.index, "reference", reference);
        const completed = await operation.wait({ timeoutMs: remaining() });
        assert.equal(completed.snapshot.state, "completed");
        assert.equal(completed.value.id, route.connection.id);
        const receiptAt = now(); remaining();
        return { end, route, reference, receiptAt };
    });
    // Group only by the exact SDK used to offer each original Connection.
    // An identical identity/Device or shared Conversation never merges clients.
    const groups = new Map();
    originals.forEach((original, index) => {
        const client = clients[original.end.index];
        if (!groups.has(client)) groups.set(client, []);
        groups.get(client).push(submissions[index]);
    });
    const results = await Promise.allSettled([...groups].map(async ([client, work]) => {
        // Original mutations/waits remain independent and occur exactly once.
        // A failed sibling is preserved while successful siblings still observe.
        const ended = await Promise.allSettled(work);
        const failure = ended.find(value => value.status === "rejected");
        let pending = ended.filter(value => value.status === "fulfilled").map(value => value.value);
        const receipts = [];
        while (pending.length) {
            let snapshot;
            try {
                snapshot = await connectionSnapshots(client, conversationId, pending.map(({ route }) => ({
                    id: route.connection.id, sessionId: sessions[route.sessionIndex].id,
                })), { timeoutMs: remaining(), signal });
            } catch (failure) {
                if (!(failure.problem?.code === 503 && failure.problem?.reason === "storage-unavailable")) throw failure;
            }
            remaining();
            if (snapshot) {
                const next = [];
                for (const [index, original] of pending.entries()) {
                    const { end, route, reference, receiptAt } = original, current = snapshot.connections[index];
                    assert.equal(current.id, route.connection.id);
                    assert.equal(current.sessionId, sessions[route.sessionIndex].id);
                    assert.ok(Number.isSafeInteger(current.generation) && current.generation >= route.generation);
                    if (current.state !== "retired" || current.generation <= route.generation) { next.push(original);continue; }
                    const result = { index: end.index, connection_id: route.connection.id, operation_id: end.operationId,
                        original_reference: reference, mutation_submissions: 1,
                        end_receipt_ms: receiptAt - endAt, retirement_observed_ms: now() - endAt,
                        original_cutoff_monotonic_ms: cutoff, limit_ms: 5000, owned_packet_tasks_joined: true };
                    await persist(end.index, "receipt", result); remaining();receipts.push(result);
                }
                pending = next;
            }
            if (pending.length) await wait(Math.min(50, remaining()), signal);
        }
        if (failure) throw failure.reason;
        return receipts;
    }));
    // All group observations are joined before choosing the outcome. Re-await
    // the already-settled original promises; this performs no new IO or replay.
    // A later snapshot/cutoff error must not hide an original end/wait failure,
    // and grouping routes1+5 must not reorder original failures ahead of route2.
    const originalsSettled = await Promise.allSettled(submissions);
    const originalFailure = originalsSettled.find(result => result.status === "rejected");
    if (originalFailure) throw originalFailure.reason;
    const failure = results.find(result => result.status === "rejected");
    if (failure) throw failure.reason;
    return { retained_connections_retired: 4, original_end_mutations: 4,
        shared_sdk_closed: false, original_cutoff_monotonic_ms: cutoff,
        receipts: results.flatMap(result => result.value).sort((a, b) => a.index - b.index) };
}
