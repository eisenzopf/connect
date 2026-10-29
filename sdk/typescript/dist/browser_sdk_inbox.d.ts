import { type Envelope } from "./uctp_wire.js";
export interface BrowserInvitationHint {
    readonly frame: Envelope;
    readonly epoch: number;
}
/** Physical-local notification metadata. No authority, media or durable inbox. */
export declare class BrowserInvitationInbox {
    private queue;
    private queuedBytes;
    private seen;
    private seenBytes;
    clear(): void;
    peek(): BrowserInvitationHint | undefined;
    consume(hint: BrowserInvitationHint): boolean;
    record(frame: Envelope, epoch: number): boolean;
    private cancel;
}
