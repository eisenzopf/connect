import { type JsonObject } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
import { type CanonicalRequestOptions, type CanonicalCommandScope } from "./canonical_validation.js";
export type { CanonicalRequestOptions, CanonicalCommandScope, } from "./canonical_validation.js";
/** Explicit finite connect/2 control. No credential refresh, retry, transport
 * fallback, native media ownership or live-event lease is implied by HTTP. */
export declare class CanonicalHttpClient {
    private readonly endpoint;
    private readonly credential;
    private readonly active;
    private closed;
    constructor(config: {
        readonly endpoint: string;
        readonly credential: string;
    });
    request(type: string, payload: JsonObject, scope: CanonicalCommandScope, options: CanonicalRequestOptions): Promise<readonly Envelope[]>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
