import { ConnectError } from "./errors.js";
/** Snapshot paired identifiers before any capture; callers cannot retarget an in-flight offer. */
export function browserOfferSelection(options, video) {
    if (options.conversationId === undefined && options.sessionId === undefined)
        return undefined;
    const valid = (value) => typeof value === "string" &&
        value.length > 0 &&
        new TextEncoder().encode(value).length <= 128 &&
        !/[\s\u0000-\u001f\u007f-\u009f]/u.test(value);
    if (!video || !valid(options.conversationId) || !valid(options.sessionId))
        throw new ConnectError("invalid-request", "Select a conversation and accepted audio/video Session using bounded identifiers");
    return {
        conversation_id: options.conversationId,
        session_id: options.sessionId,
    };
}
