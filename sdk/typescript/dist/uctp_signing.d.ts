import { type JsonObject } from "./uctp_json.js";
export type SigningAlgorithm = "EdDSA" | "ES256";
export interface SigningIdentity {
    key: CryptoKey;
    keyId: string;
    algorithm: SigningAlgorithm;
}
/** Separately enrolled Sig9421 trust, independent of HTTPS certificate trust. */
export interface ServerTrust extends SigningIdentity {
    notBefore: number;
    expiresAt: number;
}
export declare function base64url(bytes: Uint8Array): string;
export declare function unbase64url(value: string): Uint8Array<ArrayBuffer>;
export declare function validateSigningIdentity(identity: SigningIdentity, usage: "sign" | "verify"): void;
export declare function signEnvelope(frame: JsonObject, identity: SigningIdentity): Promise<JsonObject>;
export declare function verifyEnvelope(frame: JsonObject, trust: ServerTrust, now?: number): Promise<void>;
