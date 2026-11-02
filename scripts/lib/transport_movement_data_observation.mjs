// Joined runtime files bind Data claims to the original plan and actual scopes.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { validatePhysicalData, verifyPhysicalDataReceipt } from "./transport_movement_data.mjs";
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
export async function physicalDataCustody({ readOriginal, folder, stage, observed, step,
    conversationId, sessionId, connectionId, predecessor, upstream, downstream, expectedExchange }) {
    const contract = validatePhysicalData(step.data);
    assert.equal(stage.physical_data_exchange_verified, true);
    const input = await readOriginal(join(folder, "target-input.json"), 32 * 1024 * 1024);
    assert.equal(sha(Buffer.from(JSON.stringify(input.value.media))), stage.source_input_sha256);
    assert.deepEqual(input.value.media.data, contract);
    assert.equal(input.value.command.operation_id, step.operationId);
    assert.equal(input.value.command.conversation_id, conversationId);
    assert.equal(input.value.command.session_id, sessionId);
    assert.equal(input.value.command.connection_id, predecessor);
    const files = {};
    for (const suffix of ["data-start", "data-original-send", "data-wire", "data-receipt", "data-upstream-original", "data-exchange"]) {
        const name = ["data-upstream-original", "data-exchange"].includes(suffix) ? suffix : "target-observation." + suffix;
        files[suffix] = await readOriginal(join(folder, name + ".json"), 2 * 1024 * 1024);
        assert.equal(files[suffix].digest, stage.physical_data_files[suffix]);
    }
    const marker = files["data-start"].value, receipt = files["data-receipt"].value;
    assert.equal(marker.format, "connect/physical-data-start/1");
    assert.equal(marker.root_operation_id, step.operationId); assert.equal(marker.connection_id, connectionId);
    assert.equal(marker.incoming_operation_id, contract.incoming.operation_id);
    assert.equal(marker.outgoing_operation_id, contract.outgoing.operation_id);
    assert.ok(Number.isSafeInteger(marker.armed_programme_ms) && marker.armed_programme_ms >= 2000);
    assert.ok(marker.cutoff_programme_ms > marker.armed_programme_ms && marker.cutoff_programme_ms <= marker.armed_programme_ms + 8000);
    verifyPhysicalDataReceipt(receipt, { contract, conversationId, sessionId, connectionId,
        rootOperationId: step.operationId, cutoff: marker.cutoff_programme_ms });
    assert.deepEqual(receipt, observed.data_exchange_receipt);
    const scope = { conversation_id: conversationId, session_id: sessionId, connection_id: connectionId,
        operation_id: contract.outgoing.operation_id };
    const original = files["data-original-send"].value;
    assert.equal(original.type, "message.send"); assert.deepEqual(original.scope, scope);
    assert.deepEqual(original.payload, { msg_id: contract.outgoing.message_id, to: [contract.outgoing.to_participant],
        label: contract.outgoing.stream_id, content_type: "application/octet-stream", body_encoding: "base64",
        body: contract.outgoing.body_base64, reliability: { mode: "reliable_ordered" } });
    const savedUpstream = files["data-upstream-original"].value;
    assert.deepEqual(savedUpstream.reference, { schema: "connect-operation-reference/1", command: "message.send",
        operation_id: contract.incoming.operation_id,
        scope: { conversation_id: conversationId, session_id: sessionId, connection_id: upstream } });
    assert.deepEqual(savedUpstream.message, contract.incoming);
    assert.equal(savedUpstream.original_cutoff_programme_ms, marker.cutoff_programme_ms);
    const parent = files["data-exchange"].value;
    assert.deepEqual(parent, expectedExchange);
    assert.equal(parent.root_operation_id, step.operationId); assert.equal(parent.connection_id, connectionId);
    assert.equal(parent.upstream_core_queue_admission, true); assert.equal(parent.physical_incoming_exact_bytes, 65536);
    assert.equal(parent.physical_receipt_sha256, files["data-receipt"].digest);
    assert.equal(parent.original_cutoff_programme_ms, marker.cutoff_programme_ms); assert.equal(parent.mutations_replayed, false);
    assert.deepEqual(parent.downstream, { message_id: contract.outgoing.message_id, exact_received_bytes: 65536,
        payload_sha256: sha(Buffer.from(contract.outgoing.body_base64, "base64")), connection_id: downstream });
    assert.deepEqual(parent.messages, Object.values(contract).map(original => ({ operation_id: original.operation_id,
        message_id: original.message_id, from_participant: original.from_participant, to_participant: original.to_participant,
        payload_sha256: sha(Buffer.from(original.body_base64, "base64")) })));
    const frames = files["data-wire"].value.frames;
    assert.ok(Array.isArray(frames) && frames.length <= 512);
    const incoming = frames.filter(frame => frame.type === "message.send"); assert.equal(incoming.length, 1);
    const frame = incoming[0];
    assert.equal(frame.cid, conversationId); assert.equal(frame.sid, sessionId); assert.equal(frame.connid, connectionId);
    assert.equal(frame.operation_id, contract.incoming.operation_id);
    assert.ok(frame.in_reply_to === undefined && frame.expected_revision === undefined);
    for (const [key, value] of Object.entries({ msg_id: contract.incoming.message_id, from: contract.incoming.from_participant,
        to: [contract.incoming.to_participant], label: contract.incoming.stream_id, content_type: "application/octet-stream",
        body_encoding: "base64", body: contract.incoming.body_base64, reliability: { mode: "reliable_ordered" } }))
        assert.ok(JSON.stringify(frame.payload[key]) === JSON.stringify(value), "original incoming Data field: " + key);
    const completion = frames.filter(frame => frame.type === "operation.updated" && frame.payload?.state === "completed" &&
        frame.payload.operation_id === contract.outgoing.operation_id);
    assert.ok(completion.length >= 1);
    for (const completed of completion) {
        assert.equal(completed.cid, conversationId); assert.equal(completed.sid, sessionId); assert.equal(completed.connid, connectionId);
        assert.equal(completed.payload.command_type, "message.send");
        assert.deepEqual(completed.payload.result, { message_id: contract.outgoing.message_id, connection_id: connectionId, core_queue_admission: true });
        assert.ok(frames.some(frame => frame.type === "ack" && frame.in_reply_to === completed.in_reply_to &&
            frame.cid === conversationId && frame.sid === sessionId && frame.connid === connectionId));
    }
    return { original_data_messages_bound: 2, incoming_exact_bytes: 65536, downstream_exact_bytes: 65536,
        original_source_input_sha256: input.digest, files_sha256: Object.fromEntries(Object.entries(files).map(([key, file]) => [key, file.digest])),
        current_runtime_verifier_used: true, independently_reverified_signatures: false };
}
