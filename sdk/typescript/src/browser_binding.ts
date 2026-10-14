import { ConnectError } from "./errors.js";

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
export function browserOfferSelection(
    options: BrowserSessionSelection,
    video: boolean,
): BrowserOfferSelection | undefined {
    if (options.conversationId === undefined && options.sessionId === undefined)
        return undefined;
    const valid = (value: unknown): value is string =>
        typeof value === "string" &&
        value.length > 0 &&
        new TextEncoder().encode(value).length <= 128 &&
        !/[\s\u0000-\u001f\u007f-\u009f]/u.test(value);
    if (!video || !valid(options.conversationId) || !valid(options.sessionId))
        throw new ConnectError(
            "invalid-request",
            "Select a conversation and accepted audio/video Session using bounded identifiers",
        );
    return {
        conversation_id: options.conversationId,
        session_id: options.sessionId,
    };
}
