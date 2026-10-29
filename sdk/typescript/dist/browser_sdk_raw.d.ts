import { type Json } from "./uctp_json.js";
import type { CanonicalErrorKind, Certainty } from "./canonical_error.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserSdkRequestOptions } from "./browser_sdk.js";
import type { Envelope, Scope } from "./uctp_wire.js";
declare const validated: unique symbol;
/** Fixed current schema only. Local validation authenticates no signature or grant. */
export interface BrowserValidatedEnvelope {
    readonly value: Json;
    readonly [validated]: "validated-current-canonical-envelope";
}
export interface BrowserRawCanonical {
    validate(value: Json): BrowserValidatedEnvelope;
    request(value: BrowserValidatedEnvelope, options: BrowserSdkRequestOptions): Promise<readonly BrowserValidatedEnvelope[]>;
}
interface Batch {
    readonly requestId: string;
    readonly frames: readonly Envelope[];
    check(): void;
    retire(): void;
}
interface Callbacks {
    error(kind: CanonicalErrorKind, certainty: Certainty, key?: BrowserOperationReference): Error;
    perform(value: Envelope, scope: Scope, options: BrowserSdkRequestOptions, key?: BrowserOperationReference): Promise<Batch>;
}
export declare function createBrowserRaw(callbacks: Callbacks): BrowserRawCanonical;
export {};
