import {
    createIncoming,
    type SdkIncomingInvitations,
} from "./node_incoming.js";
import { createRaw, type SdkRawCanonical } from "./node_raw.js";
import {
    deliveryObservations,
    type SdkDeliveryObservation,
} from "./node_receipts.js";
import {
    handoffInput,
    moveInput,
    movementContext,
    handoffResult,
    moveResult,
    type SdkHandoffInput,
    type SdkHandoffResult,
    type SdkMoveInput,
    type SdkMoveResult,
} from "./node_movements.js";
import {
    createConnections,
    connectionHandle,
    connectionUpdate,
    streamSelection,
    type ConnectionContext,
    type ConnectionId,
    type SdkConnection,
    type SdkConnectionUpdateReceipt,
    type SdkStreamSelectionReceipt,
} from "./node_connections.js";
import {
    createSessions,
    sessionSnapshot,
    type SdkSession,
    type SdkSessions,
    type SessionId,
    type SdkSessionDecision,
    type SdkSessionSnapshot,
} from "./node_sessions.js";
import {
    canonicalJson,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import { SdkHelper, NodeSdkError, object } from "./sdk_helper.js";
import type { OperationKey as CanonicalOperationKey } from "./canonical_error.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";

declare const identity: unique symbol;
declare const resultType: unique symbol;
declare const revisionFamily: unique symbol;
export type SdkRevision<K extends string> = number & {
    readonly [revisionFamily]: K;
};
export function sdkRevision<K extends string>(value: number): SdkRevision<K> {
    if (!Number.isSafeInteger(value) || value < 0) invalid();
    return value as SdkRevision<K>;
}
export type SdkId<K extends string> = string & { readonly [identity]: K };
export type ConversationId = SdkId<"conversation">;
export type IdentityId = SdkId<"identity">;
export type ParticipantId = SdkId<"participant">;
export type MessageId = SdkId<"message">;
export type OperationId = SdkId<"operation">;
export interface SdkOperationKey extends CanonicalOperationKey {
    readonly operationId: OperationId;
    readonly scope: {
        readonly conversationId?: ConversationId;
        readonly sessionId?: SdkId<"session">;
        readonly connectionId?: SdkId<"connection">;
    };
}
type OperationKey = SdkOperationKey;
export interface SdkMutationOptions extends CanonicalRequestOptions {
    readonly operationId: OperationId;
    readonly extensions?: Readonly<Record<string, Json>>;
}
export type OperationState =
    | "accepted"
    | "preparing"
    | "ready"
    | "committing"
    | "completed"
    | "cancelled"
    | "failed"
    | "interrupted"
    | "unknown";
export interface SdkOperationSnapshot {
    readonly key: OperationKey;
    readonly state: OperationState;
    readonly revision: number;
    readonly recordId?: string;
    readonly cancelRequested?: boolean;
    readonly expiresAt?: number;
    readonly problem?: Json;
}
export interface SdkCompletion<T> {
    readonly snapshot: SdkOperationSnapshot;
    readonly value: T;
}
export interface SdkOperationKind<T> {
    readonly command: string;
    readonly [resultType]: (value: T) => T;
}
export interface SdkOperationReference<T> extends OperationKey {
    readonly [resultType]: (value: T) => T;
}
/** Interface support on the selected Node facade; not permission or readiness. */
export interface SdkCapabilitySnapshot {
    readonly profile: "connect/2";
    readonly runtimeRole: "node";
    readonly selectedTransport: "quic";
    readonly features: ReadonlySet<string>;
    readonly fallbackReason?: string;
}
export interface SdkCapabilities {
    refresh(options: CanonicalRequestOptions): Promise<SdkCapabilitySnapshot>;
    require(feature: string, options: CanonicalRequestOptions): Promise<void>;
}
function capabilitySnapshot(value: Json): SdkCapabilitySnapshot {
    if (
        !object(value, [
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
        value.features.some(
            (f) => typeof f !== "string" || !/^[a-z0-9-]{1,128}$/.test(f),
        ) ||
        new Set(value.features).size !== value.features.length
    )
        return protocol();
    // Advertise only exposed Node interfaces that this Rust configuration supports.
    const set = new Set<string>(
        value.features.filter(
            (f) =>
                f === "original-id-operations" ||
                f === "message-receipts" ||
                f === "incoming-invitations" ||
                f === "raw-canonical",
        ) as string[],
    );
    const features: ReadonlySet<string> = Object.freeze({
        get size() {
            return set.size;
        },
        has: (feature: string) => set.has(feature),
        entries: () => set.entries(),
        keys: () => set.keys(),
        values: () => set.values(),
        [Symbol.iterator]: () => set.values(),
        forEach: (
            callback: (
                value: string,
                key: string,
                owner: ReadonlySet<string>,
            ) => void,
            thisArg?: unknown,
        ) => set.forEach((v) => callback.call(thisArg, v, v, features)),
    });
    return Object.freeze({
        profile: "connect/2",
        runtimeRole: "node",
        selectedTransport: "quic",
        features,
    });
}
export interface SdkConsultationResult {
    readonly session: SdkSession;
    readonly retained: readonly SdkConnection[];
    readonly admitted: readonly SdkConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
    readonly conversationRevision?: SdkRevision<"conversation">;
}
export type SdkConsultationOperation = SdkOperation<SdkConsultationResult>;
export interface SdkConsultInput {
    readonly via: ConnectionId;
    readonly retain: ConnectionId;
    readonly with: {
        readonly first: ParticipantId;
        readonly rest: readonly ParticipantId[];
    };
    readonly hold: "silence" | "configured-media";
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface SdkMessageAcceptance {
    readonly messageId: MessageId;
    readonly operation: OperationKey;
    readonly boundary: "local-mutation";
}
export interface SdkDecisionReceipt {
    readonly decision: OperationKey;
    readonly target: OperationId;
    readonly observedState: OperationState;
    readonly disposition: string;
    readonly evidenceId?: string;
}
export interface SdkConversationSnapshot {
    readonly id: ConversationId;
    readonly revision: SdkRevision<"conversation">;
    readonly closed: boolean;
    readonly participants: readonly {
        readonly id: ParticipantId;
        readonly conversationId: ConversationId;
        readonly identityId: IdentityId;
        readonly kind: "human" | "ai" | "system" | "external";
        readonly role: string;
        readonly membershipEpoch: number;
        readonly active: boolean;
        readonly removalReason: string | null;
    }[];
}
export type SdkParticipantSnapshot =
    SdkConversationSnapshot["participants"][number];
type KindName =
    | "administrative"
    | "conversation_create"
    | "message_send"
    | "operation_cancel"
    | "operation_reconcile"
    | "conversation_participant_add"
    | "conversation_participant_update"
    | "conversation_participant_remove"
    | "session_invite"
    | "session_accept"
    | "session_update"
    | "session_end"
    | "session_reject"
    | "session_cancel"
    | "session_consult"
    | "session_consult_return"
    | "session_handoff"
    | "connection_replace"
    | "connection_offer"
    | "connection_update"
    | "connection_end"
    | "stream_subscribe"
    | "stream_unsubscribe";
type Kind<T> = {
    name: KindName;
    command: string;
    decode: (
        value: Json,
        client: NodeClient,
        key: OperationKey,
        saved: Json,
    ) => T;
};
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
const kinds = new WeakMap<object, Kind<unknown>>();
const references = new WeakMap<
    object,
    { saved: JsonObject; kind: SdkOperationKind<unknown> }
>();
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
function invalid(): never {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol(): never {
    throw new Error("Invalid typed SDK helper result");
}
export function sdkId<K extends string>(value: string): SdkId<K> {
    if (!identifier(value)) invalid();
    return value as SdkId<K>;
}
function freeze<T>(value: T): T {
    if (value && typeof value === "object") {
        for (const child of Object.values(value)) freeze(child);
        Object.freeze(value);
    }
    return value;
}
const copied = (value: Json): Json =>
    parseCanonicalJson(Buffer.from(canonicalJson(value)));
function mutation(options: SdkMutationOptions): JsonObject {
    if (!options || !identifier(options.operationId)) invalid();
    return {
        operation_id: options.operationId,
        ...(options.extensions === undefined
            ? {}
            : { extensions: options.extensions as JsonObject }),
    };
}
function makeKey(
    operationId: string,
    command: string,
    scope: CanonicalOperationKey["scope"],
): OperationKey {
    return freeze({
        operationId: operationId as OperationId,
        command,
        scope: { ...scope } as OperationKey["scope"],
    });
}
function readKey(value: Json): OperationKey {
    if (
        !object(value, ["operationId", "command", "scope"]) ||
        !identifier(value.operationId) ||
        typeof value.command !== "string" ||
        !object(value.scope, ["conversationId", "sessionId", "connectionId"])
    )
        return protocol();
    const scope: Record<string, string> = {};
    for (const k of ["conversationId", "sessionId", "connectionId"] as const) {
        const id = value.scope[k];
        if (id === null || id === undefined) continue;
        if (!identifier(id)) return protocol();
        scope[k] = id;
    }
    if (
        (scope.sessionId && !scope.conversationId) ||
        (scope.connectionId && !scope.sessionId)
    )
        return protocol();
    return makeKey(value.operationId, value.command, scope);
}
function equalKey(a: OperationKey, b: OperationKey): boolean {
    return (
        a.operationId === b.operationId &&
        a.command === b.command &&
        ["conversationId", "sessionId", "connectionId"].every(
            (k) =>
                a.scope[k as keyof typeof a.scope] ===
                b.scope[k as keyof typeof b.scope],
        )
    );
}
function snapshot(value: Json, key: OperationKey): SdkOperationSnapshot {
    if (
        !object(value, [
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
            !Number.isSafeInteger(value.expiresAt))
    )
        return protocol();
    return freeze({
        key,
        state: value.state as OperationState,
        revision: value.revision as number,
        ...(value.recordId == null
            ? {}
            : { recordId: value.recordId as string }),
        ...(value.cancelRequested == null
            ? {}
            : { cancelRequested: value.cancelRequested as boolean }),
        ...(value.expiresAt == null
            ? {}
            : { expiresAt: value.expiresAt as number }),
        ...(value.problem == null ? {} : { problem: value.problem }),
    });
}
function kind<T>(
    name: KindName,
    command: string,
    decode: Kind<T>["decode"],
): SdkOperationKind<T> {
    const value = Object.freeze({ command }) as SdkOperationKind<T>;
    kinds.set(value, { name, command, decode } as Kind<unknown>);
    return value;
}
function decision(
    value: Json,
    _client: NodeClient | undefined,
    key: OperationKey,
): SdkDecisionReceipt {
    if (
        !object(value, [
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
            : !identifier(value.evidenceId))
    )
        return protocol();
    return freeze({
        decision: key,
        target: value.target as OperationId,
        observedState: value.observedState as OperationState,
        disposition: value.disposition,
        ...(value.evidenceId == null
            ? {}
            : { evidenceId: value.evidenceId as string }),
    });
}
function participant(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkParticipant {
    if (
        !object(value, ["id", "conversationId"]) ||
        !identifier(value.id) ||
        value.conversationId !== key.scope.conversationId
    )
        return protocol();
    return client.conversations
        .byId(value.conversationId as ConversationId)
        .participants.byId(value.id as ParticipantId);
}
function participantSnapshot(
    value: Json,
    cid: ConversationId,
    pid: ParticipantId,
): SdkParticipantSnapshot {
    if (
        !object(value, [
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
            typeof value.removalReason !== "string")
    )
        return protocol();
    return freeze(value) as unknown as SdkParticipantSnapshot;
}
function session(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkSession {
    if (
        !object(value, ["id", "conversationId"]) ||
        value.id !== key.scope.sessionId ||
        value.conversationId !== key.scope.conversationId
    )
        return protocol();
    return client.conversations
        .byId(value.conversationId as ConversationId)
        .sessions.byId(value.id as SessionId);
}
function consultation(
    value: Json,
    client: NodeClient,
    key: OperationKey,
    saved: Json,
): SdkConsultationResult {
    if (
        !object(value, [
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
        Number(value.topologyRevision) < 1
    )
        return protocol();
    const owner = session(value.session, client, key),
        seen = new Set<string>();
    const decode = (v: Json): SdkConnection => {
        if (
            !object(v, [
                "id",
                "conversationId",
                "sessionId",
                "receiptStreamRevision",
            ]) ||
            !identifier(v.id) ||
            v.conversationId !== owner.conversationId ||
            v.sessionId !== owner.id ||
            v.receiptStreamRevision !== null ||
            seen.has(v.id)
        )
            return protocol();
        seen.add(v.id);
        return owner.connections.byId(v.id as ConnectionId);
    };
    if (value.retained.length + value.admitted.length > 100) return protocol();
    const retained = value.retained.map(decode),
        admitted = value.admitted.map(decode);
    if (
        !object(saved, [
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
        !retained.some(
            (c) =>
                c.id ===
                (saved.continuation as JsonObject).retained_connection_id,
        )
    )
        return protocol();
    return Object.freeze({
        session: owner,
        retained: Object.freeze(retained),
        admitted: Object.freeze(admitted),
        retired: Object.freeze([]),
        topologyRevision: sdkRevision<"topology">(
            value.topologyRevision as number,
        ),
    });
}
function validConsultationContext(
    value: Json,
    command: string,
    via: Json,
): boolean {
    const returning = command === "session.consult.return";
    if (
        !object(
            value,
            returning
                ? ["kind", "retained_connection_id", "consultation_record_id"]
                : ["kind", "retained_connection_id", "record_id"],
        ) ||
        value.kind !== (returning ? "consultation-return" : "consultation") ||
        !identifier(value.retained_connection_id) ||
        value.retained_connection_id === via
    )
        return false;
    return returning
        ? identifier(value.consultation_record_id)
        : value.record_id == null || identifier(value.record_id);
}
function sessionDecision(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkSessionDecision {
    if (
        !object(value, ["session", "state", "topologyRevision"]) ||
        value.state !==
            (key.command === "session.reject" ? "rejected" : "cancelled") ||
        !Number.isSafeInteger(value.topologyRevision) ||
        Number(value.topologyRevision) < 0
    )
        return protocol();
    return Object.freeze({
        session: session(value.session, client, key),
        state: value.state as string,
        topologyRevision: value.topologyRevision as SdkRevision<"topology">,
    });
}
function connection(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkConnection {
    return connectionHandle(
        value,
        client[connectionContext](
            key.scope.conversationId as ConversationId,
            key.scope.sessionId as SessionId,
        ),
        key.scope.connectionId as ConnectionId,
    );
}
function connectionReceipt(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkConnectionUpdateReceipt {
    return connectionUpdate(
        value,
        client[connectionContext](
            key.scope.conversationId as ConversationId,
            key.scope.sessionId as SessionId,
        ),
        key.scope.connectionId as ConnectionId,
    );
}
function selectionReceipt(
    value: Json,
    client: NodeClient,
    key: OperationKey,
): SdkStreamSelectionReceipt {
    return streamSelection(
        value,
        client[connectionContext](
            key.scope.conversationId as ConversationId,
            key.scope.sessionId as SessionId,
        ),
        key.scope.connectionId as ConnectionId,
    );
}
const operationKinds = Object.freeze({
    session_handoff: kind<SdkHandoffResult>(
        "session_handoff",
        "session.handoff",
        (value, client, key, saved) =>
            handoffResult(
                value,
                {
                    key,
                    connection: (id) =>
                        client.conversations
                            .byId(key.scope.conversationId!)
                            .sessions.byId(key.scope.sessionId!)
                            .connections.byId(id),
                },
                saved,
            ),
    ),
    connection_replace: kind<SdkMoveResult>(
        "connection_replace",
        "connection.replace",
        (value, client, key, saved) =>
            moveResult(
                value,
                {
                    key,
                    connection: (id) =>
                        client.conversations
                            .byId(key.scope.conversationId!)
                            .sessions.byId(key.scope.sessionId!)
                            .connections.byId(id),
                },
                saved,
            ),
    ),
    session_consult: kind<SdkConsultationResult>(
        "session_consult",
        "session.consult",
        consultation,
    ),
    session_consult_return: kind(
        "session_consult_return",
        "session.consult.return",
        session,
    ),
    connection_offer: kind("connection_offer", "connection.offer", connection),
    connection_update: kind(
        "connection_update",
        "connection.update",
        connectionReceipt,
    ),
    connection_end: kind("connection_end", "connection.end", connection),
    stream_subscribe: kind(
        "stream_subscribe",
        "stream.subscribe",
        selectionReceipt,
    ),
    stream_unsubscribe: kind(
        "stream_unsubscribe",
        "stream.unsubscribe",
        selectionReceipt,
    ),
    session_invite: kind("session_invite", "session.invite", session),
    session_accept: kind("session_accept", "session.accept", session),
    session_update: kind("session_update", "session.update", session),
    session_end: kind("session_end", "session.end", session),
    session_reject: kind("session_reject", "session.reject", sessionDecision),
    session_cancel: kind("session_cancel", "session.cancel", sessionDecision),
    conversation_participant_add: kind(
        "conversation_participant_add",
        "conversation.participant.add",
        participant,
    ),
    conversation_participant_update: kind(
        "conversation_participant_update",
        "conversation.participant.update",
        participant,
    ),
    conversation_participant_remove: kind(
        "conversation_participant_remove",
        "conversation.participant.remove",
        participant,
    ),
    conversation_create: kind<SdkConversation>(
        "conversation_create",
        "conversation.create",
        (value, client) => {
            if (!object(value, ["id"]) || !identifier(value.id))
                return protocol();
            return client.conversations.byId(value.id as ConversationId);
        },
    ),
    message_send: kind<SdkMessageAcceptance>(
        "message_send",
        "message.send",
        (value, _client, key) => {
            if (
                !object(value, ["messageId", "operation", "boundary"]) ||
                !identifier(value.messageId) ||
                value.boundary !== "local-mutation" ||
                !equalKey(readKey(value.operation), key)
            )
                return protocol();
            return freeze({
                messageId: value.messageId as MessageId,
                operation: key,
                boundary: "local-mutation",
            });
        },
    ),
    operation_cancel: kind("operation_cancel", "operation.cancel", decision),
    operation_reconcile: kind(
        "operation_reconcile",
        "operation.reconcile",
        decision,
    ),
});

/** Typed Conversation/message/decision slice of G6-062. Other control kinds,
 * events and remote HTTP remain under development. Incoming hints grant no custody.
 * One fixed SDK configuration, one Rust owner, no transparent restart/retry. */
export class NodeClient {
    readonly role = "node" as const;
    readonly operationKinds = operationKinds;
    readonly capabilities: SdkCapabilities;
    readonly raw: SdkRawCanonical;
    readonly conversations;
    readonly operations;
    incomingInvitations(
        options: CanonicalRequestOptions,
    ): SdkIncomingInvitations {
        return createIncoming(
            this.helper,
            (cid, sid) => this.conversations.byId(cid).sessions.byId(sid),
            options,
        );
    }
    private readonly helper: SdkHelper;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    }) {
        const captured = {
            config: config?.config,
            executable: config?.executable ?? "connect",
        };
        this.helper = new SdkHelper(captured);
        this.raw = createRaw(this.helper, captured.executable);
        const refresh = (options: CanonicalRequestOptions) =>
            this.helper.request(
                { type: "capabilities_refresh" },
                options,
                capabilitySnapshot,
            );
        this.capabilities = Object.freeze({
            refresh,
            require: async (
                feature: string,
                options: CanonicalRequestOptions,
            ): Promise<void> => {
                if (
                    typeof feature !== "string" ||
                    !/^[a-z0-9-]{1,128}$/.test(feature)
                )
                    throw new NodeSdkError("validation", "read-only");
                if (!(await refresh(options)).features.has(feature))
                    throw new NodeSdkError("unsupported", "read-only");
            },
        });
        this.conversations = Object.freeze({
            byId: (id: ConversationId): SdkConversation => {
                sdkId(id);
                return new SdkConversation(this, id);
            },
            create: async (
                input: {
                    readonly participants: readonly IdentityId[];
                    readonly policy: "persistent" | "ephemeral";
                },
                options: SdkMutationOptions,
            ): Promise<SdkOperation<SdkConversation>> => {
                if (
                    !input ||
                    !Array.isArray(input.participants) ||
                    input.participants.some((id) => !identifier(id)) ||
                    !["persistent", "ephemeral"].includes(input.policy)
                )
                    invalid();
                const key = makeKey(
                    options?.operationId,
                    "conversation.create",
                    {},
                );
                return this.helper.request(
                    {
                        type: "conversation_create",
                        participants: [...input.participants],
                        policy: input.policy,
                        mutation: mutation(options),
                    },
                    options,
                    (value) =>
                        this.fromReply(
                            value,
                            this.operationKinds.conversation_create,
                            key,
                        ),
                    key,
                    true,
                );
            },
        });
        this.operations = Object.freeze({
            restore: <T>(
                saved: Json,
                expected: SdkOperationKind<T>,
            ): SdkOperation<T> => this.restore(saved, expected),
            recover: <T>(
                reference: SdkOperationReference<T>,
            ): SdkOperation<T> => {
                const saved = references.get(reference);
                if (!saved) invalid();
                return this.restore(
                    saved.saved,
                    saved.kind as SdkOperationKind<T>,
                );
            },
        });
        Object.freeze(this);
    }
    private restore<T>(
        saved: Json,
        expected: SdkOperationKind<T>,
    ): SdkOperation<T> {
        const k = kinds.get(expected);
        if (!k || k.name === "administrative") invalid();
        let copy: Json;
        try {
            const text = canonicalJson(saved);
            if (Buffer.byteLength(text) > 4096) invalid();
            copy = copied(saved);
        } catch {
            return invalid();
        }
        if (
            !object(copy, [
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
            ])
        )
            invalid();
        const movement =
            k.name === "session_handoff" || k.name === "connection_replace";
        const transition =
            movement ||
            k.name === "session_consult" ||
            k.name === "session_consult_return";
        if (
            movement
                ? !movementContext(
                      copy.continuation,
                      k.command,
                      copy.scope.connection_id,
                  )
                : transition
                  ? !validConsultationContext(
                        copy.continuation,
                        k.command,
                        copy.scope.connection_id,
                    )
                  : "continuation" in copy
        )
            invalid();
        const s = copy.scope;
        for (const id of Object.values(s)) if (!identifier(id)) invalid();
        if (
            (s.session_id && !s.conversation_id) ||
            (s.connection_id && !s.session_id)
        )
            invalid();
        if (
            k.name === "conversation_create"
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
                      : !!s.connection_id
        )
            invalid();
        const key = makeKey(copy.operation_id, copy.command, {
            ...(s.conversation_id
                ? { conversationId: s.conversation_id as string }
                : {}),
            ...(s.session_id ? { sessionId: s.session_id as string } : {}),
            ...(s.connection_id
                ? { connectionId: s.connection_id as string }
                : {}),
        });
        const reference = freeze({ ...key }) as SdkOperationReference<T>;
        references.set(reference, {
            saved: freeze(copy),
            kind: expected as SdkOperationKind<unknown>,
        });
        return new SdkOperation(
            this,
            reference,
            expected,
            copy,
            (value) => k.decode(value, this, reference, copy) as T,
        );
    }
    private fromReply<T>(
        value: Json,
        kind: SdkOperationKind<T>,
        key: OperationKey,
    ): SdkOperation<T> {
        let operation: SdkOperation<T>;
        try {
            operation = this.restore(value, kind);
        } catch {
            return protocol();
        }
        if (!equalKey(operation.reference, key)) return protocol();
        return operation;
    }
    /** Internal to owned handles; public behavior remains typed, not raw JSON. */
    [runOperation]<T, R>(
        saved: Json,
        expected: SdkOperationKind<T>,
        action: JsonObject,
        options: CanonicalRequestOptions,
        decode: (v: Json) => R,
        key: OperationKey,
        mutating = false,
    ): Promise<R> {
        const k = kinds.get(expected);
        if (!k) invalid();
        return this.helper.request(
            { type: "operation", reference: saved, kind: k.name, action },
            options,
            decode,
            key,
            mutating,
        );
    }
    [sendMessage](
        id: ConversationId,
        input: {
            readonly messageId: MessageId;
            readonly to: {
                readonly first: ParticipantId;
                readonly rest: readonly ParticipantId[];
            };
            readonly text: string;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMessageAcceptance>> {
        if (
            !input ||
            !identifier(input.messageId) ||
            !input.to ||
            !identifier(input.to.first) ||
            !Array.isArray(input.to.rest) ||
            input.to.rest.some((id) => !identifier(id)) ||
            typeof input.text !== "string"
        )
            invalid();
        const key = makeKey(options?.operationId, "message.send", {
            conversationId: id,
        });
        return this.helper.request(
            {
                type: "conversation_send",
                conversation_id: id,
                message_id: input.messageId,
                to: [input.to.first, ...input.to.rest],
                text: input.text,
                mutation: mutation(options),
            },
            options,
            (value) =>
                this.fromReply(value, this.operationKinds.message_send, key),
            key,
            true,
        );
    }
    [readReceipts](
        cid: ConversationId,
        mid: MessageId,
        options: CanonicalRequestOptions,
    ): Promise<readonly SdkDeliveryObservation[]> {
        sdkId(mid);
        return this.helper.request(
            { type: "message_receipts", conversation_id: cid, message_id: mid },
            options,
            (value) => deliveryObservations(value, cid, mid),
        );
    }
    [refreshConversation](
        id: ConversationId,
        options: CanonicalRequestOptions,
    ): Promise<SdkConversationSnapshot> {
        return this.helper.request(
            { type: "conversation_refresh", conversation_id: id },
            options,
            (value) => {
                if (
                    !object(value, [
                        "id",
                        "revision",
                        "closed",
                        "participants",
                    ]) ||
                    value.id !== id ||
                    !Number.isSafeInteger(value.revision) ||
                    Number(value.revision) < 0 ||
                    typeof value.closed !== "boolean" ||
                    !Array.isArray(value.participants)
                )
                    return protocol();
                const seen = new Set();
                for (const p of value.participants) {
                    if (
                        !object(p, [
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
                        !["human", "ai", "system", "external"].includes(
                            p.kind,
                        ) ||
                        typeof p.role !== "string" ||
                        !Number.isSafeInteger(p.membershipEpoch) ||
                        Number(p.membershipEpoch) < 0 ||
                        typeof p.active !== "boolean" ||
                        (p.removalReason !== null &&
                            typeof p.removalReason !== "string")
                    )
                        return protocol();
                    seen.add(p.id);
                }
                return freeze(value) as unknown as SdkConversationSnapshot;
            },
        );
    }
    [mutateParticipant](
        cid: ConversationId,
        action: JsonObject,
        kind: SdkOperationKind<SdkParticipant>,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkParticipant>> {
        const key = makeKey(options?.operationId, kind.command, {
            conversationId: cid,
        });
        return this.helper.request(
            { ...action, conversation_id: cid, mutation: mutation(options) },
            options,
            (value) => this.fromReply(value, kind, key),
            key,
            true,
        );
    }
    [readParticipant](
        cid: ConversationId,
        pid: ParticipantId,
        options: CanonicalRequestOptions,
    ): Promise<SdkParticipantSnapshot> {
        return this.helper.request(
            {
                type: "participant_refresh",
                conversation_id: cid,
                participant_id: pid,
            },
            options,
            (value) => participantSnapshot(value, cid, pid),
        );
    }
    [mutateSession]<T>(
        cid: ConversationId,
        sid: SessionId,
        action: JsonObject,
        kind: SdkOperationKind<T>,
        options: SdkMutationOptions,
        via?: ConnectionId,
    ): Promise<SdkOperation<T>> {
        const key = makeKey(options?.operationId, kind.command, {
            ...(via ? { connectionId: via } : {}),
            conversationId: cid,
            sessionId: sid,
        });
        return this.helper.request(
            {
                type: "session",
                conversation_id: cid,
                session_id: sid,
                action: { ...action, mutation: mutation(options) },
            },
            options,
            (value) => {
                const op = this.fromReply(value, kind, key);
                if (
                    via &&
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
                            action.retain)
                )
                    return protocol();
                return op;
            },
            key,
            true,
        );
    }
    [returnConsultation](
        saved: Json,
        original: OperationKey,
        expected: SdkRevision<"topology">,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        if (
            original.command !== "session.consult" ||
            options?.operationId === original.operationId
        )
            invalid();
        sdkRevision(expected);
        const kind = this.operationKinds.session_consult_return;
        const key = makeKey(options?.operationId, kind.command, original.scope);
        return this.helper.request(
            {
                type: "consultation_return",
                reference: saved,
                expected_revision: expected,
                mutation: mutation(options),
            },
            options,
            (value) => {
                const operation = this.fromReply(value, kind, key);
                const before = (saved as JsonObject).continuation as JsonObject,
                    after = (value as JsonObject).continuation as JsonObject;
                if (
                    after.retained_connection_id !==
                        before.retained_connection_id ||
                    (before.record_id != null &&
                        after.consultation_record_id !== before.record_id)
                )
                    return protocol();
                return operation;
            },
            key,
            true,
        );
    }
    [submitMovement]<T>(
        type: "session_handoff" | "connection_move",
        cid: ConversationId,
        sid: SessionId,
        via: ConnectionId,
        input: JsonObject,
        kind: SdkOperationKind<T>,
        context: JsonObject,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<T>> {
        const key = makeKey(options?.operationId, kind.command, {
            conversationId: cid,
            sessionId: sid,
            connectionId: via,
        });
        return this.helper.request(
            {
                type,
                conversation_id: cid,
                session_id: sid,
                ...(type === "connection_move" ? { connection_id: via } : {}),
                input: { ...input, mutation: mutation(options) },
            },
            options,
            (value) => {
                const op = this.fromReply(value, kind, key);
                if (
                    canonicalJson((value as JsonObject).continuation) !==
                    canonicalJson(context)
                )
                    return protocol();
                return op;
            },
            key,
            true,
        );
    }
    [handoffSession](
        cid: ConversationId,
        sid: SessionId,
        input: SdkHandoffInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkHandoffResult>> {
        const captured = handoffInput(input);
        if (input.consultation !== undefined) {
            const ref = input.consultation?.reference,
                metadata = references.get(ref);
            if (
                !metadata ||
                kinds.get(metadata.kind)?.name !== "session_consult" ||
                ref.scope.conversationId !== cid ||
                ref.scope.sessionId !== sid ||
                ref.operationId === options?.operationId ||
                (metadata.saved as JsonObject).continuation === undefined ||
                ((metadata.saved as JsonObject).continuation as JsonObject)
                    .retained_connection_id !== captured.retain ||
                (options?.extensions &&
                    typeof options.extensions === "object" &&
                    Object.hasOwn(options.extensions, "consultation_record_id"))
            )
                invalid();
            captured.consultation = metadata.saved;
        }
        const context: JsonObject = {
            kind: "handoff",
            retained_connection_id: captured.retain,
            source_participant: captured.from_participant,
            target_participant: captured.to,
            candidate: captured.candidate,
            role: captured.role,
            outgoing: captured.outgoing,
        };
        return this[submitMovement](
            "session_handoff",
            cid,
            sid,
            captured.via as ConnectionId,
            captured,
            this.operationKinds.session_handoff,
            context,
            options,
        );
    }
    [moveConnection](
        cid: ConversationId,
        sid: SessionId,
        via: ConnectionId,
        input: SdkMoveInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMoveResult>> {
        const captured = moveInput(input, via),
            context: JsonObject = {
                kind: "replacement",
                retained_connection_id: captured.retain,
                target: captured.target,
            };
        return this[submitMovement](
            "connection_move",
            cid,
            sid,
            via,
            captured,
            this.operationKinds.connection_replace,
            context,
            options,
        );
    }
    [readSession](
        cid: ConversationId,
        sid: SessionId,
        options: CanonicalRequestOptions,
    ): Promise<SdkSessionSnapshot> {
        return this.helper.request(
            {
                type: "session",
                conversation_id: cid,
                session_id: sid,
                action: { type: "refresh" },
            },
            options,
            (value) => sessionSnapshot(value, cid, sid),
        );
    }
    [connectionContext](
        cid: ConversationId,
        sid: SessionId,
    ): ConnectionContext {
        return {
            cid,
            sid,
            kinds: this.operationKinds,
            move: (id, input, options) =>
                this[moveConnection](cid, sid, id, input, options),
            mutate: (id, action, kind, options) => {
                const key = makeKey(options?.operationId, kind.command, {
                    conversationId: cid,
                    sessionId: sid,
                    connectionId: id,
                });
                return this.helper.request(
                    {
                        type: "connection",
                        conversation_id: cid,
                        session_id: sid,
                        connection_id: id,
                        action: { ...action, mutation: mutation(options) },
                    },
                    options,
                    (value) => this.fromReply(value, kind, key),
                    key,
                    true,
                );
            },
            read: (id, action, options, decode) =>
                this.helper.request(
                    {
                        type: "connection",
                        conversation_id: cid,
                        session_id: sid,
                        connection_id: id,
                        action,
                    },
                    options,
                    decode,
                ),
        };
    }
    close(options: CanonicalRequestOptions): Promise<void> {
        return this.helper.close(options);
    }
}
export class SdkConversation {
    readonly sessions: SdkSessions;
    readonly participants;
    constructor(
        private readonly client: NodeClient,
        readonly id: ConversationId,
    ) {
        this.sessions = createSessions({
            cid: this.id,
            kinds: client.operationKinds,
            handoff: (sid, input, options) =>
                client[handoffSession](this.id, sid, input, options),
            connections: (sid) =>
                createConnections(client[connectionContext](this.id, sid)),
            mutate: (sid, action, kind, options, via) =>
                client[mutateSession](this.id, sid, action, kind, options, via),
            refresh: (sid, options) =>
                client[readSession](this.id, sid, options),
        });
        this.participants = Object.freeze({
            byId: (id: ParticipantId): SdkParticipant => {
                sdkId(id);
                return new SdkParticipant(client, this.id, id);
            },
            add: (
                input: {
                    readonly identityId: IdentityId;
                    readonly kind: "human" | "ai" | "system" | "external";
                    readonly role: string;
                    readonly expectedRevision: SdkRevision<"conversation">;
                },
                options: SdkMutationOptions,
            ): Promise<SdkOperation<SdkParticipant>> => {
                if (
                    !input ||
                    !identifier(input.identityId) ||
                    !["human", "ai", "system", "external"].includes(
                        input.kind,
                    ) ||
                    typeof input.role !== "string"
                )
                    invalid();
                sdkRevision<"conversation">(input.expectedRevision);
                return client[mutateParticipant](
                    this.id,
                    {
                        type: "participant_add",
                        identity_id: input.identityId,
                        kind: input.kind,
                        role: input.role,
                        expected_revision: input.expectedRevision,
                    },
                    client.operationKinds.conversation_participant_add,
                    options,
                );
            },
        });
        Object.freeze(this);
    }
    refresh(
        options: CanonicalRequestOptions,
    ): Promise<SdkConversationSnapshot> {
        return this.client[refreshConversation](this.id, options);
    }
    receipts(
        messageId: MessageId,
        options: CanonicalRequestOptions,
    ): Promise<readonly SdkDeliveryObservation[]> {
        return this.client[readReceipts](this.id, messageId, options);
    }
    send(
        input: Parameters<NodeClient[typeof sendMessage]>[1],
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMessageAcceptance>> {
        return this.client[sendMessage](this.id, input, options);
    }
}
export class SdkParticipant {
    constructor(
        private readonly client: NodeClient,
        readonly conversationId: ConversationId,
        readonly id: ParticipantId,
    ) {
        Object.freeze(this);
    }
    refresh(options: CanonicalRequestOptions): Promise<SdkParticipantSnapshot> {
        return this.client[readParticipant](
            this.conversationId,
            this.id,
            options,
        );
    }
    update(
        input: {
            readonly role: string;
            readonly expectedConversationRevision: SdkRevision<"conversation">;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkParticipant>> {
        if (!input || typeof input.role !== "string") invalid();
        sdkRevision<"conversation">(input.expectedConversationRevision);
        return this.client[mutateParticipant](
            this.conversationId,
            {
                type: "participant_update",
                participant_id: this.id,
                role: input.role,
                expected_revision: input.expectedConversationRevision,
            },
            this.client.operationKinds.conversation_participant_update,
            options,
        );
    }
    remove(
        input: {
            readonly expectedConversationRevision: SdkRevision<"conversation">;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkParticipant>> {
        if (!input) invalid();
        sdkRevision<"conversation">(input.expectedConversationRevision);
        return this.client[mutateParticipant](
            this.conversationId,
            {
                type: "participant_remove",
                participant_id: this.id,
                expected_revision: input.expectedConversationRevision,
            },
            this.client.operationKinds.conversation_participant_remove,
            options,
        );
    }
}
type OperationOwner = {
    readonly operationKinds: Pick<
        typeof operationKinds,
        "operation_cancel" | "operation_reconcile"
    >;
    readonly operations: {
        restore<T>(saved: Json, kind: SdkOperationKind<T>): SdkOperation<T>;
    };
    [runOperation]: NodeClient[typeof runOperation];
    [returnConsultation]: NodeClient[typeof returnConsultation];
};
export class SdkOperation<T> {
    #observedRecord?: string;
    private observe(value: Json): SdkOperationSnapshot {
        const observed = snapshot(value, this.reference);
        if (observed.recordId !== undefined) {
            if (
                this.#observedRecord !== undefined &&
                this.#observedRecord !== observed.recordId
            )
                return protocol();
            this.#observedRecord = observed.recordId;
        }
        return observed;
    }
    constructor(
        private readonly client: OperationOwner,
        readonly reference: SdkOperationReference<T>,
        private readonly kind: SdkOperationKind<T>,
        private readonly saved: Json,
        private readonly decode: (value: Json) => T,
    ) {
        const context = (saved as JsonObject).continuation;
        if (
            context &&
            typeof context === "object" &&
            !Array.isArray(context) &&
            typeof context.record_id === "string"
        )
            this.#observedRecord = context.record_id;
        Object.freeze(this);
    }
    returnToRetained(
        this: SdkOperation<SdkConsultationResult>,
        input: { readonly expectedTopologyRevision: SdkRevision<"topology"> },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        if (!input) invalid();
        return this.client[returnConsultation](
            this.saved,
            this.reference,
            input.expectedTopologyRevision,
            options,
        );
    }
    referenceJson(): Json {
        return copied(this.saved);
    }
    status(options: CanonicalRequestOptions): Promise<SdkOperationSnapshot> {
        return this.client[runOperation](
            this.saved,
            this.kind,
            { type: "status" },
            options,
            (value) => this.observe(value),
            this.reference,
        );
    }
    waitUntilReady(
        options: CanonicalRequestOptions,
    ): Promise<SdkOperationSnapshot> {
        return this.client[runOperation](
            this.saved,
            this.kind,
            { type: "wait_until_ready" },
            options,
            (value) => {
                const s = this.observe(value);
                if (!["ready", "committing", "completed"].includes(s.state))
                    return protocol();
                return s;
            },
            this.reference,
        );
    }
    wait(options: CanonicalRequestOptions): Promise<SdkCompletion<T>> {
        return this.client[runOperation](
            this.saved,
            this.kind,
            { type: "wait" },
            options,
            (value) => {
                if (!object(value, ["snapshot", "value"])) return protocol();
                const s = this.observe(value.snapshot);
                if (s.state !== "completed") return protocol();
                return Object.freeze({
                    snapshot: s,
                    value: this.decode(value.value),
                });
            },
            this.reference,
        );
    }
    cancel(
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkDecisionReceipt>> {
        return this.decide(
            { type: "cancel", mutation: mutation(options) },
            options,
            this.client.operationKinds.operation_cancel,
        );
    }
    reconcile(
        input: {
            readonly outcome: "completed" | "cancelled" | "failed";
            readonly evidenceId: string;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkDecisionReceipt>> {
        if (
            !input ||
            !["completed", "cancelled", "failed"].includes(input.outcome) ||
            !identifier(input.evidenceId)
        )
            invalid();
        return this.decide(
            {
                type: "reconcile",
                outcome: input.outcome,
                evidence_id: input.evidenceId,
                mutation: mutation(options),
            },
            options,
            this.client.operationKinds.operation_reconcile,
        );
    }
    private decide(
        action: JsonObject,
        options: SdkMutationOptions,
        kind: SdkOperationKind<SdkDecisionReceipt>,
    ): Promise<SdkOperation<SdkDecisionReceipt>> {
        const key = makeKey(options.operationId, kind.command, {
            ...(this.reference.scope.conversationId
                ? { conversationId: this.reference.scope.conversationId }
                : {}),
            ...(this.reference.scope.sessionId
                ? { sessionId: this.reference.scope.sessionId }
                : {}),
        });
        return this.client[runOperation](
            this.saved,
            this.kind,
            action,
            options,
            (value) => {
                const operation = this.client.operations.restore(value, kind);
                if (!equalKey(operation.reference, key)) return protocol();
                return operation;
            },
            key,
            true,
        );
    }
}

/** Internal trusted-backend operation owner; no provisioning on ParticipantClient. */
export function createAdministrativeOperations(
    helper: SdkHelper,
    validateReference: (value: Json) => { reference: JsonObject; kind: string },
) {
    const administrativeKinds = new Map<string, SdkOperationKind<Json>>();
    const select = (
        command: string,
        name: string,
    ): SdkOperationKind<Json> | SdkOperationKind<SdkDecisionReceipt> => {
        if (name === "operation_cancel" && command === "operation.cancel")
            return operationKinds.operation_cancel;
        if (name === "operation_reconcile" && command === "operation.reconcile")
            return operationKinds.operation_reconcile;
        if (name !== "administrative") return invalid();
        let token = administrativeKinds.get(command);
        if (!token) {
            token = kind("administrative", command, (value) =>
                freeze(copied(value)),
            );
            administrativeKinds.set(command, token);
        }
        return token;
    };
    function captured<T>(
        value: Json,
        expected: SdkOperationKind<T>,
    ): SdkOperation<T> {
        const saved = copied(value);
        if (
            Buffer.byteLength(canonicalJson(saved)) > 4096 ||
            !object(saved, ["schema", "operation_id", "command", "scope"]) ||
            saved.schema !== "connect-operation-reference/1" ||
            !identifier(saved.operation_id) ||
            saved.command !== expected.command ||
            !object(saved.scope, []) ||
            Object.keys(saved.scope).length
        )
            invalid();
        const key = makeKey(saved.operation_id, saved.command, {});
        const reference = freeze({ ...key }) as SdkOperationReference<T>;
        references.set(reference, {
            saved: freeze(saved),
            kind: expected as SdkOperationKind<unknown>,
        });
        const k = kinds.get(expected)!;
        const decode = (value: Json): T =>
            (k.name === "administrative"
                ? freeze(copied(value))
                : decision(value, undefined, key)) as T;
        return new SdkOperation(owner, reference, expected, saved, decode);
    }
    function restore<T = Json>(
        saved: Json,
        expected?: SdkOperationKind<T>,
    ): SdkOperation<T> {
        const checked = validateReference(saved);
        const selected = select(
            checked.reference.command as string,
            checked.kind,
        );
        if (expected && (selected as object) !== (expected as object))
            invalid();
        return captured(
            checked.reference,
            (expected ?? selected) as unknown as SdkOperationKind<T>,
        );
    }
    const owner: OperationOwner = {
        operationKinds,
        operations: { restore },
        [returnConsultation]() {
            throw new NodeSdkError("unsupported", "not-submitted");
        },
        [runOperation](
            saved,
            _kind,
            action,
            options,
            decode,
            key,
            mutating = false,
        ) {
            return helper.request(
                { type: "administrative_operation", reference: saved, action },
                options,
                decode,
                key,
                mutating,
            );
        },
    };
    return {
        restore: (saved: Json): SdkOperation<Json> => restore(saved),
        recover<T>(reference: SdkOperationReference<T>): SdkOperation<T> {
            const saved = references.get(reference),
                k = saved && kinds.get(saved.kind);
            if (
                !saved ||
                !k ||
                ![
                    "administrative",
                    "operation_cancel",
                    "operation_reconcile",
                ].includes(k.name)
            )
                invalid();
            return captured(saved.saved, saved.kind as SdkOperationKind<T>);
        },
        fromReply(value: Json, key: OperationKey): SdkOperation<Json> {
            const expected = select(
                key.command,
                "administrative",
            ) as SdkOperationKind<Json>;
            let operation: SdkOperation<Json>;
            try {
                operation = captured(value, expected);
            } catch {
                return protocol();
            }
            if (!equalKey(operation.reference, key)) return protocol();
            return operation;
        },
    };
}
