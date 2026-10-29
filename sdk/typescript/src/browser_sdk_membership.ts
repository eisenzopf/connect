import { fields } from "./browser_sdk_sessions.js";
import { canonicalJson, jsonObject, type Json } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserParticipantSnapshot } from "./browser_sdk.js";
import type {
    ConversationId,
    ParticipantId,
    IdentityId,
    SdkRevision,
} from "./sdk_identity.js";
export type BrowserIdentityKind = "human" | "ai" | "system" | "external";
const text = (v: unknown, max: number, empty = false): v is string =>
    typeof v === "string" &&
    (empty || !!v) &&
    new TextEncoder().encode(v).length <= max &&
    !/[\x00-\x1f\x7f-\x9f]/u.test(v);
export const identityKind = (v: unknown): v is BrowserIdentityKind =>
    typeof v === "string" && ["human", "ai", "system", "external"].includes(v);
export function membershipRevision(
    v: unknown,
    bad: () => never,
): SdkRevision<"conversation"> {
    if (
        typeof v !== "number" ||
        !Number.isSafeInteger(v) ||
        v < 0 ||
        v >= Number.MAX_SAFE_INTEGER
    )
        return bad();
    return v as SdkRevision<"conversation">;
}
export function membershipRole(v: unknown, bad: () => never): string {
    if (!text(v, 64)) return bad();
    try {
        canonicalJson(v);
    } catch {
        return bad();
    }
    return v;
}
export function participantView(
    v: Json,
    cid: ConversationId,
    bad: () => never,
): BrowserParticipantSnapshot {
    if (
        !jsonObject(v) ||
        v.conversation_id !== cid ||
        !identifier(v.participant_id) ||
        !identifier(v.identity_id) ||
        !identityKind(v.kind) ||
        !text(v.role, 64) ||
        typeof v.membership_epoch !== "number" ||
        !Number.isSafeInteger(v.membership_epoch) ||
        v.membership_epoch < 1 ||
        typeof v.active !== "boolean" ||
        !(v.removal_reason === null || text(v.removal_reason, 512, true))
    )
        return bad();
    fields(
        v,
        [
            "participant_id",
            "conversation_id",
            "identity_id",
            "kind",
            "role",
            "membership_epoch",
            "active",
            "removal_reason",
        ],
        bad,
    );
    try {
        canonicalJson(v);
    } catch {
        return bad();
    }
    return Object.freeze({
        id: v.participant_id as ParticipantId,
        conversationId: cid,
        identityId: v.identity_id as IdentityId,
        kind: v.kind,
        role: v.role,
        membershipEpoch: v.membership_epoch,
        active: v.active,
        removalReason: v.removal_reason,
    });
}
export function membershipResult(
    v: Json,
    key: BrowserOperationReference,
    bad: () => never,
): {
    readonly snapshot: BrowserParticipantSnapshot;
    readonly revision: SdkRevision<"conversation">;
} {
    if (
        !jsonObject(v) ||
        v.kind !== "membership" ||
        !jsonObject(v.data) ||
        v.data.kind !== "participant" ||
        !jsonObject(v.data.receipt)
    )
        return bad();
    const d = v.data,
        r = d.receipt;
    if (
        !jsonObject(r) ||
        r.operation_id !== key.operationId ||
        r.conversation_id !== key.scope.conversationId ||
        r.state !== "completed" ||
        r.completion_boundary !== "local-mutation" ||
        !jsonObject(r.status_reference) ||
        r.status_reference.action !== "get" ||
        r.status_reference.operation_id !== key.operationId
    )
        return bad();
    // A returned revision can be MAX_SAFE_INTEGER; only the *next* expected
    // revision must leave room for the Store's increment.
    if (
        typeof d.revision !== "number" ||
        !Number.isSafeInteger(d.revision) ||
        d.revision < 0
    )
        return bad();
    return Object.freeze({
        snapshot: participantView(
            d.participant,
            key.scope.conversationId as ConversationId,
            bad,
        ),
        revision: d.revision as SdkRevision<"conversation">,
    });
}
