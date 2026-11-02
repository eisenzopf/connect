// Two original64KiB Data messages cross each actual replacement owner.
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { link, open, unlink } from "node:fs/promises";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { SdkBinaryData } from "../../sdk/typescript/dist/index.js";
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
export function validatePhysicalData(contract) {
    assert.deepEqual(Object.keys(contract).sort(), ["incoming", "outgoing"]);
    for (const original of Object.values(contract)) {
        assert.deepEqual(Object.keys(original).sort(), ["body_base64", "from_participant", "message_id", "operation_id", "stream_id", "to_participant"]);
        assert.ok([original.operation_id, original.message_id, original.stream_id, original.from_participant, original.to_participant].every(id));
        assert.notEqual(original.from_participant, original.to_participant);
        assert.equal(typeof original.body_base64, "string");
        const bytes = Buffer.from(original.body_base64, "base64");
        assert.equal(bytes.length, 65536); assert.equal(bytes.toString("base64"), original.body_base64);
    }
    assert.equal(contract.incoming.to_participant, contract.outgoing.from_participant);
    for (const field of ["operation_id", "message_id", "stream_id", "body_base64"])
        assert.notEqual(contract.incoming[field], contract.outgoing[field]);
    return structuredClone(contract);
}
async function save(path, value, atomic = false) {
    const temporary = atomic ? path + "." + randomUUID() + ".tmp" : path;
    const file = await open(temporary, "wx", 0o600);
    try { await file.writeFile(JSON.stringify(value) + "\n"); await file.sync(); }
    finally { await file.close(); }
    if (atomic) { try { await link(temporary, path); } finally { await unlink(temporary); } }
    const directory = await open(dirname(path), "r");
    try { await directory.sync(); } finally { await directory.close(); }
}
async function privateJson(path) {
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
        const before = await file.stat();
        assert.ok(before.isFile() && (before.mode & 0o077) === 0 && before.size > 0 && before.size <= 256 * 1024);
        const bytes = await file.readFile(); const after = await file.stat();
        assert.equal(bytes.length, before.size); assert.equal(after.size, before.size);
        assert.equal(after.mtimeMs, before.mtimeMs);
        return { value: JSON.parse(bytes), sha256: hash(bytes) };
    } finally { await file.close(); }
}
export function verifyPhysicalDataReceipt(receipt, { contract, conversationId, sessionId, connectionId, rootOperationId, cutoff }) {
    validatePhysicalData(contract);
    assert.equal(receipt.format, "connect/physical-data-exchange/1");
    assert.equal(receipt.root_operation_id, rootOperationId);
    assert.equal(receipt.conversation_id, conversationId); assert.equal(receipt.session_id, sessionId);
    assert.equal(receipt.connection_id, connectionId);
    for (const direction of ["incoming", "outgoing"]) {
        assert.equal(receipt[direction + "_operation_id"], contract[direction].operation_id);
        assert.equal(receipt[direction + "_message_id"], contract[direction].message_id);
    }
    assert.ok(receipt.incoming_exact_body_base64 === contract.incoming.body_base64, "exact-physical-incoming-payload");
    for (const flag of ["incoming_received", "outgoing_core_queue_admission", "original_root_completed_observed"])
        assert.equal(receipt[flag], true);
    assert.equal(receipt.outgoing_mutation_submissions, 1);
    assert.equal(receipt.remote_delivery_inferred, false);
    assert.equal(typeof receipt.outgoing_recovered_read_only, "boolean");
    assert.ok(Number.isSafeInteger(receipt.cutoff_programme_ms) && receipt.cutoff_programme_ms > 0 && receipt.cutoff_programme_ms <= 62000);
    if (cutoff !== undefined) assert.equal(receipt.cutoff_programme_ms, cutoff);
    assert.ok(Number.isSafeInteger(receipt.observed_programme_ms) && receipt.observed_programme_ms >= 0 &&
        receipt.observed_programme_ms < receipt.cutoff_programme_ms);
}
export async function exchangeMovementData({ workload, owner, rootOperationId, contract, directory,
    conversationId, sessionId, upstream, upstreamClient, downstream, signal,
    now = () => performance.now(), wait = (ms, signal) => sleep(ms, undefined, { signal }) }) {
    validatePhysicalData(contract);
    assert.ok([conversationId, sessionId, owner.connection.id, rootOperationId, upstream.id, downstream.id].every(id));
    assert.ok(![contract.incoming.operation_id, contract.outgoing.operation_id].includes(rootOperationId));
    const armed = Math.floor(now() - workload.startedAt), cutoff = Math.min(62000, armed + 8000);
    assert.ok(armed >= 0 && armed < cutoff);
    const remaining = () => {
        if (signal.aborted) throw signal.reason;
        const value = Math.floor(workload.startedAt + cutoff - now());
        assert.ok(value > 0 && value <= 8000, "original-replacement-data-cutoff"); return value;
    };
    const reference = { schema: "connect-operation-reference/1", operation_id: contract.incoming.operation_id,
        command: "message.send", scope: { conversation_id: conversationId, session_id: sessionId, connection_id: upstream.id } };
    const original = { reference, original_cutoff_programme_ms: cutoff, message: contract.incoming };
    remaining(); await save(join(directory, "data-upstream-original.json"), original);
    await save(join(directory, "target-observation.data-start.json"), {
        format: "connect/physical-data-start/1", root_operation_id: rootOperationId, connection_id: owner.connection.id,
        incoming_operation_id: contract.incoming.operation_id, outgoing_operation_id: contract.outgoing.operation_id,
        armed_programme_ms: armed, cutoff_programme_ms: cutoff,
    }, true);
    let firstFailure;
    const guard = task => Promise.resolve().then(task).catch(failure => { firstFailure ??= failure; throw failure; });
    const results = await Promise.allSettled([
        guard(async () => {
            let operation;
            try {
                operation = await upstream.streams.byId(contract.incoming.stream_id).media.sendData({
                    messageId: contract.incoming.message_id, to: { first: contract.incoming.to_participant, rest: [] },
                    payload: new SdkBinaryData(Buffer.from(contract.incoming.body_base64, "base64")),
                }, { operationId: contract.incoming.operation_id, timeoutMs: remaining(), signal });
            } catch (failure) {
                if (failure.certainty !== "uncertain") throw failure;
                // Read the saved original key; an unknown send never resubmits.
                operation = upstreamClient.operations.restore(reference, upstreamClient.operationKinds.data_send);
            }
            // Public SDK JSON dictionaries intentionally have null prototypes.
            // Compare every serialized key/value, not their JS prototypes.
            assert.deepEqual(JSON.parse(JSON.stringify(operation.referenceJson())), reference);
            const completed = await operation.wait({ timeoutMs: remaining(), signal });
            assert.equal(completed.snapshot.state, "completed");
            assert.deepEqual(JSON.parse(JSON.stringify(completed.value)), { messageId: contract.incoming.message_id,
                connectionId: upstream.id, boundary: "core-queue-admission" });
            return completed.value;
        }),
        guard(async () => {
            const received = await downstream.streams.byId(contract.outgoing.stream_id).media.receiveData({ timeoutMs: remaining(), signal });
            assert.equal(received.messageId, contract.outgoing.message_id);
            assert.equal(received.operationId, contract.outgoing.operation_id);
            assert.equal(received.fromParticipant, contract.outgoing.from_participant);
            assert.equal(received.payload.kind, "binary");
            assert.ok(Buffer.from(received.payload.value).equals(Buffer.from(contract.outgoing.body_base64, "base64")), "exact-physical-outgoing-payload");
            return { message_id: received.messageId, exact_received_bytes: 65536,
                payload_sha256: hash(received.payload.value), connection_id: downstream.id };
        }),
        guard(async () => {
            for (;;) {
                remaining();
                try {
                    const captured = await privateJson(join(directory, "target-observation.data-receipt.json"));
                    verifyPhysicalDataReceipt(captured.value, { contract, conversationId, sessionId,
                        connectionId: owner.connection.id, rootOperationId, cutoff });
                    return captured;
                } catch (failure) { if (failure.code !== "ENOENT") throw failure; }
                await wait(Math.min(20, remaining()), signal);
            }
        }),
    ]);
    if (firstFailure) throw firstFailure;
    remaining(); assert.ok(results.every(result => result.status === "fulfilled"));
    const observation = { root_operation_id: rootOperationId, connection_id: owner.connection.id,
        upstream_core_queue_admission: true, physical_incoming_exact_bytes: 65536,
        downstream: results[1].value, physical_receipt_sha256: results[2].value.sha256,
        original_cutoff_programme_ms: cutoff, mutations_replayed: false,
        messages: Object.values(contract).map(original => ({ operation_id: original.operation_id, message_id: original.message_id,
            from_participant: original.from_participant, to_participant: original.to_participant,
            payload_sha256: hash(Buffer.from(original.body_base64, "base64")) })),
    };
    await save(join(directory, "data-exchange.json"), observation); remaining();
    return observation;
}
