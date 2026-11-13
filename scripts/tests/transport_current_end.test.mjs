import test from "node:test";
import assert from "node:assert/strict";
import { endRetainedConnections } from "../lib/transport_current_end.mjs";
function scenario(mode = "normal") {
    let clock = 0;
    const controller = new AbortController(), saved = [], stops = [], sends = [], waits = [], reads = [];
    const uncertain = Object.assign(Error("original end uncertain"), { certainty: "uncertain" });
    const originalRefused = Object.assign(Error("original end forbidden"), { kind: "authorization", certainty: "refused" });
    const laterOriginal = Object.assign(Error("later original end forbidden"), { kind: "authorization", certainty: "refused" });
    const observationFailure = Object.assign(Error("later snapshot unavailable"), { kind: "deadline", certainty: "read-only" });
    const routes = Array.from({ length: 6 }, (_, index) => ({ generation: 1, sessionIndex: index < 4 ? 0 : 1,
        async stopAndJoin() { stops.push(index); if (mode === "stop-at-cutoff") clock = 67000; },
        connection: { id: "connection-" + index,
            async end(input, options) {
                assert.equal(clock >= 62000, true); assert.equal(stops.includes(index), true);
                assert.deepEqual(input, { reasonCode: 200, reason: "Synthetic workload complete" });
                assert.equal(options.operationId, "end-" + index); sends.push(index);
                assert.equal(saved.filter(v => v.kind === "intent").length, 4);
                if (mode === "unknown" && index === 1) throw uncertain;
                if (["original-then-snapshot-failure", "original-then-cutoff"].includes(mode) && index === 1) throw originalRefused;
                if (mode === "ordered-original-failures" && index === 2) throw originalRefused;
                if (mode === "ordered-original-failures" && index === 5) throw laterOriginal;
                return { referenceJson: () => ({ operation_id: options.operationId, command: "connection.end",
                    scope: Object.assign(Object.create(null), { conversation_id: "room", session_id: "session-" + (index < 4 ? 0 : 1), connection_id: "connection-" + index }) }),
                    async wait(options) { waits.push(index); assert.ok(options.timeoutMs <= 5000);
                        if (mode === "late-result") clock = 67000;
                        return { snapshot: { state: "completed" }, value: { id: mode === "foreign-result" && index === 1 ? "foreign" : "connection-" + index } };
                    } };
            },
            async refresh(options) {
                reads.push(index); assert.ok(options.timeoutMs <= 5000);
                if (mode === "auth-refusal" && index === 1) throw { problem: { code: 403, reason: "forbidden" } };
                return { id: "connection-" + index, state: mode === "staggered" && index === 5 && reads.filter(v => v === index).length === 1 ? "ready" : "retired", generation: 2 };
            } } }));
    if (mode === "regressing-generation") for (const index of [1, 2, 3, 5]) routes[index].generation = 2;
    const batches = [], roleOwners = new Map();
    const roles = ["one", "two", "three", "four", "one", "two"];
    const clients = roles.map(role => {
        if (!roleOwners.has(role)) roleOwners.set(role, { identityId: "same-identity-does-not-merge-credentials", raw: { read: async (request, options) => {
            const indices = [1, 2, 3, 5].filter(index => roles[index] === role);
            batches.push({ client: roleOwners.get(role), indices: [...indices], request, options });
            if (mode === "read-503" && batches.filter(batch => batch.client === roleOwners.get(role)).length === 1) {
                reads.push(...indices);throw { problem: { code: 503, reason: "storage-unavailable" } };
            }
            if (mode === "original-then-snapshot-failure" && role === "two") throw observationFailure;
            const items = [];
            // Actual snapshot extraction/validation executes in production helper.
            for (const index of indices) {
                const current = await routes[index].connection.refresh(options);
                items.push({ kind: "connection", data: { connection_id: current.id,
                    session_id: "session-" + routes[index].sessionIndex, state: current.state, generation: current.generation } });
            }
            if (mode === "late-read" || (mode === "original-then-cutoff" && role === "two")) clock = 67000;
            if (mode === "wrong-session") items[0].data.session_id = "foreign-session";
            if (mode === "regressing-generation") items[0].data.generation = 1;
            return [{ value: { type: "ack", payload: {} } }, { value: { type: "conversation.snapshot",
                payload: { revision: 9, cursor: "original-watermark", has_more: false, items } } }];
        } } });
        return roleOwners.get(role);
    });
    const args = { routes, clients, ends: [1, 2, 3, 5].map(index => ({ index, operationId: "end-" + index })),
        conversationId: "room", sessions: [{ id: "session-0" }, { id: "session-1" }], startedAt: 0,
        signal: controller.signal, now: () => clock,
        async wait(ms) { if (mode === "cancel-before-end") { controller.abort(uncertain); throw uncertain; } clock += ms; },
        async persist(index, kind, value) { saved.push({ index, kind, value }); } };
    return { args, saved, stops, sends, waits, reads, batches, uncertain, originalRefused, laterOriginal, observationFailure, controller };
}
test("four retained current owners end after original media; moved0/4 and shared SDK are untouched", async () => {
    const s = scenario(), result = await endRetainedConnections(s.args);
    assert.equal(result.retained_connections_retired, 4); assert.equal(result.shared_sdk_closed, false);
    assert.deepEqual(s.sends, [1, 2, 3, 5]); assert.deepEqual(s.stops, [1, 2, 3, 5]);
    assert.deepEqual([...s.waits].sort((a, b) => a - b), [1, 2, 3, 5]); assert.equal(s.saved.filter(v => v.kind === "receipt").length, 4);
    assert.ok(result.receipts.every(v => v.mutation_submissions === 1 && v.original_cutoff_monotonic_ms === 67000));
});
test("early programme timer wake waits for the original boundary before any end", async () => {
    const s = scenario();
    let sleeps = 0;
    const advance = s.args.wait;
    // Model the timer returning just before a fractional monotonic deadline.
    s.args.wait = async ms => {
        assert.equal(s.sends.length, 0);
        assert.equal(s.stops.length, 0);
        sleeps++;
        await advance(sleeps === 1 ? 61999.25 : ms);
    };
    const result = await endRetainedConnections(s.args);
    assert.equal(sleeps, 2);
    assert.deepEqual(s.sends, [1, 2, 3, 5]);
    assert.ok(result.receipts.every(v => v.original_cutoff_monotonic_ms === 67000));
});
test("early programme timer wake retains cancellation before every end", async () => {
    const s = scenario();
    let sleeps = 0;
    const advance = s.args.wait;
    s.args.wait = async () => {
        if (++sleeps === 1) { await advance(61999.25); return; }
        s.controller.abort(s.uncertain);
        throw s.uncertain;
    };
    await assert.rejects(endRetainedConnections(s.args), failure => failure === s.uncertain);
    assert.equal(sleeps, 2);
    assert.equal(s.sends.length, 0);
    assert.equal(s.stops.length, 0);
});
test("uncertain original is never submitted twice and all sibling originals settle", async () => {
    const s = scenario("unknown"); await assert.rejects(endRetainedConnections(s.args), error => error === s.uncertain);
    assert.deepEqual(s.sends, [1, 2, 3, 5]); assert.deepEqual([...s.waits].sort((a, b) => a - b), [2, 3, 5]);
    assert.equal(s.saved.filter(v => v.kind === "receipt").length, 3);
});
test("packet join consuming original cutoff prevents every mutation", async () => {
    const s = scenario("stop-at-cutoff"); await assert.rejects(endRetainedConnections(s.args)); assert.equal(s.sends.length, 0); assert.deepEqual(s.stops, [1, 2, 3, 5]);
});
test("late original completion cannot pass or renew a deadline", async () => {
    const s = scenario("late-result"); await assert.rejects(endRetainedConnections(s.args));
    assert.ok(s.sends.every((v, index, all) => all.indexOf(v) === index)); assert.equal(s.saved.filter(v => v.kind === "receipt").length, 0);
});
test("foreign current connection result fails while sibling outcomes remain joined", async () => {
    const s = scenario("foreign-result"); await assert.rejects(endRetainedConnections(s.args)); assert.equal(s.sends.length, 4);
    assert.equal(s.saved.filter(v => v.kind === "receipt").length, 3);
});
test("503 retries only retirement metadata under the unchanged cutoff", async () => {
    const s = scenario("read-503"), result = await endRetainedConnections(s.args);
    assert.equal(result.retained_connections_retired, 4); assert.equal(s.sends.length, 4); assert.equal(s.reads.length, 8);
    assert.ok(result.receipts.every(v => v.original_cutoff_monotonic_ms === 67000));
});
test("current authorization refusal has no read retry or new end", async () => {
    const s = scenario("auth-refusal"); await assert.rejects(endRetainedConnections(s.args));
    assert.equal(s.reads.filter(v => v === 1).length, 1); assert.equal(s.sends.length, 4);
});
test("cancellation before original programme end performs no mutation", async () => {
    const s = scenario("cancel-before-end"); await assert.rejects(endRetainedConnections(s.args), error => error === s.uncertain); assert.equal(s.sends.length, 0);
});
test("wrong route set and duplicated original end keys refuse before custody or IO", async () => {
    for (const change of [s => { s.args.ends[0].index = 0; }, s => { s.args.ends[0].operationId = s.args.ends[1].operationId; }]) {
        const s = scenario(); change(s); await assert.rejects(endRetainedConnections(s.args));
        assert.equal(s.saved.length, 0); assert.equal(s.sends.length, 0);
    }
});


test("two original routes share one fresh snapshot only for the exact SDK object and Conversation", async () => {
    const s = scenario(), result = await endRetainedConnections(s.args);
    assert.equal(result.receipts.length, 4);
    assert.equal(s.batches.length, 3, "four per-handle refreshes must collapse to three authorized owner reads");
    assert.equal(s.args.clients[1], s.args.clients[5]);
    assert.deepEqual(s.batches.find(batch => batch.client === s.args.clients[1]).indices, [1, 5]);
    assert.equal(new Set(s.batches.map(batch => batch.client)).size, 3);
    assert.equal(new Set(s.batches.map(batch => batch.client.identityId)).size, 1,
        "equal identity strings must not merge distinct credential owners");
    assert.ok(s.batches.every(batch => batch.request.cid === "room" && batch.request.type === "conversation.get"));
    assert.equal(s.sends.length, 4); assert.equal(s.waits.length, 4);
});

test("a later observation is a new authorized read and cannot reuse an earlier retired sibling", async () => {
    const s = scenario("staggered"), result = await endRetainedConnections(s.args);
    const ownerReads = s.batches.filter(batch => batch.client === s.args.clients[1]);
    assert.equal(ownerReads.length, 2);
    assert.notEqual(ownerReads[0].request.id, ownerReads[1].request.id);
    assert.ok(ownerReads[1].options.timeoutMs < ownerReads[0].options.timeoutMs);
    assert.deepEqual(result.receipts.map(receipt => receipt.index), [1, 2, 3, 5]);
    assert.equal(s.saved.filter(value => value.kind === "receipt" && value.index === 1).length, 1);
    assert.equal(s.saved.filter(value => value.kind === "receipt" && value.index === 5).length, 1);
    assert.equal(s.sends.length, 4);
});

test("a successful snapshot arriving after the original global cutoff cannot record a retirement", async () => {
    const s = scenario("late-read");
    await assert.rejects(endRetainedConnections(s.args), /original-current-end-cutoff/);
    assert.equal(s.stops.length, 4); assert.equal(s.sends.length, 4); assert.equal(s.waits.length, 4);
    assert.equal(s.saved.filter(value => value.kind === "receipt").length, 0);
});

test("fresh grouped metadata cannot substitute a foreign Session or regress the admitted generation", async () => {
    for (const mode of ["wrong-session", "regressing-generation"]) {
        const s = scenario(mode);
        await assert.rejects(endRetainedConnections(s.args));
        assert.equal(s.stops.length, 4); assert.equal(s.sends.length, 4);
        assert.equal(s.saved.filter(value => value.kind === "receipt").length, 0);
    }
});


test("an original end refusal survives a later same-owner snapshot or cutoff failure after all groups settle", async () => {
    for (const mode of ["original-then-snapshot-failure", "original-then-cutoff"]) {
        const s = scenario(mode);
        await assert.rejects(endRetainedConnections(s.args), error => error === s.originalRefused);
        assert.deepEqual(s.stops, [1, 2, 3, 5]); assert.deepEqual(s.sends, [1, 2, 3, 5]);
        assert.deepEqual([...s.waits].sort((a, b) => a - b), [2, 3, 5]);
        assert.ok(s.batches.some(batch => batch.client === s.args.clients[5]),
            "successful original sibling must still attempt current observation");
        if (mode === "original-then-snapshot-failure") {
            assert.deepEqual(s.saved.filter(value => value.kind === "receipt").map(value => value.index).sort(), [2, 3]);
        }
    }
});

test("grouping routes1 and5 never reorders original end failures before route2", async () => {
    const s = scenario("ordered-original-failures");
    await assert.rejects(endRetainedConnections(s.args), error => error === s.originalRefused && error !== s.laterOriginal);
    assert.deepEqual(s.sends, [1, 2, 3, 5]); assert.deepEqual(s.waits, [1, 3]);
    assert.deepEqual(s.saved.filter(value => value.kind === "receipt").map(value => value.index).sort(), [1, 3]);
});


test("shared exact SDK never begins an original wait before sibling End acknowledgements after staggered packet joins", async () => {
    let clock = 0, releaseSlowJoin, releaseLongWait;
    const slowJoin = new Promise(resolve => { releaseSlowJoin = resolve; });
    const longWait = new Promise(resolve => { releaseLongWait = resolve; });
    const events = [], acknowledgements = new Set(), stops = [], models = [];
    let slowJoinEntered = false, slowJoinReleased = false;
    function model(indices) {
        let active = false;
        const queue = [];
        const client = { identityId: "same-identity-never-merges-distinct-sdk-objects", raw: { read: () => request("snapshot", null, async () => [
            { value: { type: "ack", payload: {} } },
            { value: { type: "conversation.snapshot", payload: {
                revision: 9, cursor: "original-watermark", has_more: false,
                items: indices.map(index => ({ kind: "connection", data: {
                    connection_id: "connection-" + index,
                    session_id: "session-" + (index < 4 ? 0 : 1),
                    state: "retired", generation: 2,
                } })),
            } } },
        ]) } };
        function pump() {
            if (active || !queue.length) return;
            const job = queue.shift(); active = true;
            events.push({ stage: "dispatch", kind: job.kind, index: job.index, group: [...indices] });
            Promise.resolve().then(job.body).then(value => {
                events.push({ stage: "settled", kind: job.kind, index: job.index, group: [...indices] });
                job.resolve(value);
            }, job.reject).finally(() => {
                active = false; pump();
            });
        }
        function request(kind, index, body) {
            return new Promise((resolve, reject) => { queue.push({ kind, index, body, resolve, reject }); pump(); });
        }
        models.push({ idle: () => !active && queue.length === 0 });
        return { client, request, indices };
    }
    // Exactly routes1/5 share one SDK object, as in the actual native fixture.
    // Other clients remain separate even with the same textual identity.
    const shared = model([1, 5]), second = model([2]), third = model([3]);
    const byIndex = new Map([[1, shared], [5, shared], [2, second], [3, third]]);
    const controller = new AbortController();
    const routes = Array.from({ length: 6 }, (_, index) => ({
        generation: 1, sessionIndex: index < 4 ? 0 : 1,
        async stopAndJoin() {
            stops.push(index);
            if (index === 1) { slowJoinEntered = true; await slowJoin; slowJoinReleased = true; }
        },
        connection: { id: "connection-" + index, async end(_input, options) {
            assert.equal(options.operationId, "end-" + index);
            assert.ok(options.timeoutMs > 0 && options.timeoutMs <= 5000);
            const owner = byIndex.get(index);
            return owner.request("end", index, async () => {
                acknowledgements.add(index);
                return { referenceJson: () => ({ operation_id: options.operationId, command: "connection.end",
                    scope: { conversation_id: "room", session_id: "session-" + (index < 4 ? 0 : 1), connection_id: "connection-" + index } }),
                    async wait(waitOptions) {
                        assert.ok(waitOptions.timeoutMs > 0 && waitOptions.timeoutMs <= 5000);
                        events.push({ stage: "wait-called", index, group: [...owner.indices], acknowledged: [...acknowledgements] });
                        return owner.request("wait", index, async () => {
                            if (index === 5) await longWait;
                            return { snapshot: { state: "completed" }, value: { id: "connection-" + index } };
                        });
                    } };
            });
        } },
    }));
    const saved = [], clients = routes.map((_, index) => byIndex.get(index)?.client ?? { raw: { read() { throw Error("moved owner must not be observed"); } } });
    const outcome = endRetainedConnections({ routes, clients,
        ends: [1, 2, 3, 5].map(index => ({ index, operationId: "end-" + index })),
        conversationId: "room", sessions: [{ id: "session-0" }, { id: "session-1" }], startedAt: 0,
        signal: controller.signal, now: () => clock,
        async wait(ms) { clock += ms; },
        async persist(index, kind, value) { saved.push({ index, kind, value }); },
    }).then(value => ({ value }), error => ({ error }));
    let beforeRelease;
    try {
        // One real event-loop turn drains all ready promise continuations;
        // neither release depends on observing the bug or a fabricated timeout.
        await new Promise(resolve => setImmediate(resolve));
        beforeRelease = { slow_join_entered: slowJoinEntered, slow_join_released: slowJoinReleased,
            shared_wait_started: events.some(event => event.stage === "wait-called" && event.group.length === 2),
            independent_waits_completed: [2, 3].every(index => events.some(event =>
                event.stage === "settled" && event.kind === "wait" && event.index === index)),
            independent_snapshots_completed: [2, 3].every(index => events.some(event =>
                event.stage === "settled" && event.kind === "snapshot" && event.group.length === 1 && event.group[0] === index)) };
    } finally {
        releaseSlowJoin(); releaseLongWait();
    }
    const settled = await outcome;
    // Drain the original model pumps before any intended old-condition oracle.
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(settled.error, undefined);
    assert.equal(settled.value.retained_connections_retired, 4);
    assert.equal(beforeRelease.slow_join_entered, true);
    assert.equal(beforeRelease.slow_join_released, false);
    assert.equal(beforeRelease.independent_waits_completed, true, "independent SDK waits must complete while the shared sibling join is held");
    assert.equal(beforeRelease.independent_snapshots_completed, true, "independent SDK current snapshots must complete without a global admission barrier");
    assert.equal(shared.client.identityId, second.client.identityId);
    assert.equal(shared.client.identityId, third.client.identityId);
    assert.equal(slowJoinReleased, true);
    assert.deepEqual(stops, [1, 2, 3, 5]);
    assert.deepEqual([...acknowledgements].sort((a, b) => a - b), [1, 2, 3, 5]);
    assert.equal(saved.filter(value => value.kind === "reference").length, 4);
    assert.equal(saved.filter(value => value.kind === "receipt").length, 4);
    assert.ok(models.every(owner => owner.idle()), "all original serial model queues must drain");
    const sharedWaits = events.filter(event => event.stage === "wait-called" && event.group.length === 2);
    assert.equal(sharedWaits.length, 2);
    console.error("current-end-shared-sdk counter", JSON.stringify({ original_queues_drained: true,
        original_packet_joins_drained: true, original_ends: acknowledgements.size,
        shared_wait_before_slow_join_release: beforeRelease.shared_wait_started,
        independent_waits_completed_while_shared_join_held: beforeRelease.independent_waits_completed,
        independent_snapshots_completed_while_shared_join_held: beforeRelease.independent_snapshots_completed,
        all_shared_waits_after_both_end_acknowledgements: sharedWaits.every(event => [1, 5].every(index => event.acknowledged.includes(index))) }));
    assert.ok(sharedWaits.every(event => [1, 5].every(index => event.acknowledged.includes(index))),
        "shared exact SDK began an original wait before sibling End acknowledgement");
});
