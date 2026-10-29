import { canonicalTimestamp } from "./uctp_datetime.js";
import { ConnectError } from "./errors.js";
import { canonicalWireJson, jsonObject, type JsonObject } from "./uctp_json.js";

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
export function base64url(bytes: Uint8Array): string {
    let raw = "";
    for (const byte of bytes) raw += String.fromCharCode(byte);
    return btoa(raw)
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replaceAll("=", "");
}
export function unbase64url(value: string): Uint8Array<ArrayBuffer> {
    if (!/^[A-Za-z0-9_-]+$/.test(value) || value.length % 4 === 1)
        throw new ConnectError("protocol", "Invalid base64url");
    const bytes = Uint8Array.from(
        atob(value.replaceAll("-", "+").replaceAll("_", "/")),
        (c) => c.charCodeAt(0),
    );
    if (base64url(bytes) !== value)
        throw new ConnectError("protocol", "Noncanonical base64url");
    return bytes;
}
function algorithm(name: SigningAlgorithm): AlgorithmIdentifier | EcdsaParams {
    if (name === "EdDSA") return { name: "Ed25519" };
    if (name === "ES256") return { name: "ECDSA", hash: "SHA-256" };
    throw new ConnectError("invalid-request", "Unsupported signing algorithm");
}
export function validateSigningIdentity(
    identity: SigningIdentity,
    usage: "sign" | "verify",
): void {
    const a = identity.key?.algorithm;
    if (
        !identity.keyId ||
        identity.keyId.length > 512 ||
        identity.key.type !== (usage === "sign" ? "private" : "public") ||
        !identity.key.usages.includes(usage) ||
        (identity.algorithm === "EdDSA"
            ? a?.name !== "Ed25519"
            : identity.algorithm !== "ES256" ||
              a?.name !== "ECDSA" ||
              (a as EcKeyAlgorithm).namedCurve !== "P-256")
    )
        throw new ConnectError(
            "invalid-request",
            "Signing key does not match its enrolled identity",
        );
}
export async function signEnvelope(
    frame: JsonObject,
    identity: SigningIdentity,
): Promise<JsonObject> {
    validateSigningIdentity(identity, "sign");
    const bare = { ...frame };
    delete bare.signature;
    const signature = await crypto.subtle.sign(
        algorithm(identity.algorithm),
        identity.key,
        new TextEncoder().encode(canonicalWireJson(bare)),
    );
    return {
        ...bare,
        signature: {
            keyid: identity.keyId,
            alg: identity.algorithm,
            sig: base64url(new Uint8Array(signature)),
        },
    };
}
export async function verifyEnvelope(
    frame: JsonObject,
    trust: ServerTrust,
    now = Date.now(),
): Promise<void> {
    validateSigningIdentity(trust, "verify");
    const sig = frame.signature;
    const timestamp =
        typeof frame.ts === "string" ? canonicalTimestamp(frame.ts) : NaN;
    if (
        !Number.isFinite(trust.notBefore) ||
        !Number.isFinite(trust.expiresAt) ||
        now / 1000 < trust.notBefore ||
        now / 1000 >= trust.expiresAt ||
        !Number.isFinite(timestamp) ||
        Math.abs(now - timestamp) >= 60000 ||
        !jsonObject(sig) ||
        sig.keyid !== trust.keyId ||
        sig.alg !== trust.algorithm ||
        typeof sig.sig !== "string"
    )
        throw new ConnectError(
            "authentication",
            "Untrusted or expired server envelope",
        );
    const bare = { ...frame };
    delete bare.signature;
    const bytes = unbase64url(sig.sig);
    if (
        bytes.length !== 64 ||
        !(await crypto.subtle.verify(
            algorithm(trust.algorithm),
            trust.key,
            bytes,
            new TextEncoder().encode(canonicalWireJson(bare)),
        ))
    )
        throw new ConnectError(
            "authentication",
            "Invalid server envelope signature",
        );
}
