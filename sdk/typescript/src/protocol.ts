import { validContextRecord } from "./record_context.js";
import { validMembership } from "./membership_validation.js";
import { validConversation } from "./conversations.js";
import { validEvents } from "./event_validation.js";
import { validMediaDiagnostics } from "./media_diagnostics.js";
import { validContact } from "./contact_validation.js";
import { array, fields, integer, object } from "./validation.js";
import { validTaskOutcome } from "./task_protocol.js";
import { ConnectError } from "./errors.js";
import type { Operation, Outcome } from "./types.js";
export const MAX_REQUEST = 65537;
export const MAX_RESPONSE = 600000;
const kinds: Record<Operation["op"], Outcome["kind"]> = {
    conversation_snapshot: "conversation_snapshot",
    conversation_events: "conversation_events",
    session_observations: "session_observations",
    task: "task",
    contact_pairing_start: "contact_pairing",
    contact_pairing_complete: "contact_pairing",
    contact_pairing_revoke: "contact_pairing",
    contact_state: "contact",
    resolve: "identity",
    capabilities: "capabilities",
    browser_binding: "browser_binding",
    context_deliveries: "context_deliveries",
    membership: "membership",
    membership_snapshot: "membership_snapshot",
    membership_invitations: "membership_invitations",
    create: "conversation",
    conversations: "conversations",
    close: "conversation",
    send: "accepted",
    history: "history",
    inbox: "inbox",
    request_status: "request_status",
    provider_status: "provider_status",
    call_start: "call",
    call_status: "call",
    call_end: "call",
    session_invite: "session",
    session_accept: "session",
    session_reject: "session",
    session_cancel: "session",
    session_end: "session",
    session_get: "session",
    sip_dial: "sip_leg",
    sip_status: "sip_leg",
    sip_end: "sip_leg",
    context_view: "context_view",
    availability_set: "availability",
    availability_get: "availability",
    message_receipts: "message_receipts",
    message_receipt: "receipt_acknowledged",
    receipt_preferences_get: "receipt_preferences",
    receipt_preferences_set: "receipt_preferences",
    transition: "durable_operation",
    durable_operation: "durable_operation",
    artifact_begin: "artifact",
    artifact_append: "artifact",
    artifact_finish: "artifact",
    artifact_get: "artifact_chunk",
    artifact_inspect: "artifact",
    artifact_upload_link: "artifact_link",
    artifact_download_link: "artifact_link",
    artifact_link_get: "artifact_chunk",
    artifact_link_append: "artifact",
    artifact_delete: "artifact",
    health: "health",
};
export function expectedKind(operation: Operation): Outcome["kind"] {
    const kind = kinds[operation.op];
    if (!kind) throw new ConnectError("invalid-request", "Unknown operation");
    return kind;
}
export function timeoutValue(ms: number): number {
    if (!Number.isFinite(ms) || ms <= 0 || ms > 60000)
        throw new RangeError("Timeout must be positive and at most 60000 ms");
    return ms;
}
function readiness(value: unknown): boolean {
    const states = ["ready", "degraded", "unavailable", "uncertain"];
    return (
        fields(value, ["state"]) &&
        states.includes(String(value.state)) &&
        (!("media" in value) || validMediaDiagnostics(value.media)) &&
        array(
            value.components,
            (component) =>
                fields(component, ["component", "state", "reason"]) &&
                states.includes(String(component.state)) &&
                Number.isSafeInteger(component.observed_at) &&
                (!("configured" in component) ||
                    typeof component.configured === "boolean"),
        )
    );
}
function message(value: unknown): boolean {
    return (
        fields(
            value,
            [
                "id",
                "conversation_id",
                "sender",
                "body",
                "content_type",
                "body_encoding",
                "label",
            ],
            ["sender_participant_id", "in_reply_to_msg"],
        ) &&
        ["utf8", "base64"].includes(String(value.body_encoding)) &&
        (value.accepted_at === null || integer(value.accepted_at)) &&
        array(
            value.audience,
            (x) =>
                fields(x, ["participant_id"]) &&
                integer(x.membership_epoch) &&
                Number(x.membership_epoch) > 0,
        ) &&
        array(
            value.attachments,
            (x) => fields(x, ["id", "content_type"]) && integer(x.size_bytes),
        )
    );
}
function session(value: unknown): boolean {
    return (
        fields(
            value,
            [
                "id",
                "conversation_id",
                "initiator_id",
                "participant_id",
                "state",
            ],
            ["ended_reason"],
        ) &&
        array(value.media, (x) => typeof x === "string") &&
        integer(value.created_at) &&
        integer(value.updated_at)
    );
}
function sipLeg(value: unknown): boolean {
    return (
        fields(
            value,
            ["id", "conversation_id", "session_id", "destination", "state"],
            ["connection_id", "ended_reason"],
        ) &&
        typeof value.end_requested === "boolean" &&
        integer(value.created_at) &&
        integer(value.updated_at)
    );
}
function artifact(value: unknown): boolean {
    return (
        fields(value, [
            "id",
            "conversation_id",
            "owner_id",
            "name",
            "media_type",
            "sha256",
            "state",
        ]) &&
        integer(value.size) &&
        integer(value.received) &&
        integer(value.created_at) &&
        (value.expires_at === undefined ||
            value.expires_at === null ||
            integer(value.expires_at))
    );
}
function validData(kind: unknown, data: unknown, depth = 0): boolean {
    if (!object(data) || depth > 8) return false;
    if (
        [
            "conversation_snapshot",
            "conversation_events",
            "session_observations",
        ].includes(String(kind))
    )
        return validEvents(kind, data, validData, depth);
    if (
        [
            "membership",
            "membership_snapshot",
            "membership_invitations",
        ].includes(String(kind))
    )
        return validMembership(String(kind), data);
    switch (kind) {
        case "task":
            return validTaskOutcome(data, validData, depth);
        case "contact":
        case "contact_pairing":
            return validContact(kind, data);
        case "identity":
            return (
                fields(data, ["id", "alias"]) &&
                array(data.capabilities, (x) => typeof x === "string")
            );
        case "conversation":
            return validConversation(data);
        case "conversations":
            return (
                object(data) &&
                Object.keys(data).sort().join(",") ===
                    "conversations,next_cursor" &&
                Array.isArray(data.conversations) &&
                data.conversations.length <= 100 &&
                data.conversations.every(validConversation) &&
                (data.next_cursor === null ||
                    (typeof data.next_cursor === "string" &&
                        data.next_cursor.length <= 2048))
            );
        case "accepted": {
            if (!fields(data, ["request_id", "message_id", "conversation_id"]))
                return false;
            if (!("receipt" in data)) return true; // Task effect result, not a core receipt.
            const r = data.receipt;
            return (
                object(r) &&
                Object.keys(r).sort().join(",") ===
                    "completion_boundary,conversation_id,operation_id,state,status_reference" &&
                r.operation_id === data.request_id &&
                r.conversation_id === data.conversation_id &&
                r.state === "completed" &&
                r.completion_boundary === "local-mutation" &&
                object(r.status_reference) &&
                Object.keys(r.status_reference).sort().join(",") ===
                    "action,operation_id" &&
                r.status_reference.action === "get" &&
                r.status_reference.operation_id === data.request_id
            );
        }
        case "call":
            return (
                fields(
                    data,
                    [
                        "id",
                        "conversation_id",
                        "route_id",
                        "destination",
                        "state",
                        "direction",
                    ],
                    ["provider_id", "ended_reason"],
                ) && typeof data.end_requested === "boolean"
            );
        case "session":
            return session(data);
        case "sip_leg":
            return sipLeg(data);
        case "availability":
            return (
                fields(data, ["identity_id", "state"], ["preferred_route"]) &&
                "expires_at" in data &&
                (data.expires_at === null || integer(data.expires_at)) &&
                "observed_at" in data &&
                (data.observed_at === null || integer(data.observed_at))
            );
        case "browser_binding":
            return fields(
                data,
                ["conversation_id", "participant_id", "signaling_url"],
                ["session_id"],
            );
        case "capabilities":
            return (
                fields(data, ["identity_id"], ["conversation_id"]) &&
                (!("participant_id" in data) ||
                    data.participant_id === null ||
                    typeof data.participant_id === "string") &&
                (!("extensions" in data) ||
                    array(data.extensions, (x) => typeof x === "string")) &&
                array(
                    data.routes,
                    (x) =>
                        fields(x, ["id", "transport", "readiness"]) &&
                        ["configured", "unconfigured"].includes(
                            String(x.readiness),
                        ) &&
                        array(x.media, (v) => typeof v === "string") &&
                        array(x.operations, (v) => typeof v === "string"),
                ) &&
                object(data.limits) &&
                [
                    "message_bytes",
                    "artifact_bytes",
                    "artifact_chunk_bytes",
                    "page_size",
                    "audio_frame_ms",
                ].every((k) =>
                    integer((data.limits as Record<string, unknown>)[k]),
                ) &&
                typeof data.limits.audio_codec === "string" &&
                (data.availability === null ||
                    validData("availability", data.availability, depth + 1))
            );
        case "context_deliveries":
            return (
                fields(data, [], ["next"]) && array(data.deliveries, delivery)
            );
        case "context_view":
            return (
                fields(data, ["conversation_id"]) &&
                integer(data.watermark) &&
                integer(data.omitted) &&
                (data.deliveries_omitted === undefined ||
                    integer(data.deliveries_omitted)) &&
                array(
                    data.items,
                    (x) =>
                        fields(x, ["kind", "source_id", "actor_id", "body"]) &&
                        integer(x.sequence) &&
                        (x.record === undefined || (validContextRecord(x.record) && x.record.uuid === x.source_id)),
                ) &&
                array(
                    data.deliveries,
                    (x) =>
                        fields(
                            x,
                            ["id", "provider", "state"],
                            ["session_id", "provider_ref"],
                        ) &&
                        integer(x.watermark) &&
                        integer(x.created_at) &&
                        integer(x.updated_at),
                )
            );
        case "message_receipts":
            return fields(data, [], ["next"]) && array(data.receipts, receipt);
        case "receipt_preferences":
            return (
                fields(data, ["conversation_id", "participant_id"]) &&
                integer(data.membership_epoch) &&
                Number(data.membership_epoch) > 0 &&
                typeof data.delivery === "boolean" &&
                typeof data.read === "boolean"
            );
        case "receipt_acknowledged":
            return (
                fields(data, [
                    "conversation_id",
                    "message_id",
                    "participant_id",
                ]) &&
                integer(data.membership_epoch) &&
                Number(data.membership_epoch) > 0 &&
                (data.recorded === null ||
                    ["delivered", "read"].includes(data.recorded as string))
            );
        case "durable_operation":
            return (
                fields(
                    data,
                    ["operation_id", "record_id", "command_type"],
                    ["conversation_id", "session_id", "connection_id"],
                ) &&
                integer(data.revision) &&
                integer(data.created_at) &&
                integer(data.updated_at) &&
                typeof data.cancel_requested === "boolean" &&
                (data.expected_revision === null ||
                    integer(data.expected_revision)) &&
                (data.expires_at === null || integer(data.expires_at)) &&
                [
                    "accepted",
                    "preparing",
                    "ready",
                    "committing",
                    "completed",
                    "cancelled",
                    "failed",
                    "interrupted",
                    "unknown",
                ].includes(String(data.state)) &&
                (data.result === null ||
                    (typeof data.result === "object" &&
                        !Array.isArray(data.result))) &&
                array(data.causal_event_ids, (x) => typeof x === "string") &&
                integer(data.events_omitted) &&
                array(
                    data.effects,
                    (x) =>
                        fields(
                            x,
                            ["id", "provider", "correlation_key"],
                            ["compensates_effect_id"],
                        ) &&
                        ["none", "provider-key", "status-query"].includes(
                            String(x.idempotency),
                        ) &&
                        [
                            "prepared",
                            "submitting",
                            "confirmed",
                            "failed",
                            "cancelled",
                            "unknown",
                        ].includes(String(x.state)) &&
                        integer(x.attempt) &&
                        integer(x.submission_generation),
                )
            );
        case "artifact":
            return artifact(data);
        case "artifact_link":
            return (
                fields(
                    data,
                    ["conversation_id", "artifact_id", "path"],
                    ["message_id"],
                ) &&
                integer(data.expires_at) &&
                ["upload", "download"].includes(data.purpose as string) &&
                attachmentLinkPath(data)
            );
        case "artifact_chunk":
            return (
                artifact(data.artifact) &&
                integer(data.offset) &&
                typeof data.data === "string" &&
                typeof data.eof === "boolean"
            );
        case "history":
            return fields(data, [], ["next"]) && array(data.messages, message);
        case "inbox":
            return (
                fields(data, [], ["cursor"]) &&
                array(
                    data.events,
                    (x) =>
                        fields(x, [
                            "id",
                            "kind",
                            "conversation_id",
                            "cursor",
                        ]) &&
                        "message" in x &&
                        (x.message === null || message(x.message)) &&
                        (x.call == null ||
                            validData("call", x.call, depth + 1)) &&
                        (x.session == null || session(x.session)) &&
                        (x.artifact == null || artifact(x.artifact)) &&
                        (x.receipt == null || receipt(x.receipt)),
                )
            );
        case "provider_status":
            return array(data.deliveries, (x) =>
                fields(
                    x,
                    ["job_id", "message_id", "connector", "state"],
                    ["provider_id"],
                ),
            );
        case "request_status":
            return (
                fields(data, ["state"]) &&
                "outcome" in data &&
                (data.outcome === null ||
                    (object(data.outcome) &&
                        validData(
                            data.outcome.kind,
                            data.outcome.data,
                            depth + 1,
                        )))
            );
        case "health":
            return (
                fields(data, ["installation_id", "profile"]) &&
                data.profile === "connect-control/1" &&
                Number.isSafeInteger(data.schema) &&
                Number(data.schema) >= 0 &&
                (!("readiness" in data) || readiness(data.readiness))
            );
        default:
            return false;
    }
}
export function decodeResponse(
    value: unknown,
    expected: Outcome["kind"],
): Outcome {
    if (
        !object(value) ||
        value.profile !== "connect-control/1" ||
        !object(value.result) ||
        Object.keys(value.result).length !== 1
    )
        throw new ConnectError("protocol", "Malformed Connect response");
    if ("Err" in value.result) {
        const error = value.result.Err;
        if (!fields(error, ["code", "message"]))
            throw new ConnectError("protocol", "Malformed Connect error");
        throw new ConnectError(error.code as string, error.message as string);
    }
    const result = value.result.Ok;
    if (
        !object(result) ||
        result.kind !== expected ||
        !validData(result.kind, result.data)
    )
        throw new ConnectError(
            "protocol",
            "Unexpected Connect outcome or invalid fields",
        );
    // Every public field above has been checked; additional fields are permitted.
    return result as unknown as Outcome;
}
export function parseJson(text: string): unknown {
    try {
        return JSON.parse(text) as unknown;
    } catch {
        throw new ConnectError("protocol", "Malformed JSON response");
    }
}
export function helperResponse(
    text: string,
    id: string,
    expected: Outcome["kind"],
): Outcome {
    const value = parseJson(text);
    if (!object(value) || value.id !== id || !("response" in value))
        throw new ConnectError("protocol", "Mismatched helper response");
    return decodeResponse(value.response, expected);
}

function delivery(value: unknown): boolean {
    return (
        fields(
            value,
            ["id", "provider", "state"],
            ["session_id", "provider_ref"],
        ) &&
        integer(value.watermark) &&
        integer(value.created_at) &&
        integer(value.updated_at)
    );
}

function attachmentLinkPath(data: Record<string, unknown>): boolean {
    const prefix = `/v1/conversations/${encodeURIComponent(data.conversation_id as string)}/artifacts/${encodeURIComponent(data.artifact_id as string)}/links/`;
    return (
        typeof data.path === "string" &&
        data.path.startsWith(prefix) &&
        /^[A-Za-z0-9_-]+\.[a-f0-9]{64}$/.test(data.path.slice(prefix.length))
    );
}

function receipt(x: unknown): boolean {
    return (
        fields(x, [
            "conversation_id",
            "message_id",
            "participant_id",
            "delivery_id",
            "provenance",
        ]) &&
        integer(x.membership_epoch) &&
        Number(x.membership_epoch) > 0 &&
        integer(x.observed_at) &&
        ["native", "provider"].includes(x.channel as string) &&
        [
            "queued",
            "accepted",
            "delivered",
            "read",
            "failed",
            "unknown",
        ].includes(x.state as string) &&
        (x.state !== "read" ||
            (x.channel === "native" && x.provenance === "client_reported"))
    );
}
