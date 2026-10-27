/** Explicit original identities; authority is rechecked by the signed service. */
export interface BrowserSessionSelection {
    conversationId: string;
    sessionId: string;
    connectionId: string;
    operationId: string;
    expectedRevision: number;
    participantId: string;
    audioStreamId: string;
    videoStreamId?: string;
}
/** Snapshot all identities/profile before capture; never generate replacement intent. */
export declare function browserOfferSelection(options: BrowserSessionSelection & {
    video?: boolean;
}): BrowserSessionSelection;
