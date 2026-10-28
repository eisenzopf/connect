import { type JsonObject } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
import { type CanonicalCommandScope, type CanonicalRequestOptions } from "./canonical_validation.js";
/** Current finite canonical control through one owned Rust helper. Rust verifies
 * TLS, enrollment, signatures and current authority. This adapter validates the
 * fixed helper protocol and retains its original verified signed envelopes.
 * Failure fences this instance; recovery uses a new client and the original
 * saved mutation key. No restart, credential reload or mutation retry occurs. */
export declare class CanonicalNativeClient {
    private readonly child;
    private readonly exited;
    private readonly queue;
    private active?;
    private chunks;
    private bytes;
    private closed;
    private joined;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    });
    request(type: string, payload: JsonObject, scope: CanonicalCommandScope, options: CanonicalRequestOptions): Promise<readonly Envelope[]>;
    private pipeFailure;
    private error;
    private release;
    private cancel;
    private next;
    private fence;
    private receive;
    /** EOF joins the original helper. An active abandoned exchange is uncertain;
     * close never submits Session/Connection end or retries its saved mutation. */
    close(options: CanonicalRequestOptions): Promise<void>;
}
