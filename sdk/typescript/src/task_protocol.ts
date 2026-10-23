import { validContextRecord } from "./record_context.js";
import { array, fields, integer, shape } from "./validation.js";

const taskStates = [
    "ready",
    "running",
    "waiting",
    "completed",
    "cancelled",
    "failed",
    "expired",
];
const effectStates = [
    "prepared",
    "dispatched",
    "succeeded",
    "failed",
    "uncertain",
    "cancelled",
];
const permissions = [
    "resolve",
    "capabilities",
    "send",
    "history",
    "context_view",
    "record_get",
    "context_deliveries",
    "inbox",
    "request_status",
    "artifact_begin",
    "artifact_append",
    "artifact_finish",
    "artifact_get",
    "artifact_delete",
    "session_invite",
    "session_accept",
    "session_reject",
    "session_cancel",
    "session_end",
    "session_get",
    "call_start",
    "call_status",
    "call_end",
    "sip_dial",
    "sip_status",
    "sip_end",
    "task_cancel",
];
type ControlValidator = (
    kind: unknown,
    data: unknown,
    depth: number,
) => boolean;
function scope(value: unknown): boolean {
    return (
        shape(value, [], [], ["operations", "recipients", "sms_enrollments"]) &&
        array(
            value.operations,
            (x) => typeof x === "string" && permissions.includes(x),
        ) &&
        array(value.recipients, (x) => typeof x === "string") &&
        (!("sms_enrollments" in value) ||
            array(value.sms_enrollments, (x) =>
                shape(x, ["source", "grant_id", "purpose"]),
            ))
    );
}
function record(value: unknown): boolean {
    return (
        shape(
            value,
            ["id", "owner_id", "conversation_id", "state"],
            ["initiating_event_id", "reason"],
            ["scope", "revision", "expires_at", "created_at", "updated_at"],
        ) &&
        scope(value.scope) &&
        taskStates.includes(String(value.state)) &&
        ["revision", "expires_at", "created_at", "updated_at"].every((k) =>
            integer(value[k]),
        )
    );
}
function reply(value: unknown): boolean {
    return (
        shape(
            value,
            ["event_id", "conversation_id"],
            ["task_id", "reply_to", "assigned_by"],
            ["consumed"],
        ) && typeof value.consumed === "boolean"
    );
}
function claim(value: unknown): boolean {
    return (
        shape(
            value,
            ["task_id", "step_id", "worker_id"],
            [],
            ["revision", "expires_at"],
        ) &&
        integer(value.revision) &&
        integer(value.expires_at)
    );
}
function outcome(
    value: unknown,
    validate: ControlValidator,
    depth: number,
): boolean {
    return (
        fields(value, ["kind"]) &&
        "data" in value &&
        validate(value.kind, value.data, depth + 1)
    );
}
function effect(
    value: unknown,
    validate: ControlValidator,
    depth: number,
): boolean {
    return (
        shape(
            value,
            ["id", "task_id", "step_id", "request_id", "state"],
            [],
            ["outcome", "error"],
        ) &&
        effectStates.includes(String(value.state)) &&
        "outcome" in value &&
        (value.outcome === null || outcome(value.outcome, validate, depth)) &&
        (!("error" in value) ||
            value.error === null ||
            fields(value.error, ["code", "message"]))
    );
}
function event(
    value: unknown,
    validate: ControlValidator,
    depth: number,
): boolean {
    if (
        !shape(
            value,
            ["id", "task_id", "actor_id"],
            [],
            ["sequence", "created_at", "data"],
        ) ||
        !integer(value.sequence) ||
        !integer(value.created_at) ||
        !shape(value.data, ["kind"], [], ["data"]) ||
        !("data" in value.data)
    )
        return false;
    const data = value.data.data;
    switch (value.data.kind) {
        case "state":
            return (
                shape(data, ["state"], ["reason"], ["revision"]) &&
                taskStates.includes(String(data.state)) &&
                integer(data.revision)
            );
        case "reply":
            return reply(data);
        case "effect":
            return effect(data, validate, depth);
        case "claim":
            return claim(data);
        default:
            return false;
    }
}
function source(value: unknown): boolean {
    return (
        shape(
            value,
            ["source_id", "kind", "actor_id", "provenance", "state"],
            ["body"],
            ["created_at", "record"],
        ) &&
        (value.record === undefined || (validContextRecord(value.record) && value.record.uuid === value.source_id)) &&
        "created_at" in value &&
        (value.created_at === null || integer(value.created_at)) &&
        ["available", "deleted", "expired", "unavailable"].includes(
            String(value.state),
        )
    );
}
/** Validate nested results without confusing pending/uncertain with completion. */
export function validTaskOutcome(
    value: unknown,
    validate: ControlValidator,
    depth: number,
): boolean {
    if (
        depth > 8 ||
        !shape(value, ["kind"], [], ["data"]) ||
        !("data" in value)
    )
        return false;
    const data = value.data;
    switch (value.kind) {
        case "record":
            return record(data);
        case "replies":
            return (
                shape(data, [], ["next"], ["items"]) && array(data.items, reply)
            );
        case "inbox":
            return (
                shape(data, [], ["next"], ["events"]) &&
                array(data.events, (x) => event(x, validate, depth))
            );
        case "recall":
            return (
                shape(data, [], ["next"], ["sources", "omitted"]) &&
                integer(data.omitted) &&
                array(data.sources, source)
            );
        case "effect":
            return effect(data, validate, depth);
        case "claim":
            return data === null || claim(data);
        case "reply_accepted":
            return (
                shape(data, ["request_id", "message_id"], [], ["reply"]) &&
                reply(data.reply)
            );
        case "tool":
            if (!fields(data, ["status", "task_id"])) return false;
            if (data.status === "completed")
                return (
                    shape(data, ["status", "task_id"], [], ["outcome"]) &&
                    outcome(data.outcome, validate, depth)
                );
            if (data.status === "pending")
                return shape(data, ["status", "task_id"], ["effect_id"]);
            return (
                data.status === "uncertain" &&
                shape(data, ["status", "task_id", "effect_id"])
            );
        default:
            return false;
    }
}
