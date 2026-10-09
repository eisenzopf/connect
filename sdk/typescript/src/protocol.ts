import { ConnectError } from "./errors.js";
import type { Operation, Outcome } from "./types.js";
export const MAX_REQUEST = 65537;
export const MAX_RESPONSE = 600000;
const kinds: Record<Operation["op"], Outcome["kind"]> = {
    resolve: "identity",
    create: "conversation",
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
    artifact_begin: "artifact",
    artifact_append: "artifact",
    artifact_finish: "artifact",
    artifact_get: "artifact_chunk",
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
function object(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(
    value: unknown,
    strings: string[] = [],
    nullable: string[] = [],
): value is Record<string, unknown> {
    return (
        object(value) &&
        strings.every((k) => typeof value[k] === "string") &&
        nullable.every(
            (k) =>
                k in value &&
                (value[k] === null || typeof value[k] === "string"),
        )
    );
}
function array(value: unknown, test: (v: unknown) => boolean): boolean {
    return Array.isArray(value) && value.every(test);
}
function message(value: unknown): boolean {
    return fields(value, ["id", "conversation_id", "sender", "body"]);
}
function integer(value: unknown): boolean {
    return Number.isSafeInteger(value) && Number(value) >= 0;
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
    switch (kind) {
        case "identity":
            return (
                fields(data, ["id", "alias"]) &&
                array(data.capabilities, (x) => typeof x === "string")
            );
        case "conversation":
            return (
                fields(data, ["id"]) &&
                typeof data.closed === "boolean" &&
                array(data.participants, (x) => typeof x === "string")
            );
        case "accepted":
            return fields(data, [
                "request_id",
                "message_id",
                "conversation_id",
            ]);
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
        case "context_view":
            return (
                fields(data, ["conversation_id"]) &&
                integer(data.watermark) &&
                integer(data.omitted) &&
                array(
                    data.items,
                    (x) =>
                        fields(x, ["kind", "source_id", "actor_id", "body"]) &&
                        integer(x.sequence),
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
        case "artifact":
            return artifact(data);
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
                        (x.artifact == null || artifact(x.artifact)),
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
                Number(data.schema) >= 0
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
