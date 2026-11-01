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
    return { calls, raw: { validate: value => value, request: async (request, options) => {
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
