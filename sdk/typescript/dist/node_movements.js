import { sdkRevision } from "./node_sdk.js";
import { NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
const natural = (v) => Number.isSafeInteger(v) && Number(v) >= 0;
const positive = (v) => natural(v) && v > 0;
function invalid() {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol() {
    throw new Error("Invalid typed movement result");
}
const roleValid = (v) => object(v, ["scope", "expected_conversation_revision"]) &&
    (v.scope === "session"
        ? Object.keys(v).length === 1
        : v.scope === "conversation" &&
            Object.keys(v).length === 2 &&
            natural(v.expected_conversation_revision) &&
            v.expected_conversation_revision < Number.MAX_SAFE_INTEGER);
export function handoffInput(v) {
    if (!v ||
        !identifier(v.via) ||
        !identifier(v.retain) ||
        v.via === v.retain ||
        !identifier(v.from) ||
        !identifier(v.to) ||
        v.from === v.to ||
        !["leave", "observe"].includes(v.outgoing) ||
        !v.role)
        invalid();
    let role;
    if (v.role.scope === "session" &&
        object(v.role, ["scope"]) &&
        Object.keys(v.role).length === 1)
        role = { scope: "session" };
    else if (v.role.scope === "conversation" &&
        object(v.role, ["scope", "expectedConversationRevision"]) &&
        Object.keys(v.role).length === 2 &&
        natural(v.role.expectedConversationRevision) &&
        v.role.expectedConversationRevision < Number.MAX_SAFE_INTEGER)
        role = {
            scope: "conversation",
            expected_conversation_revision: v.role.expectedConversationRevision,
        };
    else
        return invalid();
    if (v.candidate !== undefined &&
        (!identifier(v.candidate) ||
            v.candidate === v.via ||
            v.candidate === v.retain))
        invalid();
    return {
        via: v.via,
        retain: v.retain,
        from_participant: v.from,
        to: v.to,
        outgoing: v.outgoing,
        role,
        candidate: v.candidate ?? null,
        expected_revision: sdkRevision(v.expectedTopologyRevision),
    };
}
export function moveInput(v, via) {
    if (!v ||
        !identifier(v.retain) ||
        v.retain === via ||
        !object(v.target, ["endpoint", "candidate"]) ||
        Object.keys(v.target).length !== 1)
        invalid();
    const endpoint = "endpoint" in v.target, id = endpoint ? v.target.endpoint : v.target.candidate;
    if (!identifier(id) || (!endpoint && (id === via || id === v.retain)))
        invalid();
    return {
        retain: v.retain,
        target: { kind: endpoint ? "endpoint" : "candidate", id },
        expected_revision: sdkRevision(v.expectedTopologyRevision),
    };
}
export function movementContext(v, command, via) {
    if (!identifier(via) ||
        !object(v, command === "session.handoff"
            ? [
                "kind",
                "retained_connection_id",
                "source_participant",
                "target_participant",
                "candidate",
                "role",
                "outgoing",
            ]
            : ["kind", "retained_connection_id", "target"]) ||
        !identifier(v.retained_connection_id) ||
        v.retained_connection_id === via)
        return false;
    if (command === "session.handoff")
        return (v.kind === "handoff" &&
            identifier(v.source_participant) &&
            identifier(v.target_participant) &&
            v.source_participant !== v.target_participant &&
            (v.candidate == null ||
                (identifier(v.candidate) &&
                    v.candidate !== via &&
                    v.candidate !== v.retained_connection_id)) &&
            roleValid(v.role) &&
            (v.outgoing === "leave" || v.outgoing === "observe"));
    return (command === "connection.replace" &&
        v.kind === "replacement" &&
        object(v.target, ["kind", "id"]) &&
        Object.keys(v.target).length === 2 &&
        identifier(v.target.id) &&
        (v.target.kind === "endpoint" ||
            (v.target.kind === "candidate" &&
                v.target.id !== via &&
                v.target.id !== v.retained_connection_id)));
}
function original(saved) {
    if (!object(saved, [
        "schema",
        "operation_id",
        "command",
        "scope",
        "continuation",
    ]) ||
        !object(saved.continuation, [
            "kind",
            "retained_connection_id",
            "source_participant",
            "target_participant",
            "candidate",
            "role",
            "outgoing",
            "target",
        ]))
        return protocol();
    return saved.continuation;
}
function topology(v, ctx) {
    if (!Array.isArray(v.retained) ||
        !v.retained.length ||
        !Array.isArray(v.admitted) ||
        v.admitted.length !== 1 ||
        !Array.isArray(v.retired) ||
        v.retained.length + v.admitted.length + v.retired.length > 100 ||
        !positive(v.topologyRevision) ||
        (v.conversationRevision !== null && !positive(v.conversationRevision)))
        return protocol();
    const seen = new Set();
    const id = (value) => {
        if (!identifier(value) || seen.has(value))
            return protocol();
        seen.add(value);
        return value;
    };
    const connection = (value) => {
        if (!object(value, [
            "id",
            "conversationId",
            "sessionId",
            "receiptStreamRevision",
        ]) ||
            value.conversationId !== ctx.key.scope.conversationId ||
            value.sessionId !== ctx.key.scope.sessionId ||
            value.receiptStreamRevision !== null)
            return protocol();
        return ctx.connection(id(value.id));
    };
    const retained = Object.freeze(v.retained.map(connection)), admitted = Object.freeze(v.admitted.map(connection)), retired = Object.freeze(v.retired.map(id));
    return Object.freeze({
        retained,
        admitted,
        retired,
        topologyRevision: sdkRevision(v.topologyRevision),
        ...(v.conversationRevision === null
            ? {}
            : {
                conversationRevision: sdkRevision(v.conversationRevision),
            }),
    });
}
export function handoffResult(value, ctx, saved) {
    if (!object(value, [
        "retained",
        "admitted",
        "retired",
        "topologyRevision",
        "conversationRevision",
        "roleChange",
    ]))
        return protocol();
    const c = original(saved), t = topology(value, ctx), role = c.role;
    if (!t.retained.some((v) => v.id === c.retained_connection_id) ||
        (c.candidate != null && t.admitted[0].id !== c.candidate) ||
        (c.outgoing === "leave") !==
            t.retired.includes(ctx.key.scope.connectionId) ||
        (role.scope === "session"
            ? t.conversationRevision !== undefined
            : t.conversationRevision !==
                Number(role.expected_conversation_revision) + 1))
        return protocol();
    let roleChange;
    if (value.roleChange !== null) {
        const r = value.roleChange;
        if (!object(r, [
            "participantRevision",
            "roleScope",
            "outgoing",
            "sourceParticipant",
            "targetParticipant",
            "cleanup",
        ]) ||
            !positive(r.participantRevision) ||
            r.roleScope !== role.scope ||
            r.outgoing !== c.outgoing ||
            r.sourceParticipant !== c.source_participant ||
            r.targetParticipant !== c.target_participant ||
            r.cleanup !== (c.outgoing === "leave" ? "pending" : "not_requested"))
            return protocol();
        roleChange = Object.freeze({ ...r });
    }
    return Object.freeze({
        ...t,
        ...(roleChange === undefined ? {} : { roleChange }),
    });
}
export function moveResult(value, ctx, saved) {
    if (!object(value, [
        "retained",
        "admitted",
        "retired",
        "topologyRevision",
        "conversationRevision",
        "connection",
        "binding",
    ]))
        return protocol();
    const c = original(saved), target = c.target, t = topology(value, ctx), binding = value.binding, connection = value.connection;
    if (t.conversationRevision !== undefined ||
        !t.retained.some((v) => v.id === c.retained_connection_id) ||
        !t.retired.includes(ctx.key.scope.connectionId) ||
        !object(connection, [
            "id",
            "conversationId",
            "sessionId",
            "receiptStreamRevision",
        ]) ||
        connection.id !== t.admitted[0].id ||
        connection.conversationId !== ctx.key.scope.conversationId ||
        connection.sessionId !== ctx.key.scope.sessionId ||
        connection.receiptStreamRevision !== null ||
        !object(binding, [
            "generation",
            "participantId",
            "membershipEpoch",
            "endpointId",
            "deviceId",
        ]) ||
        !positive(binding.generation) ||
        !positive(binding.membershipEpoch) ||
        !identifier(binding.participantId) ||
        !identifier(binding.endpointId) ||
        !identifier(binding.deviceId) ||
        (target.kind === "endpoint"
            ? binding.endpointId !== target.id
            : connection.id !== target.id))
        return protocol();
    return Object.freeze({
        ...t,
        connection: t.admitted[0],
        binding: Object.freeze({
            ...binding,
        }),
    });
}
