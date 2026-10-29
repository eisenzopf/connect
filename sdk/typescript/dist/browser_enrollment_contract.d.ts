import { type SigningAlgorithm } from "./uctp_signing.js";
export declare const ENROLLMENT_PROFILE = "connect-enrollment/1";
export type BrowserEnrollmentTransport = "webtransport" | "wss-webrtc";
export interface BrowserEnrollmentRequest {
    operation_id: string;
    completion_operation_id: string;
    identity_id: string;
    device: {
        id: string;
        kind: string;
        platform: string;
    };
    key_id: string;
    algorithm: SigningAlgorithm;
    public_key: string;
    transport: BrowserEnrollmentTransport;
    endpoint_address: string;
    capabilities: string[];
    browser_origin: string;
    operations: string[];
    recipients: string[];
    credential_expires_at: number;
    challenge_expires_at: number;
}
export interface BrowserEnrollmentOffer {
    profile: typeof ENROLLMENT_PROFILE;
    enrollment_id: string;
    endpoint_id: string;
    installation_id: string;
    issuer_identity_id: string;
    request: BrowserEnrollmentRequest;
    challenge: string;
}
export interface BrowserEnrollmentCredential {
    enrollment_id: string;
    endpoint_id: string;
    device_id: string;
    key_id: string;
    transport: BrowserEnrollmentTransport;
    browser_origin: string;
    credential: {
        operation_id: string;
        credential_id: string;
        identity_id: string;
        issuer_identity_id: string;
        issuer_credential_id: string;
        installation_id: string;
        operations: string[];
        recipients: string[];
        issued_at: number;
        expires_at: number;
        credential: string;
    };
}
export declare function invalid(): never;
export declare function closed(value: unknown, names: string[]): asserts value is Record<string, unknown>;
export declare function id(value: unknown): asserts value is string;
export declare function origin(value: unknown): string;
export declare function browserOrigin(): string;
export declare function json(value: unknown): string;
export declare function snapshot<T>(value: T): T;
export declare function normalizeRequest(input: BrowserEnrollmentRequest): BrowserEnrollmentRequest;
export declare function normalizeOffer(input: BrowserEnrollmentOffer): BrowserEnrollmentOffer;
/** Exact Rust/JCS proof context, including every normalized approved field. */
export declare function enrollmentProofMessage(input: BrowserEnrollmentOffer): Promise<Uint8Array<ArrayBuffer>>;
export declare function checkCredential(input: unknown, offer: BrowserEnrollmentOffer): BrowserEnrollmentCredential;
