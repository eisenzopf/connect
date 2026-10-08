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
    health(): Promise<import("./types.js").Health>;
}
