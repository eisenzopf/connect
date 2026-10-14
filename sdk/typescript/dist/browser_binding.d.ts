/** Explicit, nonsecret selection. Discovery and media admission each recheck authority. */
export interface BrowserSessionSelection {
    conversationId?: string;
    sessionId?: string;
}
export interface BrowserOfferSelection {
    conversation_id: string;
    session_id: string;
}
/** Snapshot paired identifiers before any capture; callers cannot retarget an in-flight offer. */
export declare function browserOfferSelection(options: BrowserSessionSelection, video: boolean): BrowserOfferSelection | undefined;
