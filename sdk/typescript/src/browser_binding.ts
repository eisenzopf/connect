import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";

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
export function browserOfferSelection(
    options: BrowserSessionSelection & { video?: boolean },
): BrowserSessionSelection {
    const selection = {
        conversationId: options.conversationId,
        sessionId: options.sessionId,
        connectionId: options.connectionId,
        operationId: options.operationId,
        expectedRevision: options.expectedRevision,
        participantId: options.participantId,
        audioStreamId: options.audioStreamId,
        videoStreamId: options.videoStreamId,
    };
    if (
        ![
            selection.conversationId,
            selection.sessionId,
            selection.connectionId,
            selection.operationId,
            selection.participantId,
            selection.audioStreamId,
        ].every(identifier) ||
        !Number.isSafeInteger(selection.expectedRevision) ||
        selection.expectedRevision < 0 ||
        (options.video !== undefined && typeof options.video !== "boolean") ||
        (options.video === true
            ? !identifier(selection.videoStreamId) ||
              selection.videoStreamId === selection.audioStreamId
            : selection.videoStreamId !== undefined)
    )
        throw new ConnectError(
            "invalid-request",
            "Explicit bounded Conversation/Session/Connection/operation/participant/stream IDs, revision and matching media profile required",
        );
    return selection;
}
