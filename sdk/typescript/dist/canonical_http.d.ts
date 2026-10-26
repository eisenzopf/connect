import { type JsonObject } from "./uctp_json.js";
import { type Envelope, type Scope } from "./uctp_wire.js";
export interface CanonicalRequestOptions {
    readonly timeoutMs: number;
    readonly signal?: AbortSignal;
}
export type CanonicalCommandScope = Omit<Scope, "in_reply_to">;
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
