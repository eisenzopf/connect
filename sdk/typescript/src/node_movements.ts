import { sdkRevision } from "./node_sdk.js";
import type {
    SdkId,
    SdkRevision,
    ParticipantId,
    SdkConsultationOperation,
    SdkOperationKey,
} from "./node_sdk.js";
import type { ConnectionId, SdkConnection } from "./node_connections.js";
import { NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
import type { Json, JsonObject } from "./uctp_json.js";

export type EndpointId = SdkId<"endpoint">;
export type DeviceId = SdkId<"device">;
export interface SdkTopologyChange {
    readonly retained: readonly SdkConnection[];
    readonly admitted: readonly SdkConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
    readonly conversationRevision?: SdkRevision<"conversation">;
}
export type SdkHandoffRole =
    | { readonly scope: "session" }
    | {
          readonly scope: "conversation";
          readonly expectedConversationRevision: SdkRevision<"conversation">;
      };
export interface SdkHandoffInput {
    readonly via: ConnectionId;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    readonly retain: ConnectionId;
    readonly candidate?: ConnectionId;
    readonly outgoing: "leave" | "observe";
    readonly role: SdkHandoffRole;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
    readonly consultation?: SdkConsultationOperation;
}
export interface SdkHandoffResult extends SdkTopologyChange {
    /** Captured host result, not current role authority or native custody. */
    readonly roleChange?: {
        readonly participantRevision: number;
        readonly roleScope: "session" | "conversation";
        readonly outgoing: "leave" | "observe";
        readonly sourceParticipant: ParticipantId;
        readonly targetParticipant: ParticipantId;
        readonly cleanup: "pending" | "not_requested";
    };
}
export type SdkMoveTarget =
    | { readonly endpoint: EndpointId; readonly candidate?: never }
    | { readonly candidate: ConnectionId; readonly endpoint?: never };
export interface SdkMoveInput {
    readonly retain: ConnectionId;
    readonly target: SdkMoveTarget;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface SdkMoveResult extends SdkTopologyChange {
    readonly connection: SdkConnection;
    /** Captured binding observation; metadata handles acquire no media ownership. */
    readonly binding: {
        readonly generation: number;
        readonly participantId: ParticipantId;
        readonly membershipEpoch: number;
        readonly endpointId: EndpointId;
        readonly deviceId: DeviceId;
    };
}
const natural = (v: unknown): v is number =>
    Number.isSafeInteger(v) && Number(v) >= 0;
const positive = (v: unknown): v is number => natural(v) && v > 0;
function invalid(): never {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol(): never {
    throw new Error("Invalid typed movement result");
}
const roleValid = (v: Json): boolean =>
    object(v, ["scope", "expected_conversation_revision"]) &&
    (v.scope === "session"
        ? Object.keys(v).length === 1
        : v.scope === "conversation" &&
          Object.keys(v).length === 2 &&
          natural(v.expected_conversation_revision) &&
          v.expected_conversation_revision < Number.MAX_SAFE_INTEGER);
export function handoffInput(v: SdkHandoffInput): JsonObject {
    if (
        !v ||
        !identifier(v.via) ||
        !identifier(v.retain) ||
        v.via === v.retain ||
        !identifier(v.from) ||
        !identifier(v.to) ||
        v.from === v.to ||
        !["leave", "observe"].includes(v.outgoing) ||
        !v.role
    )
        invalid();
    let role: JsonObject;
    if (
        v.role.scope === "session" &&
        object(v.role, ["scope"]) &&
        Object.keys(v.role).length === 1
    )
        role = { scope: "session" };
    else if (
        v.role.scope === "conversation" &&
        object(v.role, ["scope", "expectedConversationRevision"]) &&
        Object.keys(v.role).length === 2 &&
        natural(v.role.expectedConversationRevision) &&
        v.role.expectedConversationRevision < Number.MAX_SAFE_INTEGER
    )
        role = {
            scope: "conversation",
            expected_conversation_revision: v.role.expectedConversationRevision,
        };
    else return invalid();
    if (
        v.candidate !== undefined &&
        (!identifier(v.candidate) ||
            v.candidate === v.via ||
            v.candidate === v.retain)
    )
        invalid();
    return {
        via: v.via,
        retain: v.retain,
        from_participant: v.from,
        to: v.to,
        outgoing: v.outgoing,
        role,
        candidate: v.candidate ?? null,
        expected_revision: sdkRevision<"topology">(v.expectedTopologyRevision),
    };
}
export function moveInput(v: SdkMoveInput, via: ConnectionId): JsonObject {
    if (
        !v ||
        !identifier(v.retain) ||
        v.retain === via ||
        !object(v.target, ["endpoint", "candidate"]) ||
        Object.keys(v.target).length !== 1
    )
        invalid();
    const endpoint = "endpoint" in v.target,
        id = endpoint ? v.target.endpoint : v.target.candidate;
    if (!identifier(id) || (!endpoint && (id === via || id === v.retain)))
        invalid();
    return {
        retain: v.retain,
        target: { kind: endpoint ? "endpoint" : "candidate", id },
        expected_revision: sdkRevision<"topology">(v.expectedTopologyRevision),
    };
}
export function movementContext(v: Json, command: string, via: Json): boolean {
    if (
        !identifier(via) ||
        !object(
            v,
            command === "session.handoff"
                ? [
                      "kind",
                      "retained_connection_id",
                      "source_participant",
                      "target_participant",
                      "candidate",
                      "role",
                      "outgoing",
                  ]
                : ["kind", "retained_connection_id", "target"],
        ) ||
        !identifier(v.retained_connection_id) ||
        v.retained_connection_id === via
    )
        return false;
    if (command === "session.handoff")
        return (
            v.kind === "handoff" &&
            identifier(v.source_participant) &&
            identifier(v.target_participant) &&
            v.source_participant !== v.target_participant &&
            (v.candidate == null ||
                (identifier(v.candidate) &&
                    v.candidate !== via &&
                    v.candidate !== v.retained_connection_id)) &&
            roleValid(v.role) &&
            (v.outgoing === "leave" || v.outgoing === "observe")
        );
    return (
        command === "connection.replace" &&
        v.kind === "replacement" &&
        object(v.target, ["kind", "id"]) &&
        Object.keys(v.target).length === 2 &&
        identifier(v.target.id) &&
        (v.target.kind === "endpoint" ||
            (v.target.kind === "candidate" &&
                v.target.id !== via &&
                v.target.id !== v.retained_connection_id))
    );
}
interface Context {
    readonly key: SdkOperationKey;
    connection(id: ConnectionId): SdkConnection;
}
function original(saved: Json): JsonObject {
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
            "source_participant",
            "target_participant",
            "candidate",
            "role",
            "outgoing",
            "target",
        ])
    )
        return protocol();
    return saved.continuation;
}
function topology(v: JsonObject, ctx: Context): SdkTopologyChange {
    if (
        !Array.isArray(v.retained) ||
        !v.retained.length ||
        !Array.isArray(v.admitted) ||
        v.admitted.length !== 1 ||
        !Array.isArray(v.retired) ||
        v.retained.length + v.admitted.length + v.retired.length > 100 ||
        !positive(v.topologyRevision) ||
        (v.conversationRevision !== null && !positive(v.conversationRevision))
    )
        return protocol();
    const seen = new Set<string>();
    const id = (value: Json): ConnectionId => {
        if (!identifier(value) || seen.has(value)) return protocol();
        seen.add(value);
        return value as ConnectionId;
    };
    const connection = (value: Json): SdkConnection => {
        if (
            !object(value, [
                "id",
                "conversationId",
                "sessionId",
                "receiptStreamRevision",
            ]) ||
            value.conversationId !== ctx.key.scope.conversationId ||
            value.sessionId !== ctx.key.scope.sessionId ||
            value.receiptStreamRevision !== null
        )
            return protocol();
        return ctx.connection(id(value.id));
    };
    const retained = Object.freeze(v.retained.map(connection)),
        admitted = Object.freeze(v.admitted.map(connection)),
        retired = Object.freeze(v.retired.map(id));
    return Object.freeze({
        retained,
        admitted,
        retired,
        topologyRevision: sdkRevision<"topology">(v.topologyRevision),
        ...(v.conversationRevision === null
            ? {}
            : {
                  conversationRevision: sdkRevision<"conversation">(
                      v.conversationRevision,
                  ),
              }),
    });
}
export function handoffResult(
    value: Json,
    ctx: Context,
    saved: Json,
): SdkHandoffResult {
    if (
        !object(value, [
            "retained",
            "admitted",
            "retired",
            "topologyRevision",
            "conversationRevision",
            "roleChange",
        ])
    )
        return protocol();
    const c = original(saved),
        t = topology(value, ctx),
        role = c.role as JsonObject;
    if (
        !t.retained.some((v) => v.id === c.retained_connection_id) ||
        (c.candidate != null && t.admitted[0].id !== c.candidate) ||
        (c.outgoing === "leave") !==
            t.retired.includes(ctx.key.scope.connectionId!) ||
        (role.scope === "session"
            ? t.conversationRevision !== undefined
            : t.conversationRevision !==
              Number(role.expected_conversation_revision) + 1)
    )
        return protocol();
    let roleChange: SdkHandoffResult["roleChange"];
    if (value.roleChange !== null) {
        const r = value.roleChange;
        if (
            !object(r, [
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
            r.cleanup !== (c.outgoing === "leave" ? "pending" : "not_requested")
        )
            return protocol();
        roleChange = Object.freeze({ ...r }) as unknown as NonNullable<
            SdkHandoffResult["roleChange"]
        >;
    }
    return Object.freeze({
        ...t,
        ...(roleChange === undefined ? {} : { roleChange }),
    });
}
export function moveResult(
    value: Json,
    ctx: Context,
    saved: Json,
): SdkMoveResult {
    if (
        !object(value, [
            "retained",
            "admitted",
            "retired",
            "topologyRevision",
            "conversationRevision",
            "connection",
            "binding",
        ])
    )
        return protocol();
    const c = original(saved),
        target = c.target as JsonObject,
        t = topology(value, ctx),
        binding = value.binding,
        connection = value.connection;
    if (
        t.conversationRevision !== undefined ||
        !t.retained.some((v) => v.id === c.retained_connection_id) ||
        !t.retired.includes(ctx.key.scope.connectionId!) ||
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
            : connection.id !== target.id)
    )
        return protocol();
    return Object.freeze({
        ...t,
        connection: t.admitted[0],
        binding: Object.freeze({
            ...binding,
        }) as unknown as SdkMoveResult["binding"],
    });
}
