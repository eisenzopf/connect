import { SdkHelper } from "./sdk_helper.js";
import { type Json } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
import type { ConversationId, OperationId, ParticipantId } from "./node_sdk.js";
import type { SessionId, SdkSession, SdkSessionMedium } from "./node_sessions.js";
/** Signed metadata hint. Refresh the Session before any separate decision. */
export interface SdkIncomingInvitation {
    readonly session: SdkSession;
    readonly fromParticipant: ParticipantId;
    readonly toParticipant: ParticipantId;
    readonly operationId: OperationId;
    readonly source: Readonly<{
        kind: "session.invite";
        correlationId: OperationId;
    }>;
    readonly medium: SdkSessionMedium;
    readonly intent: string;
    readonly capabilities: Json;
}
export interface SdkIncomingInvitations {
    next(options: CanonicalRequestOptions): Promise<SdkIncomingInvitation | undefined>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
export declare function createIncoming(helper: SdkHelper, session: (cid: ConversationId, sid: SessionId) => SdkSession): SdkIncomingInvitations;
