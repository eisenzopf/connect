import { SdkHelper } from "./sdk_helper.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
import type { ConversationId, ParticipantId, OperationId, SdkId } from "./node_sdk.js";
declare const cursorBrand: unique symbol;
export type SdkCursor = string & {
    readonly [cursorBrand]: true;
};
export type SdkEventData = null | boolean | number | string | readonly SdkEventData[] | {
    readonly [key: string]: SdkEventData;
};
export interface SdkJournalEvent {
    readonly eventId: string;
    readonly cursor: SdkCursor;
    readonly conversationId: ConversationId;
    readonly sessionId?: SdkId<"session">;
    readonly connectionId?: SdkId<"connection">;
    readonly operationId?: OperationId;
    readonly participantId?: ParticipantId;
    readonly audience: readonly Readonly<{
        participantId: ParticipantId;
        membershipEpoch: number;
    }>[];
    readonly kind: string;
    readonly revision: number;
    readonly source: Readonly<{
        kind: string;
        correlationId: string;
    }>;
    readonly observedAt: number;
    readonly data: SdkEventData;
}
export interface SdkEventStream extends AsyncIterableIterator<SdkJournalEvent> {
    next(): Promise<IteratorResult<SdkJournalEvent, undefined>>;
    /** Complete the original authorized bootstrap/subscription; no fact is consumed. */
    ready(): Promise<SdkCursor>;
    close(): Promise<void>;
}
export declare function sdkCursor(value: string): SdkCursor;
export declare function createEvents(helper: SdkHelper, cid: ConversationId, input: {
    readonly after?: SdkCursor;
}, options: CanonicalRequestOptions): SdkEventStream;
export {};
