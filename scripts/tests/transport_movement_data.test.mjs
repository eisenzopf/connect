import assert from "node:assert/strict";
import test from "node:test";
import { mkdir, mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { exchangeMovementData, validatePhysicalData, verifyPhysicalDataReceipt } from "../lib/transport_movement_data.mjs";
const contract = () => ({
    incoming: { operation_id: "incoming-op", message_id: "incoming-message", stream_id: "retained-data",
        from_participant: "upstream-participant", to_participant: "physical-participant", body_base64: Buffer.alloc(65536, 7).toString("base64") },
    outgoing: { operation_id: "outgoing-op", message_id: "outgoing-message", stream_id: "new-physical-data",
        from_participant: "physical-participant", to_participant: "downstream-participant", body_base64: Buffer.alloc(65536, 8).toString("base64") },
});
function receipt(original) {
    return { format: "connect/physical-data-exchange/1", root_operation_id: "root", conversation_id: "room", session_id: "session", connection_id: "physical",
        incoming_operation_id: original.incoming.operation_id, incoming_message_id: original.incoming.message_id,
        outgoing_operation_id: original.outgoing.operation_id, outgoing_message_id: original.outgoing.message_id,
        incoming_exact_body_base64: original.incoming.body_base64, incoming_received: true, outgoing_core_queue_admission: true,
        original_root_completed_observed: true, outgoing_mutation_submissions: 1, outgoing_recovered_read_only: false,
        cutoff_programme_ms: 10000, observed_programme_ms: 2100, remote_delivery_inferred: false };
}
async function scenario(mode) {
    const root = resolve(".local"); await mkdir(root, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(root, "movement-data-"));
    const original = contract(), control = new AbortController();
    let clock = 2000, sends = 0, waits = 0, restores = 0, receives = 0;
    const uncertainty = Object.assign(Error("original send uncertain"), { certainty: "uncertain" });
    const reference = { schema: "connect-operation-reference/1", operation_id: original.incoming.operation_id, command: "message.send",
        scope: { conversation_id: "room", session_id: "session", connection_id: "upstream" } };
    const sdkJson = value => JSON.parse(JSON.stringify(value), (_, value) => value && typeof value === "object" && !Array.isArray(value) ? Object.assign(Object.create(null), value) : value);
    const operation = { referenceJson: () => {
        const captured = structuredClone(reference);
        if (mode === "foreign-reference") captured.scope.connection_id = "foreign";
        if (mode === "changed-reference-key") captured.operation_id = "different-original";
        if (mode === "extra-reference-field") captured.extra = true;
        return sdkJson(captured);
    }, async wait(options) {
        waits++; assert.ok(options.timeoutMs <= 8000);
        return { snapshot: { state: "completed" }, value: sdkJson({ messageId: original.incoming.message_id,
            connectionId: mode === "foreign-acceptance" ? "foreign" : "upstream", boundary: "core-queue-admission" }) };
    } };
    const upstream = { id: "upstream", streams: { byId(id) {
        assert.equal(id, original.incoming.stream_id);
        return { media: { async sendData(input, options) {
            sends++; assert.equal(sends, 1); assert.equal(options.operationId, original.incoming.operation_id);
            assert.equal(input.messageId, original.incoming.message_id);
            assert.deepEqual(input.to, { first: original.incoming.to_participant, rest: [] });
            assert.deepEqual(Buffer.from(input.payload.value), Buffer.alloc(65536, 7));
            const persisted = JSON.parse(await readFile(join(directory, "data-upstream-original.json"), "utf8"));
            assert.deepEqual(persisted.reference, reference); assert.deepEqual(persisted.message, original.incoming);
            const marker = JSON.parse(await readFile(join(directory, "target-observation.data-start.json"), "utf8"));
            assert.equal(marker.connection_id, "physical"); assert.equal(marker.cutoff_programme_ms, 10000);
            if (mode === "unknown") throw uncertainty;
            if (mode === "refused") throw Object.assign(Error("current authorization refused"), { certainty: "read-only" });
            return operation;
        } } };
    } } };
    const downstream = { id: "downstream", streams: { byId(id) {
        assert.equal(id, original.outgoing.stream_id);
        return { media: { async receiveData() {
            receives++;
            const value = receipt(original);
            if (mode === "foreign-physical") value.connection_id = "foreign";
            if (mode === "late-physical") value.observed_programme_ms = value.cutoff_programme_ms;
            if (mode !== "missing-physical") {
                const pending = join(directory, "receipt.pending");
                await writeFile(pending, JSON.stringify(value), { mode: 0o600 });
                await rename(pending, join(directory, "target-observation.data-receipt.json"));
            }
            if (mode === "cancelled") control.abort(uncertainty);
            const payload = Buffer.alloc(65536, 8);
            if (mode === "corrupted") payload[444] = 9;
            return { messageId: original.outgoing.message_id, operationId: original.outgoing.operation_id,
                fromParticipant: mode === "foreign-sender" ? "foreign" : original.outgoing.from_participant,
                payload: { kind: "binary", value: payload } };
        } } };
    } } };
    try {
        const run = exchangeMovementData({ workload: { startedAt: 0 }, owner: { connection: { id: "physical" } },
            rootOperationId: "root", contract: original, directory, conversationId: "room", sessionId: "session",
            upstream, downstream, signal: control.signal, now: () => clock,
            upstreamClient: { operationKinds: { data_send: "data-kind" }, operations: { restore(saved, kind) {
                restores++; assert.deepEqual(saved, reference); assert.equal(kind, "data-kind"); return operation;
            } } }, async wait(ms) { clock += ms; } });
        if (!["normal", "unknown"].includes(mode)) {
            await assert.rejects(run); assert.equal(sends, 1); assert.equal(receives, 1);
            assert.equal(restores, 0); return;
        }
        const result = await run;
        assert.equal(sends, 1); assert.equal(waits, 1); assert.equal(receives, 1);
        assert.equal(restores, mode === "unknown" ? 1 : 0);
        assert.equal(result.physical_incoming_exact_bytes, 65536);
        assert.equal(result.downstream.exact_received_bytes, 65536);
        assert.equal(result.original_cutoff_programme_ms, 10000);
        assert.equal(result.mutations_replayed, false); assert.equal(result.messages.length, 2);
        assert.deepEqual(JSON.parse(await readFile(join(directory, "data-exchange.json"), "utf8")), result);
    } finally { await rm(directory, { recursive: true, force: true }); }
}
test("one original exchange uses the retained upstream stream and exact new downstream stream with durable prior intents", () => scenario("normal"));
test("an uncertain SDK send recovers the saved original operation by read only and never resubmits", () => scenario("unknown"));
test("a refused original joins both receive observers without mutation retry", () => scenario("refused"));
test("foreign upstream acceptance cannot pass a replacement Data exchange", () => scenario("foreign-acceptance"));
test("null-prototype SDK references still reject changed scope, original key and additional JSON fields", async () => {
    for (const mode of ["foreign-reference", "changed-reference-key", "extra-reference-field"]) await scenario(mode);
});
test("a physical receipt for another Connection cannot pass native acceptance and exact downstream bytes", () => scenario("foreign-physical"));
test("a late physical receipt cannot renew the original eight-second exchange cutoff", () => scenario("late-physical"));
test("missing physical bytes are bounded by the original cutoff rather than inferred from SDK admission", () => scenario("missing-physical"));
test("wrong downstream sender or one altered payload byte refuses exact Data qualification", async () => {
    await scenario("foreign-sender"); await scenario("corrupted");
});
test("parent cancellation joins every original observer without resubmission", () => scenario("cancelled"));
test("partial, noncanonical or cross-audience Data contracts cannot shrink the64KiB fixture", () => {
    for (const change of [c => { c.incoming.body_base64 = Buffer.alloc(65535).toString("base64"); },
        c => { c.incoming.body_base64 += "\n"; }, c => { c.outgoing.from_participant = "foreign"; },
        c => { c.outgoing.operation_id = c.incoming.operation_id; }, c => { c.outgoing.stream_id = c.incoming.stream_id; },
        c => { c.incoming.to_participant = c.incoming.from_participant; }, c => { c.incoming.extra = true; }]) {
        const value = contract(); change(value); assert.throws(() => validatePhysicalData(value));
    }
});
test("receipt verification binds full scope, exact originals, source bytes and the original cutoff", () => {
    const original = contract(), options = { contract: original, conversationId: "room", sessionId: "session", connectionId: "physical", rootOperationId: "root", cutoff: 10000 };
    verifyPhysicalDataReceipt(receipt(original), options);
    for (const key of ["root_operation_id", "conversation_id", "session_id", "connection_id", "incoming_operation_id", "outgoing_message_id", "incoming_exact_body_base64"])
        assert.throws(() => verifyPhysicalDataReceipt({ ...receipt(original), [key]: "foreign" }, options));
    assert.throws(() => verifyPhysicalDataReceipt({ ...receipt(original), cutoff_programme_ms: 10001 }, options));
    assert.throws(() => verifyPhysicalDataReceipt({ ...receipt(original), outgoing_mutation_submissions: 2 }, options));
});
