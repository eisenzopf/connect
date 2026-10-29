import { type Json } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
import type { BrowserConversationRecoveryPage } from "./browser_transport_client.js";
import type { ConversationId, SessionId, ConnectionId, ParticipantId, OperationId, SdkId } from "./sdk_identity.js";
import type { BrowserSdkRequestOptions } from "./browser_sdk.js";
export type BrowserCursor = SdkId<"cursor">;
export interface BrowserJournalEvent {
    readonly eventId: string;
    readonly cursor: BrowserCursor;
    readonly conversationId: ConversationId;
    readonly sessionId?: SessionId;
    readonly connectionId?: ConnectionId;
    readonly participantId?: ParticipantId;
    readonly operationId?: OperationId;
    readonly audience: readonly {
        readonly participantId: ParticipantId;
        readonly membershipEpoch: number;
    }[];
    readonly kind: string;
    readonly revision: number;
    readonly source: {
        readonly kind: string;
        readonly correlationId: string;
    };
    readonly observedAt: number;
    readonly data: Json;
}
export interface BrowserEventStream extends AsyncIterableIterator<BrowserJournalEvent> {
    next(): Promise<IteratorResult<BrowserJournalEvent, undefined>>;
    /** Establish the original authorized baseline without consuming a fact. */
    ready(): Promise<BrowserCursor>;
    close(): Promise<void>;
}
interface Context {
    check(): void;
    read(after?: BrowserCursor): Promise<BrowserConversationRecoveryPage>;
    reserve(retire: () => void): () => void;
    error(kind: "validation" | "cancelled" | "conflict" | "resync-required"): Error;
    translated(error: unknown): Error;
    bad(): never;
}
export declare function isBrowserCursor(v: unknown): v is BrowserCursor;
export declare function journalEvent(frame: Envelope, cid: ConversationId, bad: () => never): BrowserJournalEvent;
export declare function createBrowserEvents(cid: ConversationId, input: {
    readonly after?: BrowserCursor;
}, options: BrowserSdkRequestOptions, ctx: Context): BrowserEventStream;
export {};
