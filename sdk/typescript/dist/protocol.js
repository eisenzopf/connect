import { ConnectError } from "./errors.js";
export const MAX_REQUEST = 65537;
export const MAX_RESPONSE = 600000;
const kinds = {
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
    health: "health",
};
export function expectedKind(operation) {
    const kind = kinds[operation.op];
    if (!kind)
        throw new ConnectError("invalid-request", "Unknown operation");
    return kind;
}
export function timeoutValue(ms) {
    if (!Number.isFinite(ms) || ms <= 0 || ms > 60000)
        throw new RangeError("Timeout must be positive and at most 60000 ms");
    return ms;
}
function object(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(value, strings = [], nullable = []) {
    return (object(value) &&
        strings.every((k) => typeof value[k] === "string") &&
        nullable.every((k) => k in value &&
            (value[k] === null || typeof value[k] === "string")));
}
function array(value, test) {
    return Array.isArray(value) && value.every(test);
}
function message(value) {
    return fields(value, ["id", "conversation_id", "sender", "body"]);
}
function validData(kind, data, depth = 0) {
    if (!object(data) || depth > 8)
        return false;
    switch (kind) {
        case "identity":
            return (fields(data, ["id", "alias"]) &&
                array(data.capabilities, (x) => typeof x === "string"));
        case "conversation":
            return (fields(data, ["id"]) &&
                typeof data.closed === "boolean" &&
                array(data.participants, (x) => typeof x === "string"));
        case "accepted":
            return fields(data, [
                "request_id",
                "message_id",
                "conversation_id",
            ]);
        case "call":
            return (fields(data, [
                "id",
                "conversation_id",
                "route_id",
                "destination",
                "state",
                "direction",
            ], ["provider_id", "ended_reason"]) && typeof data.end_requested === "boolean");
        case "history":
            return fields(data, [], ["next"]) && array(data.messages, message);
        case "inbox":
            return (fields(data, [], ["cursor"]) &&
                array(data.events, (x) => fields(x, [
                    "id",
                    "kind",
                    "conversation_id",
                    "cursor",
                ]) &&
                    "message" in x &&
                    (x.message === null || message(x.message)) &&
                    (x.call == null ||
                        validData("call", x.call, depth + 1))));
        case "provider_status":
            return array(data.deliveries, (x) => fields(x, ["job_id", "message_id", "connector", "state"], ["provider_id"]));
        case "request_status":
            return (fields(data, ["state"]) &&
                "outcome" in data &&
                (data.outcome === null ||
                    (object(data.outcome) &&
                        validData(data.outcome.kind, data.outcome.data, depth + 1))));
        case "health":
            return (fields(data, ["installation_id", "profile"]) &&
                data.profile === "connect-control/1" &&
                Number.isSafeInteger(data.schema) &&
                Number(data.schema) >= 0);
        default:
            return false;
    }
}
export function decodeResponse(value, expected) {
    if (!object(value) ||
        value.profile !== "connect-control/1" ||
        !object(value.result) ||
        Object.keys(value.result).length !== 1)
        throw new ConnectError("protocol", "Malformed Connect response");
    if ("Err" in value.result) {
        const error = value.result.Err;
        if (!fields(error, ["code", "message"]))
            throw new ConnectError("protocol", "Malformed Connect error");
        throw new ConnectError(error.code, error.message);
    }
    const result = value.result.Ok;
    if (!object(result) ||
        result.kind !== expected ||
        !validData(result.kind, result.data))
        throw new ConnectError("protocol", "Unexpected Connect outcome or invalid fields");
    // Every public field above has been checked; additional fields are permitted.
    return result;
}
export function parseJson(text) {
    try {
        return JSON.parse(text);
    }
    catch {
        throw new ConnectError("protocol", "Malformed JSON response");
    }
}
export function helperResponse(text, id, expected) {
    const value = parseJson(text);
    if (!object(value) || value.id !== id || !("response" in value))
        throw new ConnectError("protocol", "Mismatched helper response");
    return decodeResponse(value.response, expected);
}
