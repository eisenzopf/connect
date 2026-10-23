import type { SnapshotQuery, EventQuery, ObservationQuery } from "./events.js";
import { TaskOperations } from "./task_operations.js";
import type { MembershipRequest, ArtifactLink, ConversationCreate, ConversationClose, ConversationListQuery, TransitionRequest, OperationAction, MessageAudience, MessageDraft, Operation, Outcome } from "./types.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export declare abstract class Operations extends TaskOperations {
    abstract request(operation: Operation): Promise<Outcome>;
    private data;
    conversationSnapshot(conversation_id: string, query?: SnapshotQuery): Promise<import("./events.js").SnapshotPage>;
    conversationEvents(conversation_id: string, query?: EventQuery): Promise<import("./events.js").EventPage>;
    sessionObservations(conversation_id: string, session_id: string, query?: ObservationQuery): Promise<import("./events.js").ObservationPage>;
    admitTransition(request: TransitionRequest): Promise<import("./transitions.js").OperationRecord>;
    operationAction(request: OperationAction): Promise<import("./transitions.js").OperationRecord>;
    contactPairingStart(request_id: string, peer_identity_id: string, expires_at: number): Promise<import("./contacts.js").ContactPairing>;
    contactPairingComplete(request_id: string, pairing_id: string, challenge: string): Promise<import("./contacts.js").ContactPairing>;
    contactPairingRevoke(request_id: string, pairing_id: string): Promise<import("./contacts.js").ContactPairing>;
    contactState(peer_identity_id: string): Promise<import("./contacts.js").ContactState>;
    capabilities(address: string, conversation_id?: string | null): Promise<import("./types.js").Capabilities>;
    browserBinding(conversation_id?: string, session_id?: string): Promise<import("./types.js").BrowserBinding>;
    contextDeliveries(conversation_id: string, before?: string | null, limit?: number): Promise<import("./types.js").ContextDeliveryPage>;
    resolve(address: string): Promise<import("./types.js").Identity>;
    create(request_id: string, participants: string[]): Promise<import("./conversations.js").Conversation>;
    createConversation(request_id: string, request: ConversationCreate): Promise<import("./conversations.js").Conversation>;
    listConversations(query?: ConversationListQuery): Promise<import("./conversations.js").ConversationPage>;
    changeMembership(request: MembershipRequest): Promise<import("./membership.js").MembershipOutcome>;
    membershipSnapshot(conversation_id: string): Promise<import("./membership.js").MembershipSnapshot>;
    membershipInvitations(after?: string | null, limit?: number): Promise<import("./membership.js").MembershipInvitations>;
    /** Plain UTF-8 text to all current members; audience is frozen on acceptance. */
    send(request_id: string, conversation_id: string, body: string): Promise<{
        request_id: string;
        message_id: string;
        conversation_id: string;
        receipt: import("./types.js").MutationReceipt;
    }>;
    sendMessage(request_id: string, conversation_id: string, message: MessageDraft): Promise<{
        request_id: string;
        message_id: string;
        conversation_id: string;
        receipt: import("./types.js").MutationReceipt;
    }>;
    messageReceipts(conversation_id: string, message_id: string, after?: string | null, limit?: number): Promise<import("./types.js").ReceiptPage>;
    messageReceipt(request_id: string, conversation_id: string, message_id: string, state: "delivered" | "read"): Promise<import("./types.js").ReceiptAcknowledgement>;
    receiptPreferences(conversation_id: string): Promise<import("./types.js").ReceiptPreferences>;
    setReceiptPreferences(request_id: string, conversation_id: string, delivery: boolean, read: boolean): Promise<import("./types.js").ReceiptPreferences>;
    history(conversation_id: string, after?: string | null, limit?: number): Promise<import("./types.js").History>;
    inbox(cursor?: string | null, limit?: number, wait_ms?: number): Promise<import("./types.js").Inbox>;
    requestStatus(request_id: string): Promise<import("./types.js").RequestStatus>;
    providerStatus(conversation_id: string): Promise<import("./types.js").ProviderStatus>;
    closeConversation(request_id: string, conversation_id: string, options?: ConversationClose): Promise<import("./conversations.js").Conversation>;
    callStart(request_id: string, conversation_id: string, route_id: string, destination: string): Promise<import("./types.js").Call>;
    callStatus(conversation_id: string, call_id: string): Promise<import("./types.js").Call>;
    callEnd(request_id: string, conversation_id: string, call_id: string): Promise<import("./types.js").Call>;
    sessionInvite(request_id: string, conversation_id: string, participant_id: string, media: string[]): Promise<import("./types.js").Session>;
    sessionAccept(request_id: string, conversation_id: string, session_id: string): Promise<import("./types.js").Session>;
    sessionReject(request_id: string, conversation_id: string, session_id: string, reason?: string | null): Promise<import("./types.js").Session>;
    sessionCancel(request_id: string, conversation_id: string, session_id: string): Promise<import("./types.js").Session>;
    sessionEnd(request_id: string, conversation_id: string, session_id: string, reason?: string | null): Promise<import("./types.js").Session>;
    sessionGet(conversation_id: string, session_id: string): Promise<import("./types.js").Session>;
    sipDial(request_id: string, conversation_id: string, session_id: string, destination: string): Promise<import("./types.js").SipLeg>;
    sipStatus(conversation_id: string, sip_leg_id: string): Promise<import("./types.js").SipLeg>;
    sipEnd(request_id: string, conversation_id: string, sip_leg_id: string): Promise<import("./types.js").SipLeg>;
    contextView(conversation_id: string, limit?: number): Promise<import("./types.js").ContextView>;
    availabilitySet(request_id: string, state: "available" | "unavailable", expires_at: number, preferred_route?: string | null): Promise<import("./types.js").Availability>;
    availabilityGet(identity_id: string): Promise<import("./types.js").Availability>;
    artifactBegin(request_id: string, conversation_id: string, name: string, media_type: string, size: number, sha256: string, expires_at?: number | null, to?: MessageAudience): Promise<import("./types.js").Artifact>;
    artifactInspect(conversation_id: string, artifact_id: string): Promise<import("./types.js").Artifact>;
    artifactUploadLink(conversation_id: string, artifact_id: string, expires_in?: number): Promise<ArtifactLink>;
    artifactDownloadLink(conversation_id: string, artifact_id: string, message_id: string, expires_in?: number): Promise<ArtifactLink>;
    artifactLinkGet(link: ArtifactLink, offset?: number, limit?: number): Promise<import("./types.js").ArtifactChunk>;
    /** data is base64, and request_id remains stable when refreshing a link. */
    artifactLinkAppend(request_id: string, link: ArtifactLink, offset: number, data: string): Promise<import("./types.js").Artifact>;
    artifactAppend(request_id: string, conversation_id: string, artifact_id: string, offset: number, data: string): Promise<import("./types.js").Artifact>;
    artifactFinish(request_id: string, conversation_id: string, artifact_id: string): Promise<import("./types.js").Artifact>;
    artifactGet(conversation_id: string, artifact_id: string, offset?: number, limit?: number): Promise<import("./types.js").ArtifactChunk>;
    artifactDelete(request_id: string, conversation_id: string, artifact_id: string): Promise<import("./types.js").Artifact>;
    health(): Promise<import("./types.js").Health>;
}
