import {
    browserOrigin,
    invalid,
    enrollmentProofMessage,
    normalizeOffer,
    type BrowserEnrollmentOffer,
} from "./browser_enrollment_contract.js";
import {
    base64url,
    validateSigningIdentity,
    type SigningAlgorithm,
} from "./uctp_signing.js";

/** Structured-clone state for IndexedDB, never JSON or localStorage. Private key
 * bytes are nonextractable; possession of a key handle remains sensitive. */
export interface BrowserEnrollmentKeyState {
    algorithm: SigningAlgorithm;
    publicKey: CryptoKey;
    privateKey: CryptoKey;
}
export class BrowserEnrollmentKey {
    #state: BrowserEnrollmentKeyState;
    #public: string;
    private constructor(state: BrowserEnrollmentKeyState, publicKey: string) {
        this.#state = state;
        this.#public = publicKey;
    }
    static async generate(
        algorithm: SigningAlgorithm,
    ): Promise<BrowserEnrollmentKey> {
        browserOrigin();
        if (!["EdDSA", "ES256"].includes(algorithm)) invalid();
        const pair =
            algorithm === "EdDSA"
                ? await crypto.subtle.generateKey({ name: "Ed25519" }, false, [
                      "sign",
                      "verify",
                  ])
                : await crypto.subtle.generateKey(
                      { name: "ECDSA", namedCurve: "P-256" },
                      false,
                      ["sign", "verify"],
                  );
        return this.restore({
            algorithm,
            publicKey: pair.publicKey,
            privateKey: pair.privateKey,
        });
    }
    static async restore(
        state: BrowserEnrollmentKeyState,
    ): Promise<BrowserEnrollmentKey> {
        browserOrigin();
        const { algorithm, publicKey, privateKey } = state;
        validateSigningIdentity(
            { algorithm, key: privateKey, keyId: "enrollment" },
            "sign",
        );
        validateSigningIdentity(
            { algorithm, key: publicKey, keyId: "enrollment" },
            "verify",
        );
        if (
            privateKey.extractable ||
            privateKey.usages.length !== 1 ||
            publicKey.usages.length !== 1
        )
            invalid();
        const publicBytes = new Uint8Array(
            await crypto.subtle.exportKey("raw", publicKey),
        );
        const probe = new TextEncoder().encode(
            "connect-enrollment-key-check/1",
        );
        const params =
            algorithm === "EdDSA"
                ? { name: "Ed25519" }
                : { name: "ECDSA", hash: "SHA-256" };
        const proof = await crypto.subtle.sign(params, privateKey, probe);
        if (!(await crypto.subtle.verify(params, publicKey, proof, probe)))
            invalid();
        return new BrowserEnrollmentKey(
            { algorithm, publicKey, privateKey },
            base64url(publicBytes),
        );
    }
    get algorithm(): SigningAlgorithm {
        return this.#state.algorithm;
    }
    get publicKey(): string {
        return this.#public;
    }
    /** Deliberate private structured-clone export. Nonextractable key handles
     * may be stored by IndexedDB; JSON encoding cannot restore them. */
    snapshot(): BrowserEnrollmentKeyState {
        return { ...this.#state };
    }
    signing(keyId: string) {
        return {
            algorithm: this.algorithm,
            keyId,
            key: this.#state.privateKey,
        };
    }
    async claim(input: BrowserEnrollmentOffer) {
        const offer = normalizeOffer(input);
        const r = offer.request;
        if (
            r.algorithm !== this.algorithm ||
            r.public_key !== this.publicKey ||
            r.browser_origin !== browserOrigin()
        )
            invalid();
        const signature = await crypto.subtle.sign(
            this.algorithm === "EdDSA"
                ? { name: "Ed25519" }
                : { name: "ECDSA", hash: "SHA-256" },
            this.#state.privateKey,
            await enrollmentProofMessage(offer),
        );
        if (signature.byteLength !== 64) invalid();
        return {
            operation_id: r.completion_operation_id,
            enrollment_id: offer.enrollment_id,
            identity_id: r.identity_id,
            device_id: r.device.id,
            endpoint_id: offer.endpoint_id,
            key_id: r.key_id,
            transport: r.transport,
            browser_origin: r.browser_origin,
            challenge: offer.challenge,
            signature: base64url(new Uint8Array(signature)),
        };
    }
}
