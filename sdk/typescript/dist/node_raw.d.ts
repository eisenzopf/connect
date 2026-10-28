import { type CanonicalRequestOptions } from "./canonical_validation.js";
import { SdkHelper } from "./sdk_helper.js";
import { type Json } from "./uctp_json.js";
declare const identity: unique symbol;
/** Local schema validation grants neither authentication nor media custody. */
export interface SdkValidatedEnvelope {
    readonly value: Json;
    readonly [identity]: "validated-current-canonical-envelope";
}
export interface SdkRawCanonical {
    /** Synchronous frozen Rust schema check, bounded to two seconds, without configuration or network. */
    validate(value: Json): SdkValidatedEnvelope;
    /** Same owned Rust SDK peer; fresh transport metadata, original logical intent, no retry. */
    request(envelope: SdkValidatedEnvelope, options: CanonicalRequestOptions): Promise<readonly SdkValidatedEnvelope[]>;
}
export declare function createRaw(helper: SdkHelper, executable: string): SdkRawCanonical;
export {};
