import { ConnectError } from "./errors.js";
import { canonicalJson, type Json } from "./uctp_json.js";
import { unbase64url, type SigningAlgorithm } from "./uctp_signing.js";

export const ENROLLMENT_PROFILE = "connect-enrollment/1";
export type BrowserEnrollmentTransport = "webtransport" | "wss-webrtc";
export interface BrowserEnrollmentRequest {
    operation_id: string;
    completion_operation_id: string;
    identity_id: string;
    device: { id: string; kind: string; platform: string };
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
export function invalid(): never {
    throw new ConnectError(
        "invalid-request",
        "Invalid bounded browser enrollment",
    );
}
export function closed(
    value: unknown,
    names: string[],
): asserts value is Record<string, unknown> {
    if (
        !value ||
        typeof value !== "object" ||
        Array.isArray(value) ||
        ![Object.prototype, null].includes(Object.getPrototypeOf(value)) ||
        Object.keys(value).length !== names.length ||
        names.some((k) => !Object.hasOwn(value, k))
    )
        invalid();
}
export function id(value: unknown): asserts value is string {
    if (typeof value !== "string" || !/^[A-Za-z0-9._~-]{1,128}$/.test(value))
        invalid();
}
export function origin(value: unknown): string {
    if (
        typeof value !== "string" ||
        value.length > 2048 ||
        !/^[\x21-\x7e]+$/.test(value)
    )
        invalid();
    let url: URL;
    try {
        url = new URL(value);
    } catch {
        return invalid();
    }
    if (
        url.protocol !== "https:" ||
        url.origin !== value ||
        url.username ||
        url.password ||
        url.port === "0"
    )
        invalid();
    return value;
}
export function browserOrigin(): string {
    if (globalThis.isSecureContext !== true || !globalThis.location) {
        throw new ConnectError(
            "unsupported-transport",
            "Secure browser context required for enrollment",
        );
    }
    return origin(globalThis.location.origin);
}
export function json(value: unknown): string {
    return canonicalJson(value as Json, 65536);
}
export function snapshot<T>(value: T): T {
    return JSON.parse(json(value)) as T;
}
function timestamp(value: unknown): asserts value is number {
    if (!Number.isSafeInteger(value) || (value as number) <= 0) invalid();
}
function set(value: unknown, max: number, operations = false): string[] {
    if (
        !Array.isArray(value) ||
        value.length > max ||
        value.some(
            (v) =>
                typeof v !== "string" ||
                !(
                    operations
                        ? /^[A-Za-z0-9_.:-]{1,128}$/
                        : /^[A-Za-z0-9._~-]{1,128}$/
                ).test(v) ||
                (operations && ["admin", "credential.delegate"].includes(v)),
        )
    )
        invalid();
    return [...new Set(value as string[])].sort();
}
export function normalizeRequest(
    input: BrowserEnrollmentRequest,
): BrowserEnrollmentRequest {
    const value = snapshot(input);
    closed(value, [
        "operation_id",
        "completion_operation_id",
        "identity_id",
        "device",
        "key_id",
        "algorithm",
        "public_key",
        "transport",
        "endpoint_address",
        "capabilities",
        "browser_origin",
        "operations",
        "recipients",
        "credential_expires_at",
        "challenge_expires_at",
    ]);
    for (const key of [
        "operation_id",
        "completion_operation_id",
        "identity_id",
        "key_id",
    ])
        id(value[key]);
    if (value.operation_id === value.completion_operation_id) invalid();
    closed(value.device, ["id", "kind", "platform"]);
    id(value.device.id);
    id(value.device.kind);
    if (
        typeof value.device.platform !== "string" ||
        !value.device.platform.length ||
        new TextEncoder().encode(value.device.platform).length > 128 ||
        /[\u0000-\u001f\u007f-\u009f]/.test(value.device.platform)
    )
        invalid();
    if (
        !["EdDSA", "ES256"].includes(value.algorithm) ||
        typeof value.public_key !== "string" ||
        value.public_key.length > 128
    )
        invalid();
    const publicKey = unbase64url(value.public_key);
    if (
        publicKey.length !== (value.algorithm === "EdDSA" ? 32 : 65) ||
        (value.algorithm === "ES256" && publicKey[0] !== 4)
    )
        invalid();
    if (!["webtransport", "wss-webrtc"].includes(value.transport)) invalid();
    origin(value.browser_origin);
    if (
        typeof value.endpoint_address !== "string" ||
        value.endpoint_address.length > 2048 ||
        !/^[\x21-\x7e]+$/.test(value.endpoint_address) ||
        !value.endpoint_address.startsWith("https://") ||
        value.endpoint_address.includes("@")
    )
        invalid();
    let endpoint: URL;
    try {
        endpoint = new URL(value.endpoint_address);
    } catch {
        return invalid();
    }
    if (
        !endpoint.hostname ||
        endpoint.username ||
        endpoint.password ||
        endpoint.search ||
        endpoint.hash
    )
        invalid();
    value.endpoint_address = endpoint.href;
    value.capabilities = set(value.capabilities, 32);
    value.operations = set(value.operations, 64, true);
    value.recipients = set(value.recipients, 128);
    if (!value.capabilities.length || !value.operations.length) invalid();
    timestamp(value.credential_expires_at);
    timestamp(value.challenge_expires_at);
    if (value.challenge_expires_at > value.credential_expires_at) invalid();
    return value;
}
export function normalizeOffer(
    input: BrowserEnrollmentOffer,
): BrowserEnrollmentOffer {
    const value = snapshot(input);
    closed(value, [
        "profile",
        "enrollment_id",
        "endpoint_id",
        "installation_id",
        "issuer_identity_id",
        "request",
        "challenge",
    ]);
    if (
        value.profile !== ENROLLMENT_PROFILE ||
        typeof value.challenge !== "string" ||
        !/^[a-f0-9]{64}$/.test(value.challenge)
    )
        invalid();
    for (const key of [
        "enrollment_id",
        "endpoint_id",
        "installation_id",
        "issuer_identity_id",
    ])
        id(value[key]);
    if ([".", ".."].includes(value.enrollment_id)) invalid();
    value.request = normalizeRequest(value.request);
    return value;
}
/** Exact Rust/JCS proof context, including every normalized approved field. */
export async function enrollmentProofMessage(
    input: BrowserEnrollmentOffer,
): Promise<Uint8Array<ArrayBuffer>> {
    const offer = normalizeOffer(input);
    const digest = new Uint8Array(
        await crypto.subtle.digest(
            "SHA-256",
            new TextEncoder().encode(json(offer)),
        ),
    );
    const domain = new TextEncoder().encode("connect-enrollment-proof/1\0");
    const message = new Uint8Array(domain.length + digest.length);
    message.set(domain);
    message.set(digest, domain.length);
    return message;
}
export function checkCredential(
    input: unknown,
    offer: BrowserEnrollmentOffer,
): BrowserEnrollmentCredential {
    const value = snapshot(input) as BrowserEnrollmentCredential;
    closed(value, [
        "enrollment_id",
        "endpoint_id",
        "device_id",
        "key_id",
        "transport",
        "browser_origin",
        "credential",
    ]);
    const c = value.credential;
    const r = offer.request;
    closed(c, [
        "operation_id",
        "credential_id",
        "identity_id",
        "issuer_identity_id",
        "issuer_credential_id",
        "installation_id",
        "operations",
        "recipients",
        "issued_at",
        "expires_at",
        "credential",
    ]);
    const now = Date.now() / 1000;
    const uuid = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/;
    if (
        value.enrollment_id !== offer.enrollment_id ||
        value.endpoint_id !== offer.endpoint_id ||
        value.device_id !== r.device.id ||
        value.key_id !== r.key_id ||
        value.transport !== r.transport ||
        value.browser_origin !== r.browser_origin ||
        c.operation_id !== r.completion_operation_id ||
        c.identity_id !== r.identity_id ||
        c.issuer_identity_id !== offer.issuer_identity_id ||
        c.installation_id !== offer.installation_id ||
        json(c.operations) !== json(r.operations) ||
        json(c.recipients) !== json(r.recipients) ||
        c.expires_at !== r.credential_expires_at ||
        !Number.isSafeInteger(c.issued_at) ||
        c.issued_at <= 0 ||
        c.issued_at > now ||
        c.issued_at >= c.expires_at ||
        c.expires_at <= now ||
        !uuid.test(c.credential_id) ||
        !uuid.test(c.issuer_credential_id) ||
        typeof c.credential !== "string" ||
        !/^[a-f0-9]{64}$/.test(c.credential)
    ) {
        throw new ConnectError(
            "protocol",
            "Enrollment credential does not match the original approved offer",
        );
    }
    return value;
}
