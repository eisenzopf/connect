import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson, jsonObject } from "./uctp_json.js";
const fields = [
    "recordId",
    "operationId",
    "conversationId",
    "sessionId",
    "sourceConnectionId",
    "retainedConnectionId",
    "targetConnectionId",
    "endpointId",
    "deviceId",
];
export function replacementRoot(input) {
    if (!input ||
        typeof input !== "object" ||
        Object.keys(input).length !== fields.length ||
        fields.some((field) => !identifier(input[field])) ||
        new Set([
            input.sourceConnectionId,
            input.retainedConnectionId,
            input.targetConnectionId,
        ]).size !== 3)
        throw new ConnectError("invalid-request", "Use the exact original replacement root and distinct Connections");
    return Object.freeze(Object.fromEntries(fields.map((field) => [field, input[field]])));
}
/** Validate signed original-root observations before allocation and on every
 * continuation. A preparing record is not a completed move or a media grant. */
export function replacementObservation(root, value) {
    if (value.operation_id !== root.operationId ||
        value.record_id !== root.recordId ||
        value.command_type !== "connection.replace" ||
        !Number.isSafeInteger(value.revision) ||
        Number(value.revision) < 0)
        throw new ConnectError("protocol", "Replacement observation changed the original root");
}
export function replacementFrame(root, frames, connectionId) {
    const records = frames.filter((frame) => frame.type === "operation.updated");
    if (records.length !== 1 ||
        records[0].cid !== root.conversationId ||
        records[0].sid !== root.sessionId ||
        records[0].connid !== connectionId)
        throw new ConnectError("protocol", "Replacement response changed the original scope");
    replacementObservation(root, records[0].payload);
    return records[0].payload;
}
export function replacementReceipt(root, value) {
    replacementObservation(root, value);
    const result = value.result;
    if (value.state === "completed" &&
        jsonObject(result) &&
        result.activation_state === "stopped" &&
        result.connection_id === root.targetConnectionId &&
        result.replaces === root.sourceConnectionId)
        throw new ConnectError("refused", "Original replacement completed with a stopped target; recover its original receipt");
    if (value.state !== "completed" ||
        !jsonObject(result) ||
        result.activation_state !== "ready" ||
        result.connection_id !== root.targetConnectionId ||
        result.replaces !== root.sourceConnectionId ||
        result.endpoint_id !== root.endpointId ||
        result.device_id !== root.deviceId ||
        !Number.isSafeInteger(result.generation) ||
        Number(result.generation) < 2 ||
        !Array.isArray(result.retained_connection_ids) ||
        result.retained_connection_ids.length !== 1 ||
        result.retained_connection_ids[0] !== root.retainedConnectionId ||
        !Array.isArray(result.admitted_connection_ids) ||
        result.admitted_connection_ids.length !== 1 ||
        result.admitted_connection_ids[0] !== root.targetConnectionId ||
        !Array.isArray(result.retired_connection_ids) ||
        result.retired_connection_ids.length !== 1 ||
        result.retired_connection_ids[0] !== root.sourceConnectionId ||
        !jsonObject(result.readiness) ||
        !jsonObject(result.physical_readiness) ||
        canonicalJson(result.readiness) !==
            canonicalJson(result.physical_readiness) ||
        !Array.isArray(result.readiness.streams) ||
        !result.readiness.streams.every(identifier) ||
        !Number.isSafeInteger(result.readiness.stream_revision) ||
        Number(result.readiness.stream_revision) < 3)
        throw new ConnectError("protocol", "Replacement did not complete with the original live target");
    return Object.freeze({
        recordId: root.recordId,
        generation: Number(result.generation),
        streamRevision: Number(result.readiness.stream_revision),
    });
}
