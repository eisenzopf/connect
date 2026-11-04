import { type Envelope } from "./uctp_wire.js";
import { type JsonObject } from "./uctp_json.js";
/** Original movement identity, supplied by the initiating controller. This
 * descriptor conveys no authority; every phase uses the target's signed peer. */
export interface BrowserReplacementRoot {
    readonly recordId: string;
    readonly operationId: string;
    readonly conversationId: string;
    readonly sessionId: string;
    readonly sourceConnectionId: string;
    readonly retainedConnectionId: string;
    readonly targetConnectionId: string;
    readonly endpointId: string;
    readonly deviceId: string;
}
export interface BrowserReplacementReceipt {
    readonly generation: number;
    readonly streamRevision: number;
    readonly recordId: string;
}
export declare function replacementRoot(input: BrowserReplacementRoot): BrowserReplacementRoot;
/** Validate signed original-root observations before allocation and on every
 * continuation. A preparing record is not a completed move or a media grant. */
export declare function replacementObservation(root: BrowserReplacementRoot, value: JsonObject): void;
export declare function replacementFrame(root: BrowserReplacementRoot, frames: Envelope[], connectionId: string): JsonObject;
export declare function replacementReceipt(root: BrowserReplacementRoot, value: JsonObject): BrowserReplacementReceipt;
