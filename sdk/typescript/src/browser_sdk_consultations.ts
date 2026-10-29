import { fields, nonEmpty } from "./browser_sdk_sessions.js";
import {
    canonicalJson,
    jsonObject,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserNonEmpty } from "./browser_sdk_streams.js";
import type { BrowserConnection, BrowserSession } from "./browser_sdk.js";
import type {
    ConnectionId,
    ParticipantId,
    SdkId,
    SdkRevision,
} from "./sdk_identity.js";

export type BrowserOperationRecordId = SdkId<"operation-record">;
/** Opaque persistence context; this is original intent, never an authority grant. */
export type BrowserConsultationContinuation =
    | {
          readonly kind: "consultation";
          readonly retained_connection_id: ConnectionId;
          readonly record_id?: BrowserOperationRecordId;
      }
    | {
          readonly kind: "consultation-return";
          readonly retained_connection_id: ConnectionId;
          readonly consultation_record_id: BrowserOperationRecordId;
      };
export interface ConsultationReference extends BrowserOperationReference {
    readonly continuation?: BrowserConsultationContinuation;
}
export interface BrowserConsultInput {
    readonly via: ConnectionId;
    readonly retain: ConnectionId;
    readonly with: BrowserNonEmpty<ParticipantId>;
    readonly hold: "silence" | "configured-media";
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface BrowserTopologyChange {
    readonly retained: readonly BrowserConnection[];
    readonly admitted: readonly BrowserConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
}
export interface BrowserConsultationResult {
    readonly session: BrowserSession;
    readonly topology: BrowserTopologyChange;
}
export interface BrowserConsultationReturnReceipt {
    readonly consultationRecordId: BrowserOperationRecordId;
    readonly topology: BrowserTopologyChange;
    readonly remoteContinuing: boolean;
}
export const consultationCommand = (command: string) =>
    ["session.consult", "session.consult.return"].includes(command);
const revision = (value: unknown) =>
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value < Number.MAX_SAFE_INTEGER;
export function consultationContinuation(
    value: Json | undefined,
    key: BrowserOperationReference,
    bad: () => never,
): BrowserConsultationContinuation | undefined {
    if (!consultationCommand(key.command)) {
        if (value !== undefined) bad();
        return undefined;
    }
    if (
        !value ||
        !jsonObject(value) ||
        !identifier(value.retained_connection_id) ||
        value.retained_connection_id === key.scope.connectionId
    )
        return bad();
    if (key.command === "session.consult") {
        fields(value, ["kind", "retained_connection_id", "record_id"], bad);
        if (
            value.kind !== "consultation" ||
            (value.record_id !== undefined && !identifier(value.record_id))
        )
            return bad();
    } else {
        fields(
            value,
            ["kind", "retained_connection_id", "consultation_record_id"],
            bad,
        );
        if (
            value.kind !== "consultation-return" ||
            !identifier(value.consultation_record_id)
        )
            return bad();
    }
    return value as unknown as BrowserConsultationContinuation;
}
export function consultBody(
    input: BrowserConsultInput,
    bad: () => never,
): JsonObject {
    fields(
        input,
        ["via", "retain", "with", "hold", "expectedTopologyRevision"],
        bad,
    );
    if (
        !identifier(input.via) ||
        !identifier(input.retain) ||
        input.via === input.retain ||
        !["silence", "configured-media"].includes(input.hold) ||
        !revision(input.expectedTopologyRevision)
    )
        return bad();
    const participants = nonEmpty(input.with, identifier, 100, bad).sort();
    if (
        participants.length < 2 ||
        new Set(participants).size !== participants.length
    )
        return bad();
    return {
        retained_connid: input.retain,
        private_participants: participants,
        hold_treatment: input.hold,
    };
}
export function consultationObservation(
    value: JsonObject,
    key: ConsultationReference,
    expected: number | undefined,
    bad: () => never,
): void {
    if (
        !identifier(value.record_id) ||
        value.command_type !== key.command ||
        !revision(value.expected_revision) ||
        (expected !== undefined && value.expected_revision !== expected) ||
        (key.continuation?.kind === "consultation" &&
            key.continuation.record_id !== undefined &&
            value.record_id !== key.continuation.record_id)
    )
        bad();
    if (value.state === "completed")
        consultationCompletion(value.result, key, bad);
}
export function consultationCompletion(
    value: Json,
    key: ConsultationReference,
    bad: () => never,
) {
    if (
        !jsonObject(value) ||
        new TextEncoder().encode(canonicalJson(value)).length > 32768
    )
        return bad();
    fields(
        value,
        [
            "retained_connection_ids",
            "admitted_connection_ids",
            "retired_connection_ids",
            "session_revision",
            "conversation_revision",
            "consultation_record_id",
            "remote_continuing",
        ],
        bad,
    );
    const lists = [
        value.retained_connection_ids,
        value.admitted_connection_ids,
        value.retired_connection_ids,
    ];
    if (
        lists.some(
            (list) =>
                !Array.isArray(list) || list.some((id) => !identifier(id)),
        )
    )
        return bad();
    const [retained, admitted, retired] = lists as string[][],
        all = [...retained, ...admitted, ...retired];
    if (
        !retained.length ||
        all.length > 100 ||
        new Set(all).size !== all.length ||
        !(
            typeof value.session_revision === "number" &&
            Number.isSafeInteger(value.session_revision) &&
            value.session_revision > 0
        ) ||
        value.session_revision === 0 ||
        (value.conversation_revision !== undefined &&
            value.conversation_revision !== null)
    )
        return bad();
    const context = key.continuation;
    if (!context) return bad();
    if (key.command === "session.consult") {
        if (
            context.kind !== "consultation" ||
            !admitted.length ||
            retired.length ||
            (value.consultation_record_id !== undefined &&
                value.consultation_record_id !== null) ||
            (value.remote_continuing !== undefined &&
                value.remote_continuing !== null) ||
            !retained.includes(context.retained_connection_id)
        )
            return bad();
    } else if (
        key.command !== "session.consult.return" ||
        context.kind !== "consultation-return" ||
        value.consultation_record_id !== context.consultation_record_id ||
        typeof value.remote_continuing !== "boolean" ||
        admitted.length ||
        (!retained.includes(context.retained_connection_id) &&
            !(
                value.remote_continuing === false &&
                retired.includes(context.retained_connection_id)
            ))
    )
        return bad();
    return {
        retained: retained as ConnectionId[],
        admitted: admitted as ConnectionId[],
        retired: retired as ConnectionId[],
        topologyRevision: value.session_revision as SdkRevision<"topology">,
        ...(key.command === "session.consult.return"
            ? {
                  consultationRecordId:
                      value.consultation_record_id as BrowserOperationRecordId,
                  remoteContinuing: value.remote_continuing as boolean,
              }
            : {}),
    };
}
