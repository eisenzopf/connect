import { type BrowserApplicationReadOptions } from "./browser_application_reads.js";
import { BrowserClient, type BrowserClientOptions, type RequestOptions } from "./browser_client.js";
import { type ServerTrust } from "./uctp_signing.js";
import { type BrowserEnrollmentKeyState } from "./browser_enrollment_key.js";
import { type BrowserEnrollmentCredential, type BrowserEnrollmentOffer, type BrowserEnrollmentRequest } from "./browser_enrollment_contract.js";
export { BrowserEnrollmentKey } from "./browser_enrollment_key.js";
export type { BrowserEnrollmentKeyState } from "./browser_enrollment_key.js";
export type { BrowserEnrollmentRequest, BrowserEnrollmentOffer, BrowserEnrollmentCredential, BrowserEnrollmentTransport, } from "./browser_enrollment_contract.js";
export interface BrowserEnrollmentState {
    version: 1;
    origin: string;
    request: BrowserEnrollmentRequest;
    key: BrowserEnrollmentKeyState;
    offer?: BrowserEnrollmentOffer;
}
export interface BrowserEnrollmentContact {
    url: string;
    server: ServerTrust;
    serverCertificateHashes?: WebTransportHash[];
}
/** Protected in-memory approved state; JSON serialization exposes no fields.
 * Use BrowserEnrollmentStore to persist before network IO. */
export declare class BrowserEnrollmentSession {
    #private;
    private constructor();
    /** Explicit private structured-clone restoration, also used by the vault.
     * An expired challenge remains signable for current completed recovery. */
    static restore(state: BrowserEnrollmentState): Promise<BrowserEnrollmentSession>;
    get offer(): BrowserEnrollmentOffer;
    snapshot(): BrowserEnrollmentState;
    claim(options?: RequestOptions): Promise<BrowserEnrollmentCredential>;
    /** Explicit separately enrolled HTTP read service; never a control fallback. */
    applicationReadOptions(credential: BrowserEnrollmentCredential, url: string): BrowserApplicationReadOptions;
    /** Explicit memory-only installation into exactly the approved transport.
     * Contact/TLS and server signing trust must come from trusted configuration. */
    clientOptions(credential: BrowserEnrollmentCredential, contact: BrowserEnrollmentContact): BrowserClientOptions;
    open(credential: BrowserEnrollmentCredential, contact: BrowserEnrollmentContact, options?: RequestOptions): Promise<BrowserClient>;
}
