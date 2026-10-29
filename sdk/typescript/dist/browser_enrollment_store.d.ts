import { BrowserEnrollmentKey } from "./browser_enrollment_key.js";
import { BrowserEnrollmentSession } from "./browser_enrollment.js";
import { type BrowserEnrollmentOffer, type BrowserEnrollmentRequest } from "./browser_enrollment_contract.js";
/** Origin-local IndexedDB vault. Saves a nonextractable client key and complete
 * original intent before approval; it never persists an issued bearer. Each
 * name owns one immutable enrollment. Rotation uses a fresh name and key. */
export declare class BrowserEnrollmentStore {
    #private;
    constructor(database?: string);
    /** Complete original request and its client-owned key must be saved before
     * asking a trusted backend to approve. Name collisions never replace state. */
    prepare(name: string, serverOrigin: string, input: BrowserEnrollmentRequest, key: BrowserEnrollmentKey): Promise<BrowserEnrollmentRequest>;
    request(name: string): Promise<BrowserEnrollmentRequest>;
    /** Saves only an offer matching the complete saved original intent. A
     * different offer cannot replace an approved challenge, even after expiry. */
    acceptOffer(name: string, input: BrowserEnrollmentOffer): Promise<BrowserEnrollmentSession>;
    load(name: string): Promise<BrowserEnrollmentSession>;
    /** Explicit local forgetting only; does not revoke any server credential. */
    forget(name: string): Promise<void>;
}
