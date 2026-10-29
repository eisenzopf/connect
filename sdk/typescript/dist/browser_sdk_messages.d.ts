import type { MessageId, ParticipantId } from "./sdk_identity.js";
import type { BrowserNonEmpty } from "./browser_sdk_streams.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
export interface BrowserMessageAcceptance {
    readonly messageId: MessageId;
    readonly operation: BrowserOperationReference;
    readonly boundary: "local-mutation";
}
export declare function messageBody(input: {
    readonly messageId: MessageId;
    readonly to: BrowserNonEmpty<ParticipantId>;
    readonly text: string;
}, bad: () => never): JsonObject;
export declare function messageResult(value: Json, key: BrowserOperationReference, bad: () => never): BrowserMessageAcceptance;
