import type { ConversationId, MessageId, ParticipantId } from "./sdk_identity.js";
export interface BrowserApplicationReadOptions {
    readonly url: string;
    readonly transport: "http-json";
    readonly identityId: string;
    readonly deviceId: string;
    readonly browserOrigin: string;
    readonly credential: string;
    readonly expiresAt: number;
}
export interface BrowserApplicationReadContext {
    readonly signal: AbortSignal;
    readonly timeoutMs: number;
    readonly identityId: string;
    readonly deviceId: string;
    readonly physicalEpoch: number;
}
export type BrowserApplicationReadResolver = (context: BrowserApplicationReadContext) => Promise<BrowserApplicationReadOptions>;
export interface BrowserDeliveryObservation {
    readonly participantId: ParticipantId;
    readonly membershipEpoch: number;
    readonly channel: string;
    readonly deliveryId: string;
    readonly state: "queued" | "accepted" | "delivered" | "read" | "failed" | "unknown";
    readonly provenance: "journal" | "client_reported" | "provider_observation" | "local_attempt";
    readonly observedAt: number;
}
export declare function applicationReadOrigin(value: string): string;
/** Independently disposable HTTPS reads. No canonical credential or framing is reused. */
export declare class BrowserApplicationReads {
    #private;
    readonly resolve: BrowserApplicationReadResolver | undefined;
    readonly identityId: string;
    readonly deviceId: string;
    constructor(resolve: BrowserApplicationReadResolver | undefined, identityId: string, deviceId: string);
    retire(): void;
    receipts(cid: ConversationId, mid: MessageId, options: {
        timeoutMs: number;
        signal?: AbortSignal;
    }, physicalEpoch: number, current: () => void): Promise<readonly BrowserDeliveryObservation[]>;
}
