import { type BrowserEnrollmentOffer } from "./browser_enrollment_contract.js";
import { type SigningAlgorithm } from "./uctp_signing.js";
/** Structured-clone state for IndexedDB, never JSON or localStorage. Private key
 * bytes are nonextractable; possession of a key handle remains sensitive. */
export interface BrowserEnrollmentKeyState {
    algorithm: SigningAlgorithm;
    publicKey: CryptoKey;
    privateKey: CryptoKey;
}
export declare class BrowserEnrollmentKey {
    #private;
    private constructor();
    static generate(algorithm: SigningAlgorithm): Promise<BrowserEnrollmentKey>;
    static restore(state: BrowserEnrollmentKeyState): Promise<BrowserEnrollmentKey>;
    get algorithm(): SigningAlgorithm;
    get publicKey(): string;
    /** Deliberate private structured-clone export. Nonextractable key handles
     * may be stored by IndexedDB; JSON encoding cannot restore them. */
    snapshot(): BrowserEnrollmentKeyState;
    signing(keyId: string): {
        algorithm: SigningAlgorithm;
        keyId: string;
        key: CryptoKey;
    };
    claim(input: BrowserEnrollmentOffer): Promise<{
        operation_id: string;
        enrollment_id: string;
        identity_id: string;
        device_id: string;
        endpoint_id: string;
        key_id: string;
        transport: import("./browser_enrollment_contract.js").BrowserEnrollmentTransport;
        browser_origin: string;
        challenge: string;
        signature: string;
    }>;
}
