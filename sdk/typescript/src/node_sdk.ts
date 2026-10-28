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
    readonly revision: number;
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
type KindName =
    | "conversation_create"
    | "message_send"
    | "operation_cancel"
    | "operation_reconcile";
type Kind<T> = {
    name: KindName;
    command: string;
    decode: (value: Json, client: NodeClient, key: OperationKey) => T;
};
const runOperation = Symbol("operation");
const sendMessage = Symbol("send");
const refreshConversation = Symbol("refresh");
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
        !states.has(String(value.state)) ||
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
    _client: NodeClient,
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
        !states.has(String(value.observedState)) ||
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
const operationKinds = Object.freeze({
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
 * invitations, administration, events and remote HTTP remain under development.
 * One fixed SDK configuration, one Rust owner, no transparent restart/retry. */
export class NodeClient {
    readonly role = "node" as const;
    readonly operationKinds = operationKinds;
    readonly conversations;
    readonly operations;
    private readonly helper: SdkHelper;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    }) {
        this.helper = new SdkHelper(config);
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
        if (!k) invalid();
        let copy: Json;
        try {
            const text = canonicalJson(saved);
            if (Buffer.byteLength(text) > 4096) invalid();
            copy = copied(saved);
        } catch {
            return invalid();
        }
        if (
            !object(copy, ["schema", "operation_id", "command", "scope"]) ||
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
                : k.name === "message_send"
                  ? !s.conversation_id || !!s.session_id || !!s.connection_id
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
        return new SdkOperation(this, reference, expected, copy);
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
                        !["human", "ai", "system", "external"].includes(
                            String(p.kind),
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
    close(options: CanonicalRequestOptions): Promise<void> {
        return this.helper.close(options);
    }
}
export class SdkConversation {
    constructor(
        private readonly client: NodeClient,
        readonly id: ConversationId,
    ) {
        Object.freeze(this);
    }
    refresh(
        options: CanonicalRequestOptions,
    ): Promise<SdkConversationSnapshot> {
        return this.client[refreshConversation](this.id, options);
    }
    send(
        input: Parameters<NodeClient[typeof sendMessage]>[1],
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMessageAcceptance>> {
        return this.client[sendMessage](this.id, input, options);
    }
}
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
        private readonly client: NodeClient,
        readonly reference: SdkOperationReference<T>,
        private readonly kind: SdkOperationKind<T>,
        private readonly saved: Json,
    ) {
        Object.freeze(this);
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
                const k = kinds.get(this.kind)!;
                return Object.freeze({
                    snapshot: s,
                    value: k.decode(
                        value.value,
                        this.client,
                        this.reference,
                    ) as T,
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
