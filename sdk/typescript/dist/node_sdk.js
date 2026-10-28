import { createEvents, } from "./node_events.js";
import { createIncoming, } from "./node_incoming.js";
import { createRaw } from "./node_raw.js";
import { deliveryObservations, } from "./node_receipts.js";
import { handoffInput, moveInput, movementContext, handoffResult, moveResult, } from "./node_movements.js";
import { createConnections, connectionHandle, connectionUpdate, streamSelection, } from "./node_connections.js";
import { createSessions, sessionSnapshot, } from "./node_sessions.js";
import { canonicalJson, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import { SdkHelper, NodeSdkError, object } from "./sdk_helper.js";
export function sdkRevision(value) {
    if (!Number.isSafeInteger(value) || value < 0)
        invalid();
    return value;
}
function capabilitySnapshot(value) {
    if (!object(value, [
        "profile",
        "runtime_role",
        "selected_transport",
        "fallback_reason",
        "features",
    ]) ||
        value.profile !== "connect/2" ||
        value.runtime_role !== "rust" ||
        value.selected_transport !== "quic" ||
        value.fallback_reason !== null ||
        !Array.isArray(value.features) ||
        value.features.length > 128 ||
        value.features.some((f) => typeof f !== "string" || !/^[a-z0-9-]{1,128}$/.test(f)) ||
        new Set(value.features).size !== value.features.length)
        return protocol();
    // Advertise only exposed Node interfaces that this Rust configuration supports.
    const set = new Set(value.features.filter((f) => f === "original-id-operations" ||
        f === "message-receipts" ||
        f === "incoming-invitations" ||
        f === "durable-events" ||
        f === "raw-canonical"));
    const features = Object.freeze({
        get size() {
            return set.size;
        },
        has: (feature) => set.has(feature),
        entries: () => set.entries(),
        keys: () => set.keys(),
        values: () => set.values(),
        [Symbol.iterator]: () => set.values(),
        forEach: (callback, thisArg) => set.forEach((v) => callback.call(thisArg, v, v, features)),
    });
    return Object.freeze({
        profile: "connect/2",
        runtimeRole: "node",
        selectedTransport: "quic",
        features,
    });
}
const runOperation = Symbol("operation");
const sendMessage = Symbol("send");
const refreshConversation = Symbol("refresh");
const readReceipts = Symbol("receipts");
const mutateParticipant = Symbol("participant-mutation");
const readParticipant = Symbol("participant-read");
const mutateSession = Symbol("session-mutation");
const readSession = Symbol("session-read");
const connectionContext = Symbol("connection-context");
const returnConsultation = Symbol("consultation-return");
const handoffSession = Symbol("session-handoff");
const moveConnection = Symbol("connection-move");
const submitMovement = Symbol("movement-submit");
const kinds = new WeakMap();
const references = new WeakMap();
const states = new Set([
    "accepted",
    "preparing",
    "ready",
    "committing",
    "completed",
    "cancelled",
    "failed",
    "interrupted",
    "unknown",
]);
function invalid() {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol() {
    throw new Error("Invalid typed SDK helper result");
}
export function sdkId(value) {
    if (!identifier(value))
        invalid();
    return value;
}
function freeze(value) {
    if (value && typeof value === "object") {
        for (const child of Object.values(value))
            freeze(child);
        Object.freeze(value);
    }
    return value;
}
const copied = (value) => parseCanonicalJson(Buffer.from(canonicalJson(value)));
function mutation(options) {
    if (!options || !identifier(options.operationId))
        invalid();
    return {
        operation_id: options.operationId,
        ...(options.extensions === undefined
            ? {}
            : { extensions: options.extensions }),
    };
}
function makeKey(operationId, command, scope) {
    return freeze({
        operationId: operationId,
        command,
        scope: { ...scope },
    });
}
function readKey(value) {
    if (!object(value, ["operationId", "command", "scope"]) ||
        !identifier(value.operationId) ||
        typeof value.command !== "string" ||
        !object(value.scope, ["conversationId", "sessionId", "connectionId"]))
        return protocol();
    const scope = {};
    for (const k of ["conversationId", "sessionId", "connectionId"]) {
        const id = value.scope[k];
        if (id === null || id === undefined)
            continue;
        if (!identifier(id))
            return protocol();
        scope[k] = id;
    }
    if ((scope.sessionId && !scope.conversationId) ||
        (scope.connectionId && !scope.sessionId))
        return protocol();
    return makeKey(value.operationId, value.command, scope);
}
function equalKey(a, b) {
    return (a.operationId === b.operationId &&
        a.command === b.command &&
        ["conversationId", "sessionId", "connectionId"].every((k) => a.scope[k] ===
            b.scope[k]));
}
function snapshot(value, key) {
    if (!object(value, [
        "key",
        "state",
        "revision",
        "recordId",
        "cancelRequested",
        "expiresAt",
        "problem",
    ]) ||
        !equalKey(readKey(value.key), key) ||
        typeof value.state !== "string" ||
        !states.has(value.state) ||
        !Number.isSafeInteger(value.revision) ||
        Number(value.revision) < 0 ||
        (value.recordId !== null &&
            value.recordId !== undefined &&
            !identifier(value.recordId)) ||
        (value.cancelRequested !== null &&
            value.cancelRequested !== undefined &&
            typeof value.cancelRequested !== "boolean") ||
        (value.expiresAt !== null &&
            value.expiresAt !== undefined &&
            !Number.isSafeInteger(value.expiresAt)))
        return protocol();
    return freeze({
        key,
        state: value.state,
        revision: value.revision,
        ...(value.recordId == null
            ? {}
            : { recordId: value.recordId }),
        ...(value.cancelRequested == null
            ? {}
            : { cancelRequested: value.cancelRequested }),
        ...(value.expiresAt == null
            ? {}
            : { expiresAt: value.expiresAt }),
        ...(value.problem == null ? {} : { problem: value.problem }),
    });
}
function kind(name, command, decode) {
    const value = Object.freeze({ command });
    kinds.set(value, { name, command, decode });
    return value;
}
function decision(value, _client, key) {
    if (!object(value, [
        "decision",
        "target",
        "observedState",
        "disposition",
        "evidenceId",
    ]) ||
        !equalKey(readKey(value.decision), key) ||
        !identifier(value.target) ||
        value.target === key.operationId ||
        typeof value.observedState !== "string" ||
        !states.has(value.observedState) ||
        typeof value.disposition !== "string" ||
        !value.disposition ||
        value.disposition.length > 256 ||
        /[\x00-\x1f\x7f]/.test(value.disposition) ||
        (key.command === "operation.cancel"
            ? value.evidenceId !== null
            : !identifier(value.evidenceId)))
        return protocol();
    return freeze({
        decision: key,
        target: value.target,
        observedState: value.observedState,
        disposition: value.disposition,
        ...(value.evidenceId == null
            ? {}
            : { evidenceId: value.evidenceId }),
    });
}
function participant(value, client, key) {
    if (!object(value, ["id", "conversationId"]) ||
        !identifier(value.id) ||
        value.conversationId !== key.scope.conversationId)
        return protocol();
    return client.conversations
        .byId(value.conversationId)
        .participants.byId(value.id);
}
function participantSnapshot(value, cid, pid) {
    if (!object(value, [
        "id",
        "conversationId",
        "identityId",
        "kind",
        "role",
        "membershipEpoch",
        "active",
        "removalReason",
    ]) ||
        value.id !== pid ||
        value.conversationId !== cid ||
        !identifier(value.identityId) ||
        typeof value.kind !== "string" ||
        !["human", "ai", "system", "external"].includes(value.kind) ||
        typeof value.role !== "string" ||
        !Number.isSafeInteger(value.membershipEpoch) ||
        Number(value.membershipEpoch) < 0 ||
        typeof value.active !== "boolean" ||
        (value.removalReason !== null &&
            typeof value.removalReason !== "string"))
        return protocol();
    return freeze(value);
}
function session(value, client, key) {
    if (!object(value, ["id", "conversationId"]) ||
        value.id !== key.scope.sessionId ||
        value.conversationId !== key.scope.conversationId)
        return protocol();
    return client.conversations
        .byId(value.conversationId)
        .sessions.byId(value.id);
}
function consultation(value, client, key, saved) {
    if (!object(value, [
        "session",
        "retained",
        "admitted",
        "retired",
        "topologyRevision",
        "conversationRevision",
    ]) ||
        !Array.isArray(value.retained) ||
        !value.retained.length ||
        !Array.isArray(value.admitted) ||
        !value.admitted.length ||
        !Array.isArray(value.retired) ||
        value.retired.length !== 0 ||
        value.conversationRevision !== null ||
        !Number.isSafeInteger(value.topologyRevision) ||
        Number(value.topologyRevision) < 1)
        return protocol();
    const owner = session(value.session, client, key), seen = new Set();
    const decode = (v) => {
        if (!object(v, [
            "id",
            "conversationId",
            "sessionId",
            "receiptStreamRevision",
        ]) ||
            !identifier(v.id) ||
            v.conversationId !== owner.conversationId ||
            v.sessionId !== owner.id ||
            v.receiptStreamRevision !== null ||
            seen.has(v.id))
            return protocol();
        seen.add(v.id);
        return owner.connections.byId(v.id);
    };
    if (value.retained.length + value.admitted.length > 100)
        return protocol();
    const retained = value.retained.map(decode), admitted = value.admitted.map(decode);
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
            "record_id",
        ]) ||
        !retained.some((c) => c.id ===
            saved.continuation.retained_connection_id))
        return protocol();
    return Object.freeze({
        session: owner,
        retained: Object.freeze(retained),
        admitted: Object.freeze(admitted),
        retired: Object.freeze([]),
        topologyRevision: sdkRevision(value.topologyRevision),
    });
}
function validConsultationContext(value, command, via) {
    const returning = command === "session.consult.return";
    if (!object(value, returning
        ? ["kind", "retained_connection_id", "consultation_record_id"]
        : ["kind", "retained_connection_id", "record_id"]) ||
        value.kind !== (returning ? "consultation-return" : "consultation") ||
        !identifier(value.retained_connection_id) ||
        value.retained_connection_id === via)
        return false;
    return returning
        ? identifier(value.consultation_record_id)
        : value.record_id == null || identifier(value.record_id);
}
function sessionDecision(value, client, key) {
    if (!object(value, ["session", "state", "topologyRevision"]) ||
        value.state !==
            (key.command === "session.reject" ? "rejected" : "cancelled") ||
        !Number.isSafeInteger(value.topologyRevision) ||
        Number(value.topologyRevision) < 0)
        return protocol();
    return Object.freeze({
        session: session(value.session, client, key),
        state: value.state,
        topologyRevision: value.topologyRevision,
    });
}
function connection(value, client, key) {
    return connectionHandle(value, client[connectionContext](key.scope.conversationId, key.scope.sessionId), key.scope.connectionId);
}
function connectionReceipt(value, client, key) {
    return connectionUpdate(value, client[connectionContext](key.scope.conversationId, key.scope.sessionId), key.scope.connectionId);
}
function selectionReceipt(value, client, key) {
    return streamSelection(value, client[connectionContext](key.scope.conversationId, key.scope.sessionId), key.scope.connectionId);
}
const operationKinds = Object.freeze({
    session_handoff: kind("session_handoff", "session.handoff", (value, client, key, saved) => handoffResult(value, {
        key,
        connection: (id) => client.conversations
            .byId(key.scope.conversationId)
            .sessions.byId(key.scope.sessionId)
            .connections.byId(id),
    }, saved)),
    connection_replace: kind("connection_replace", "connection.replace", (value, client, key, saved) => moveResult(value, {
        key,
        connection: (id) => client.conversations
            .byId(key.scope.conversationId)
            .sessions.byId(key.scope.sessionId)
            .connections.byId(id),
    }, saved)),
    session_consult: kind("session_consult", "session.consult", consultation),
    session_consult_return: kind("session_consult_return", "session.consult.return", session),
    connection_offer: kind("connection_offer", "connection.offer", connection),
    connection_update: kind("connection_update", "connection.update", connectionReceipt),
    connection_end: kind("connection_end", "connection.end", connection),
    stream_subscribe: kind("stream_subscribe", "stream.subscribe", selectionReceipt),
    stream_unsubscribe: kind("stream_unsubscribe", "stream.unsubscribe", selectionReceipt),
    session_invite: kind("session_invite", "session.invite", session),
    session_accept: kind("session_accept", "session.accept", session),
    session_update: kind("session_update", "session.update", session),
    session_end: kind("session_end", "session.end", session),
    session_reject: kind("session_reject", "session.reject", sessionDecision),
    session_cancel: kind("session_cancel", "session.cancel", sessionDecision),
    conversation_participant_add: kind("conversation_participant_add", "conversation.participant.add", participant),
    conversation_participant_update: kind("conversation_participant_update", "conversation.participant.update", participant),
    conversation_participant_remove: kind("conversation_participant_remove", "conversation.participant.remove", participant),
    conversation_create: kind("conversation_create", "conversation.create", (value, client) => {
        if (!object(value, ["id"]) || !identifier(value.id))
            return protocol();
        return client.conversations.byId(value.id);
    }),
    message_send: kind("message_send", "message.send", (value, _client, key) => {
        if (!object(value, ["messageId", "operation", "boundary"]) ||
            !identifier(value.messageId) ||
            value.boundary !== "local-mutation" ||
            !equalKey(readKey(value.operation), key))
            return protocol();
        return freeze({
            messageId: value.messageId,
            operation: key,
            boundary: "local-mutation",
        });
    }),
    operation_cancel: kind("operation_cancel", "operation.cancel", decision),
    operation_reconcile: kind("operation_reconcile", "operation.reconcile", decision),
});
/** Typed Conversation/message/decision slice of G6-062. Other control kinds,
 * remote HTTP remains under development. Journal facts and incoming hints grant no custody.
 * One fixed SDK configuration, one Rust owner, no transparent restart/retry. */
const eventStream = Symbol("eventStream");
export class NodeClient {
    role = "node";
    operationKinds = operationKinds;
    capabilities;
    raw;
    conversations;
    operations;
    incomingInvitations(options) {
        return createIncoming(this.helper, (cid, sid) => this.conversations.byId(cid).sessions.byId(sid), options);
    }
    [eventStream](cid, input, options) {
        return createEvents(this.helper, cid, input, options);
    }
    helper;
    constructor(config) {
        const captured = {
            config: config?.config,
            executable: config?.executable ?? "connect",
        };
        this.helper = new SdkHelper(captured);
        this.raw = createRaw(this.helper, captured.executable);
        const refresh = (options) => this.helper.request({ type: "capabilities_refresh" }, options, capabilitySnapshot);
        this.capabilities = Object.freeze({
            refresh,
            require: async (feature, options) => {
                if (typeof feature !== "string" ||
                    !/^[a-z0-9-]{1,128}$/.test(feature))
                    throw new NodeSdkError("validation", "read-only");
                if (!(await refresh(options)).features.has(feature))
                    throw new NodeSdkError("unsupported", "read-only");
            },
        });
        this.conversations = Object.freeze({
            byId: (id) => {
                sdkId(id);
                return new SdkConversation(this, id);
            },
            create: async (input, options) => {
                if (!input ||
                    !Array.isArray(input.participants) ||
                    input.participants.some((id) => !identifier(id)) ||
                    !["persistent", "ephemeral"].includes(input.policy))
                    invalid();
                const key = makeKey(options?.operationId, "conversation.create", {});
                return this.helper.request({
                    type: "conversation_create",
                    participants: [...input.participants],
                    policy: input.policy,
                    mutation: mutation(options),
                }, options, (value) => this.fromReply(value, this.operationKinds.conversation_create, key), key, true);
            },
        });
        this.operations = Object.freeze({
            restore: (saved, expected) => this.restore(saved, expected),
            recover: (reference) => {
                const saved = references.get(reference);
                if (!saved)
                    invalid();
                return this.restore(saved.saved, saved.kind);
            },
        });
        Object.freeze(this);
    }
    restore(saved, expected) {
        const k = kinds.get(expected);
        if (!k || k.name === "administrative")
            invalid();
        let copy;
        try {
            const text = canonicalJson(saved);
            if (Buffer.byteLength(text) > 4096)
                invalid();
            copy = copied(saved);
        }
        catch {
            return invalid();
        }
        if (!object(copy, [
            "schema",
            "operation_id",
            "command",
            "scope",
            "continuation",
        ]) ||
            copy.schema !== "connect-operation-reference/1" ||
            !identifier(copy.operation_id) ||
            copy.command !== k.command ||
            !object(copy.scope, [
                "conversation_id",
                "session_id",
                "connection_id",
            ]))
            invalid();
        const movement = k.name === "session_handoff" || k.name === "connection_replace";
        const transition = movement ||
            k.name === "session_consult" ||
            k.name === "session_consult_return";
        if (movement
            ? !movementContext(copy.continuation, k.command, copy.scope.connection_id)
            : transition
                ? !validConsultationContext(copy.continuation, k.command, copy.scope.connection_id)
                : "continuation" in copy)
            invalid();
        const s = copy.scope;
        for (const id of Object.values(s))
            if (!identifier(id))
                invalid();
        if ((s.session_id && !s.conversation_id) ||
            (s.connection_id && !s.session_id))
            invalid();
        if (k.name === "conversation_create"
            ? Object.keys(s).length !== 0
            : transition ||
                k.name.startsWith("connection_") ||
                k.name.startsWith("stream_")
                ? !s.conversation_id || !s.session_id || !s.connection_id
                : k.name.startsWith("session_")
                    ? !s.conversation_id || !s.session_id || !!s.connection_id
                    : k.name === "message_send" ||
                        k.name.startsWith("conversation_participant_")
                        ? !s.conversation_id ||
                            !!s.session_id ||
                            !!s.connection_id
                        : !!s.connection_id)
            invalid();
        const key = makeKey(copy.operation_id, copy.command, {
            ...(s.conversation_id
                ? { conversationId: s.conversation_id }
                : {}),
            ...(s.session_id ? { sessionId: s.session_id } : {}),
            ...(s.connection_id
                ? { connectionId: s.connection_id }
                : {}),
        });
        const reference = freeze({ ...key });
        references.set(reference, {
            saved: freeze(copy),
            kind: expected,
        });
        return new SdkOperation(this, reference, expected, copy, (value) => k.decode(value, this, reference, copy));
    }
    fromReply(value, kind, key) {
        let operation;
        try {
            operation = this.restore(value, kind);
        }
        catch {
            return protocol();
        }
        if (!equalKey(operation.reference, key))
            return protocol();
        return operation;
    }
    /** Internal to owned handles; public behavior remains typed, not raw JSON. */
    [runOperation](saved, expected, action, options, decode, key, mutating = false) {
        const k = kinds.get(expected);
        if (!k)
            invalid();
        return this.helper.request({ type: "operation", reference: saved, kind: k.name, action }, options, decode, key, mutating);
    }
    [sendMessage](id, input, options) {
        if (!input ||
            !identifier(input.messageId) ||
            !input.to ||
            !identifier(input.to.first) ||
            !Array.isArray(input.to.rest) ||
            input.to.rest.some((id) => !identifier(id)) ||
            typeof input.text !== "string")
            invalid();
        const key = makeKey(options?.operationId, "message.send", {
            conversationId: id,
        });
        return this.helper.request({
            type: "conversation_send",
            conversation_id: id,
            message_id: input.messageId,
            to: [input.to.first, ...input.to.rest],
            text: input.text,
            mutation: mutation(options),
        }, options, (value) => this.fromReply(value, this.operationKinds.message_send, key), key, true);
    }
    [readReceipts](cid, mid, options) {
        sdkId(mid);
        return this.helper.request({ type: "message_receipts", conversation_id: cid, message_id: mid }, options, (value) => deliveryObservations(value, cid, mid));
    }
    [refreshConversation](id, options) {
        return this.helper.request({ type: "conversation_refresh", conversation_id: id }, options, (value) => {
            if (!object(value, [
                "id",
                "revision",
                "closed",
                "participants",
            ]) ||
                value.id !== id ||
                !Number.isSafeInteger(value.revision) ||
                Number(value.revision) < 0 ||
                typeof value.closed !== "boolean" ||
                !Array.isArray(value.participants))
                return protocol();
            const seen = new Set();
            for (const p of value.participants) {
                if (!object(p, [
                    "id",
                    "conversationId",
                    "identityId",
                    "kind",
                    "role",
                    "membershipEpoch",
                    "active",
                    "removalReason",
                ]) ||
                    !identifier(p.id) ||
                    seen.has(p.id) ||
                    p.conversationId !== id ||
                    !identifier(p.identityId) ||
                    typeof p.kind !== "string" ||
                    !["human", "ai", "system", "external"].includes(p.kind) ||
                    typeof p.role !== "string" ||
                    !Number.isSafeInteger(p.membershipEpoch) ||
                    Number(p.membershipEpoch) < 0 ||
                    typeof p.active !== "boolean" ||
                    (p.removalReason !== null &&
                        typeof p.removalReason !== "string"))
                    return protocol();
                seen.add(p.id);
            }
            return freeze(value);
        });
    }
    [mutateParticipant](cid, action, kind, options) {
        const key = makeKey(options?.operationId, kind.command, {
            conversationId: cid,
        });
        return this.helper.request({ ...action, conversation_id: cid, mutation: mutation(options) }, options, (value) => this.fromReply(value, kind, key), key, true);
    }
    [readParticipant](cid, pid, options) {
        return this.helper.request({
            type: "participant_refresh",
            conversation_id: cid,
            participant_id: pid,
        }, options, (value) => participantSnapshot(value, cid, pid));
    }
    [mutateSession](cid, sid, action, kind, options, via) {
        const key = makeKey(options?.operationId, kind.command, {
            ...(via ? { connectionId: via } : {}),
            conversationId: cid,
            sessionId: sid,
        });
        return this.helper.request({
            type: "session",
            conversation_id: cid,
            session_id: sid,
            action: { ...action, mutation: mutation(options) },
        }, options, (value) => {
            const op = this.fromReply(value, kind, key);
            if (via &&
                (!object(value, [
                    "schema",
                    "operation_id",
                    "command",
                    "scope",
                    "continuation",
                ]) ||
                    !object(value.continuation, [
                        "kind",
                        "retained_connection_id",
                        "record_id",
                    ]) ||
                    value.continuation.retained_connection_id !==
                        action.retain))
                return protocol();
            return op;
        }, key, true);
    }
    [returnConsultation](saved, original, expected, options) {
        if (original.command !== "session.consult" ||
            options?.operationId === original.operationId)
            invalid();
        sdkRevision(expected);
        const kind = this.operationKinds.session_consult_return;
        const key = makeKey(options?.operationId, kind.command, original.scope);
        return this.helper.request({
            type: "consultation_return",
            reference: saved,
            expected_revision: expected,
            mutation: mutation(options),
        }, options, (value) => {
            const operation = this.fromReply(value, kind, key);
            const before = saved.continuation, after = value.continuation;
            if (after.retained_connection_id !==
                before.retained_connection_id ||
                (before.record_id != null &&
                    after.consultation_record_id !== before.record_id))
                return protocol();
            return operation;
        }, key, true);
    }
    [submitMovement](type, cid, sid, via, input, kind, context, options) {
        const key = makeKey(options?.operationId, kind.command, {
            conversationId: cid,
            sessionId: sid,
            connectionId: via,
        });
        return this.helper.request({
            type,
            conversation_id: cid,
            session_id: sid,
            ...(type === "connection_move" ? { connection_id: via } : {}),
            input: { ...input, mutation: mutation(options) },
        }, options, (value) => {
            const op = this.fromReply(value, kind, key);
            if (canonicalJson(value.continuation) !==
                canonicalJson(context))
                return protocol();
            return op;
        }, key, true);
    }
    [handoffSession](cid, sid, input, options) {
        const captured = handoffInput(input);
        if (input.consultation !== undefined) {
            const ref = input.consultation?.reference, metadata = references.get(ref);
            if (!metadata ||
                kinds.get(metadata.kind)?.name !== "session_consult" ||
                ref.scope.conversationId !== cid ||
                ref.scope.sessionId !== sid ||
                ref.operationId === options?.operationId ||
                metadata.saved.continuation === undefined ||
                metadata.saved.continuation
                    .retained_connection_id !== captured.retain ||
                (options?.extensions &&
                    typeof options.extensions === "object" &&
                    Object.hasOwn(options.extensions, "consultation_record_id")))
                invalid();
            captured.consultation = metadata.saved;
        }
        const context = {
            kind: "handoff",
            retained_connection_id: captured.retain,
            source_participant: captured.from_participant,
            target_participant: captured.to,
            candidate: captured.candidate,
            role: captured.role,
            outgoing: captured.outgoing,
        };
        return this[submitMovement]("session_handoff", cid, sid, captured.via, captured, this.operationKinds.session_handoff, context, options);
    }
    [moveConnection](cid, sid, via, input, options) {
        const captured = moveInput(input, via), context = {
            kind: "replacement",
            retained_connection_id: captured.retain,
            target: captured.target,
        };
        return this[submitMovement]("connection_move", cid, sid, via, captured, this.operationKinds.connection_replace, context, options);
    }
    [readSession](cid, sid, options) {
        return this.helper.request({
            type: "session",
            conversation_id: cid,
            session_id: sid,
            action: { type: "refresh" },
        }, options, (value) => sessionSnapshot(value, cid, sid));
    }
    [connectionContext](cid, sid) {
        return {
            cid,
            sid,
            kinds: this.operationKinds,
            move: (id, input, options) => this[moveConnection](cid, sid, id, input, options),
            mutate: (id, action, kind, options) => {
                const key = makeKey(options?.operationId, kind.command, {
                    conversationId: cid,
                    sessionId: sid,
                    connectionId: id,
                });
                return this.helper.request({
                    type: "connection",
                    conversation_id: cid,
                    session_id: sid,
                    connection_id: id,
                    action: { ...action, mutation: mutation(options) },
                }, options, (value) => this.fromReply(value, kind, key), key, true);
            },
            read: (id, action, options, decode) => this.helper.request({
                type: "connection",
                conversation_id: cid,
                session_id: sid,
                connection_id: id,
                action,
            }, options, decode),
        };
    }
    close(options) {
        return this.helper.close(options);
    }
}
export class SdkConversation {
    client;
    id;
    sessions;
    participants;
    constructor(client, id) {
        this.client = client;
        this.id = id;
        this.sessions = createSessions({
            cid: this.id,
            kinds: client.operationKinds,
            handoff: (sid, input, options) => client[handoffSession](this.id, sid, input, options),
            connections: (sid) => createConnections(client[connectionContext](this.id, sid)),
            mutate: (sid, action, kind, options, via) => client[mutateSession](this.id, sid, action, kind, options, via),
            refresh: (sid, options) => client[readSession](this.id, sid, options),
        });
        this.participants = Object.freeze({
            byId: (id) => {
                sdkId(id);
                return new SdkParticipant(client, this.id, id);
            },
            add: (input, options) => {
                if (!input ||
                    !identifier(input.identityId) ||
                    !["human", "ai", "system", "external"].includes(input.kind) ||
                    typeof input.role !== "string")
                    invalid();
                sdkRevision(input.expectedRevision);
                return client[mutateParticipant](this.id, {
                    type: "participant_add",
                    identity_id: input.identityId,
                    kind: input.kind,
                    role: input.role,
                    expected_revision: input.expectedRevision,
                }, client.operationKinds.conversation_participant_add, options);
            },
        });
        Object.freeze(this);
    }
    events(input, options) {
        return this.client[eventStream](this.id, input, options);
    }
    refresh(options) {
        return this.client[refreshConversation](this.id, options);
    }
    receipts(messageId, options) {
        return this.client[readReceipts](this.id, messageId, options);
    }
    send(input, options) {
        return this.client[sendMessage](this.id, input, options);
    }
}
export class SdkParticipant {
    client;
    conversationId;
    id;
    constructor(client, conversationId, id) {
        this.client = client;
        this.conversationId = conversationId;
        this.id = id;
        Object.freeze(this);
    }
    refresh(options) {
        return this.client[readParticipant](this.conversationId, this.id, options);
    }
    update(input, options) {
        if (!input || typeof input.role !== "string")
            invalid();
        sdkRevision(input.expectedConversationRevision);
        return this.client[mutateParticipant](this.conversationId, {
            type: "participant_update",
            participant_id: this.id,
            role: input.role,
            expected_revision: input.expectedConversationRevision,
        }, this.client.operationKinds.conversation_participant_update, options);
    }
    remove(input, options) {
        if (!input)
            invalid();
        sdkRevision(input.expectedConversationRevision);
        return this.client[mutateParticipant](this.conversationId, {
            type: "participant_remove",
            participant_id: this.id,
            expected_revision: input.expectedConversationRevision,
        }, this.client.operationKinds.conversation_participant_remove, options);
    }
}
export class SdkOperation {
    client;
    reference;
    kind;
    saved;
    decode;
    #observedRecord;
    observe(value) {
        const observed = snapshot(value, this.reference);
        if (observed.recordId !== undefined) {
            if (this.#observedRecord !== undefined &&
                this.#observedRecord !== observed.recordId)
                return protocol();
            this.#observedRecord = observed.recordId;
        }
        return observed;
    }
    constructor(client, reference, kind, saved, decode) {
        this.client = client;
        this.reference = reference;
        this.kind = kind;
        this.saved = saved;
        this.decode = decode;
        const context = saved.continuation;
        if (context &&
            typeof context === "object" &&
            !Array.isArray(context) &&
            typeof context.record_id === "string")
            this.#observedRecord = context.record_id;
        Object.freeze(this);
    }
    returnToRetained(input, options) {
        if (!input)
            invalid();
        return this.client[returnConsultation](this.saved, this.reference, input.expectedTopologyRevision, options);
    }
    referenceJson() {
        return copied(this.saved);
    }
    status(options) {
        return this.client[runOperation](this.saved, this.kind, { type: "status" }, options, (value) => this.observe(value), this.reference);
    }
    waitUntilReady(options) {
        return this.client[runOperation](this.saved, this.kind, { type: "wait_until_ready" }, options, (value) => {
            const s = this.observe(value);
            if (!["ready", "committing", "completed"].includes(s.state))
                return protocol();
            return s;
        }, this.reference);
    }
    wait(options) {
        return this.client[runOperation](this.saved, this.kind, { type: "wait" }, options, (value) => {
            if (!object(value, ["snapshot", "value"]))
                return protocol();
            const s = this.observe(value.snapshot);
            if (s.state !== "completed")
                return protocol();
            return Object.freeze({
                snapshot: s,
                value: this.decode(value.value),
            });
        }, this.reference);
    }
    cancel(options) {
        return this.decide({ type: "cancel", mutation: mutation(options) }, options, this.client.operationKinds.operation_cancel);
    }
    reconcile(input, options) {
        if (!input ||
            !["completed", "cancelled", "failed"].includes(input.outcome) ||
            !identifier(input.evidenceId))
            invalid();
        return this.decide({
            type: "reconcile",
            outcome: input.outcome,
            evidence_id: input.evidenceId,
            mutation: mutation(options),
        }, options, this.client.operationKinds.operation_reconcile);
    }
    decide(action, options, kind) {
        const key = makeKey(options.operationId, kind.command, {
            ...(this.reference.scope.conversationId
                ? { conversationId: this.reference.scope.conversationId }
                : {}),
            ...(this.reference.scope.sessionId
                ? { sessionId: this.reference.scope.sessionId }
                : {}),
        });
        return this.client[runOperation](this.saved, this.kind, action, options, (value) => {
            const operation = this.client.operations.restore(value, kind);
            if (!equalKey(operation.reference, key))
                return protocol();
            return operation;
        }, key, true);
    }
}
/** Internal trusted-backend operation owner; no provisioning on ParticipantClient. */
export function createAdministrativeOperations(helper, validateReference) {
    const administrativeKinds = new Map();
    const select = (command, name) => {
        if (name === "operation_cancel" && command === "operation.cancel")
            return operationKinds.operation_cancel;
        if (name === "operation_reconcile" && command === "operation.reconcile")
            return operationKinds.operation_reconcile;
        if (name !== "administrative")
            return invalid();
        let token = administrativeKinds.get(command);
        if (!token) {
            token = kind("administrative", command, (value) => freeze(copied(value)));
            administrativeKinds.set(command, token);
        }
        return token;
    };
    function captured(value, expected) {
        const saved = copied(value);
        if (Buffer.byteLength(canonicalJson(saved)) > 4096 ||
            !object(saved, ["schema", "operation_id", "command", "scope"]) ||
            saved.schema !== "connect-operation-reference/1" ||
            !identifier(saved.operation_id) ||
            saved.command !== expected.command ||
            !object(saved.scope, []) ||
            Object.keys(saved.scope).length)
            invalid();
        const key = makeKey(saved.operation_id, saved.command, {});
        const reference = freeze({ ...key });
        references.set(reference, {
            saved: freeze(saved),
            kind: expected,
        });
        const k = kinds.get(expected);
        const decode = (value) => (k.name === "administrative"
            ? freeze(copied(value))
            : decision(value, undefined, key));
        return new SdkOperation(owner, reference, expected, saved, decode);
    }
    function restore(saved, expected) {
        const checked = validateReference(saved);
        const selected = select(checked.reference.command, checked.kind);
        if (expected && selected !== expected)
            invalid();
        return captured(checked.reference, (expected ?? selected));
    }
    const owner = {
        operationKinds,
        operations: { restore },
        [returnConsultation]() {
            throw new NodeSdkError("unsupported", "not-submitted");
        },
        [runOperation](saved, _kind, action, options, decode, key, mutating = false) {
            return helper.request({ type: "administrative_operation", reference: saved, action }, options, decode, key, mutating);
        },
    };
    return {
        restore: (saved) => restore(saved),
        recover(reference) {
            const saved = references.get(reference), k = saved && kinds.get(saved.kind);
            if (!saved ||
                !k ||
                ![
                    "administrative",
                    "operation_cancel",
                    "operation_reconcile",
                ].includes(k.name))
                invalid();
            return captured(saved.saved, saved.kind);
        },
        fromReply(value, key) {
            const expected = select(key.command, "administrative");
            let operation;
            try {
                operation = captured(value, expected);
            }
            catch {
                return protocol();
            }
            if (!equalKey(operation.reference, key))
                return protocol();
            return operation;
        },
    };
}
