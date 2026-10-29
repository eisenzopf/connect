import { type Json } from "./uctp_json.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserParticipantSnapshot } from "./browser_sdk.js";
import type { ConversationId, SdkRevision } from "./sdk_identity.js";
export type BrowserIdentityKind = "human" | "ai" | "system" | "external";
export declare const identityKind: (v: unknown) => v is BrowserIdentityKind;
export declare function membershipRevision(v: unknown, bad: () => never): SdkRevision<"conversation">;
export declare function membershipRole(v: unknown, bad: () => never): string;
export declare function participantView(v: Json, cid: ConversationId, bad: () => never): BrowserParticipantSnapshot;
export declare function membershipResult(v: Json, key: BrowserOperationReference, bad: () => never): {
    readonly snapshot: BrowserParticipantSnapshot;
    readonly revision: SdkRevision<"conversation">;
};
