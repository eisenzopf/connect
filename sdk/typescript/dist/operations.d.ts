import type { Operation, Outcome } from "./types.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export declare abstract class Operations {
    abstract request(operation: Operation): Promise<Outcome>;
    private data;
    resolve(address: string): Promise<import("./types.js").Identity>;
    create(request_id: string, participants: string[]): Promise<import("./types.js").Conversation>;
    send(request_id: string, conversation_id: string, body: string): Promise<import("./types.js").Accepted>;
    history(conversation_id: string, after?: string | null, limit?: number): Promise<import("./types.js").History>;
    inbox(cursor?: string | null, limit?: number, wait_ms?: number): Promise<import("./types.js").Inbox>;
    requestStatus(request_id: string): Promise<import("./types.js").RequestStatus>;
    providerStatus(conversation_id: string): Promise<import("./types.js").ProviderStatus>;
    closeConversation(request_id: string, conversation_id: string): Promise<import("./types.js").Conversation>;
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
    artifactBegin(request_id: string, conversation_id: string, name: string, media_type: string, size: number, sha256: string, expires_at?: number | null): Promise<import("./types.js").Artifact>;
    artifactAppend(request_id: string, conversation_id: string, artifact_id: string, offset: number, data: string): Promise<import("./types.js").Artifact>;
    artifactFinish(request_id: string, conversation_id: string, artifact_id: string): Promise<import("./types.js").Artifact>;
    artifactGet(conversation_id: string, artifact_id: string, offset?: number, limit?: number): Promise<import("./types.js").ArtifactChunk>;
    artifactDelete(request_id: string, conversation_id: string, artifact_id: string): Promise<import("./types.js").Artifact>;
    health(): Promise<import("./types.js").Health>;
}
