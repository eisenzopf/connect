import assert from "node:assert/strict";
import test from "node:test";
import { connectionSnapshots } from "../lib/transport_snapshot.mjs";

const originals = Array.from({ length: 6 }, (_, index) => ({ id: "connection_" + index, sessionId: "session_" + (index % 2) }));
const item = (original, state = "retired") => ({ kind: "connection", data: {
    connection_id: original.id, session_id: original.sessionId, state, generation: 3,
} });
const page = (items, more = false, next = undefined) => ({ revision: 9, cursor: "watermark", items, has_more: more,
    ...(next ? { next_page: next } : {}) });
function client(pages, hook = () => {}) {
    const calls = [];
    return { calls, raw: { read: async (request, options) => {
        calls.push({ request, options }); hook(calls.length);
        const payload = pages[calls.length - 1]; assert.ok(payload, "unexpected-page-request");
        return [{ value: { type: "ack", payload: {} } }, { value: { type: "conversation.snapshot", payload } }];
    } } };
}
const read = sdk => connectionSnapshots(sdk, "conversation_original", originals, { timeoutMs: 8000 });

test("all six original Connections are read once from one terminal public snapshot", async () => {
    const sdk = client([page(originals.map(v => item(v)))]), result = await read(sdk);
    assert.equal(sdk.calls.length, 1); assert.equal(result.revision, 9); assert.equal(result.pages, 1);
    assert.deepEqual(result.connections.map(v => v.id), originals.map(v => v.id));
    assert.equal(sdk.calls[0].request.cid, "conversation_original");
    assert.equal(sdk.calls[0].request.type, "conversation.get");
});
test("partial pages use opaque continuation and never hide a still-ready original", async () => {
    const sdk = client([page(originals.slice(0, 3).map(v => item(v)), true, "opaque_page"),
        page(originals.slice(3).map((v, i) => item(v, i === 2 ? "ready" : "retired")))]);
    const result = await read(sdk);
    assert.equal(result.pages, 2); assert.equal(result.connections[5].state, "ready");
    assert.equal(sdk.calls[1].request.payload.page, "opaque_page");
    assert.ok(sdk.calls[1].options.timeoutMs <= sdk.calls[0].options.timeoutMs);
});
test("changed revision, watermark, duplicate or missing original and changed Session refuse", async () => {
    const first = page(originals.slice(0, 3).map(v => item(v)), true, "opaque_page");
    for (const changed of [
        { ...page(originals.slice(3).map(v => item(v))), revision: 10 },
        { ...page(originals.slice(3).map(v => item(v))), cursor: "other_watermark" },
        page([item(originals[0]), ...originals.slice(3).map(v => item(v))]),
        page(originals.slice(3, 5).map(v => item(v))),
        page(originals.slice(3).map(v => item({ ...v, sessionId: "other_session" }))),
    ]) await assert.rejects(read(client([first, changed])));
});
test("repeated continuation and empty nonterminal pages refuse rather than imply retirement", async () => {
    await assert.rejects(read(client([page([item(originals[0])], true, "same"),
        page([item(originals[1])], true, "same")])), /repeated-snapshot-page/);
    await assert.rejects(read(client([page([], true, "next")])), /empty-nonterminal-snapshot-page/);
});
test("current authority refusal propagates without retry or submitting a mutation", async () => {
    const denied = Error("current-authority-refused"), sdk = client([], () => { throw denied; });
    await assert.rejects(read(sdk), error => error === denied);
    assert.equal(sdk.calls.length, 1);
    assert.ok(sdk.calls.every(v => v.request.type === "conversation.get"));
});


const busy = () => Object.assign(Error("temporary current authority admission"), { kind: "application", certainty: "refused",
    problem: { code: 503, category: "authorization", reason: "authority_unavailable" } });
const resync = () => Object.assign(Error("current snapshot page evicted"), { kind: "conflict", certainty: "refused",
    problem: { code: 409, category: "state", reason: "resync_required" } });

test("owned raw read avoids a synchronous validator and retries only signed temporary admission under the same budget", async () => {
    const sdk = client([undefined, page(originals.map(v => item(v)))], count => { if (count === 1) throw busy(); });
    sdk.raw.validate = () => assert.fail("no synchronous validator child");
    sdk.raw.request = () => assert.fail("no separate raw request");
    const result = await read(sdk);
    assert.equal(result.pages, 1); assert.equal(sdk.calls.length, 2);
    assert.ok(sdk.calls[1].options.timeoutMs < sdk.calls[0].options.timeoutMs);
    assert.deepEqual(sdk.calls.map(call => call.request.payload), [{ limit: 100 }, { limit: 100 }]);
});

test("page eviction discards every old fact and restarts the authorized read without renewing its budget", async () => {
    const final = { ...page(originals.map(v => item(v))), revision: 10, cursor: "fresh-watermark" };
    const sdk = client([page([item(originals[0], "ready")], true, "old-page"), undefined, final],
        count => { if (count === 2) throw resync(); });
    const result = await read(sdk);
    assert.equal(result.revision, 10); assert.equal(result.pages, 1);
    assert.ok(result.connections.every(connection => connection.state === "retired"));
    assert.deepEqual(sdk.calls.map(call => call.request.payload.page), [undefined, "old-page", undefined]);
    assert.ok(sdk.calls[2].options.timeoutMs <= sdk.calls[0].options.timeoutMs);
});

test("only two continuation resyncs are allowed and first-page refusal is not retried", async () => {
    const failure = resync(), first = client([], () => { throw failure; });
    await assert.rejects(read(first), error => error === failure); assert.equal(first.calls.length, 1);
    const next = page([item(originals[0])], true, "page");
    const sdk = client([next, undefined, next, undefined, next, undefined], count => { if (count % 2 === 0) throw failure; });
    await assert.rejects(read(sdk), error => error === failure); assert.equal(sdk.calls.length, 6);
});

test("uncertain, operation-bearing and genuine authorization refusals are never retried", async () => {
    for (const failure of [Object.assign(busy(), { certainty: "uncertain" }),
        Object.assign(busy(), { operation: { operation_id: "original" } }),
        Object.assign(busy(), { observation: { state: "original" } }),
        Object.assign(busy(), { problem: { code: 403, category: "authorization", reason: "forbidden" } })]) {
        const sdk = client([], () => { throw failure; });
        await assert.rejects(read(sdk), error => error === failure); assert.equal(sdk.calls.length, 1);
    }
});

test("cancellation during temporary admission starts no later read", async () => {
    const stop = new AbortController(), sdk = client([], () => { stop.abort(); throw busy(); });
    await assert.rejects(connectionSnapshots(sdk, "conversation_original", originals, { timeoutMs: 8000, signal: stop.signal }),
        error => error.name === "AbortError");
    assert.equal(sdk.calls.length, 1);
});
