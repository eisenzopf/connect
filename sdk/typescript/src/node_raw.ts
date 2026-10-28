import { spawnSync } from "node:child_process";
import { type OperationKey } from "./canonical_error.js";
import {
    checkScope,
    type CanonicalRequestOptions,
} from "./canonical_validation.js";
import { NodeSdkError, SdkHelper, object } from "./sdk_helper.js";
import {
    canonicalJson,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier, validateEnvelope } from "./uctp_wire.js";
import { unbase64url } from "./uctp_signing.js";

declare const identity: unique symbol;
/** Local schema validation grants neither authentication nor media custody. */
export interface SdkValidatedEnvelope {
    readonly value: Json;
    readonly [identity]: "validated-current-canonical-envelope";
}
export interface SdkRawCanonical {
    /** Synchronous frozen Rust schema check, bounded to two seconds, without configuration or network. */
    validate(value: Json): SdkValidatedEnvelope;
    /** Same owned Rust SDK peer; fresh transport metadata, original logical intent, no retry. */
    request(
        envelope: SdkValidatedEnvelope,
        options: CanonicalRequestOptions,
    ): Promise<readonly SdkValidatedEnvelope[]>;
}
interface Intent {
    readonly key?: OperationKey;
    readonly error?: NodeSdkError;
}
const tokens = new WeakMap<object, { value: JsonObject; intent: Intent }>();
function freeze<T>(value: T): T {
    if (value && typeof value === "object") {
        Object.values(value).forEach(freeze);
        Object.freeze(value);
    }
    return value;
}
function key(value: Json, envelope: JsonObject): OperationKey | undefined {
    if (value === null) return undefined;
    if (
        !object(value, ["operationId", "command", "scope"]) ||
        !identifier(value.operationId) ||
        value.operationId !== envelope.operation_id ||
        value.command !== envelope.type ||
        !object(value.scope, ["conversationId", "sessionId", "connectionId"])
    )
        throw new Error();
    const scope: Record<string, string> = {};
    for (const [wire, field] of [
        ["conversationId", "cid"],
        ["sessionId", "sid"],
        ["connectionId", "connid"],
    ]) {
        const id = value.scope[wire];
        if (
            id !== (envelope[field] ?? null) ||
            (id !== null && !identifier(id))
        )
            throw new Error();
        if (id !== null) scope[wire] = id as string;
    }
    return freeze({
        operationId: value.operationId,
        command: value.command as string,
        scope,
    });
}
/** Classification comes from the same Rust RawCanonical intent function as execution. */
function intent(value: Json, envelope: JsonObject): Intent {
    if (
        object(value, ["key"]) &&
        Object.keys(value).length === 1 &&
        "key" in value
    )
        return { key: key(value.key, envelope) };
    if (
        !object(value, ["error"]) ||
        !object(value.error, [
            "kind",
            "certainty",
            "operation",
            "problem",
            "observation",
        ])
    )
        throw new Error();
    const e = value.error;
    if (
        (e.kind !== "unsupported" && e.kind !== "validation") ||
        (e.certainty !== "not-submitted" && e.certainty !== "read-only") ||
        e.problem !== null ||
        e.observation !== null
    )
        throw new Error();
    const operation = key(e.operation, envelope);
    if (operation && e.certainty !== "not-submitted") throw new Error();
    return { error: new NodeSdkError(e.kind, e.certainty, operation) };
}
function token(value: JsonObject, descriptor: Intent): SdkValidatedEnvelope {
    const result = freeze({ value }) as SdkValidatedEnvelope;
    tokens.set(result, { value, intent: descriptor });
    return result;
}
function batch(
    value: Json,
    original: JsonObject,
): readonly SdkValidatedEnvelope[] {
    if (
        !object(value, ["requestId", "envelopes"]) ||
        !identifier(value.requestId) ||
        !Array.isArray(value.envelopes) ||
        value.envelopes.length < 1 ||
        value.envelopes.length > 128
    )
        throw new Error();
    const ids = new Set<string>(),
        replies: SdkValidatedEnvelope[] = [];
    for (const item of value.envelopes) {
        if (!object(item, ["value", "request"])) throw new Error();
        // Rust has validated the full frozen schema and actual signature. JS checks
        // framing/capture again, without claiming a separate cryptographic verifier.
        const frame = parseCanonicalJson(
            Buffer.from(canonicalJson(item.value)),
        );
        validateEnvelope(frame);
        if (
            ids.has(frame.id) ||
            frame.in_reply_to !== value.requestId ||
            ["ack", "error"].includes(
                (replies.at(-1)?.value as JsonObject)?.type as string,
            ) ||
            frame.type === "error"
        )
            throw new Error();
        const sig = frame.signature;
        if (
            !object(sig, ["keyid", "alg", "sig"]) ||
            typeof sig.keyid !== "string" ||
            !sig.keyid ||
            sig.keyid.length > 512 ||
            typeof sig.alg !== "string" ||
            !["EdDSA", "ES256"].includes(sig.alg) ||
            typeof sig.sig !== "string" ||
            unbase64url(sig.sig).length !== 64
        )
            throw new Error();
        checkScope(
            frame,
            original,
            original.type === "conversation.create" ||
                original.type === "conversation.list",
        );
        ids.add(frame.id);
        replies.push(token(frame, intent(item.request, frame)));
    }
    if ((replies.at(-1)!.value as JsonObject).type !== "ack") throw new Error();
    return Object.freeze(replies);
}
export function createRaw(
    helper: SdkHelper,
    executable: string,
): SdkRawCanonical {
    return Object.freeze({
        validate(value: Json): SdkValidatedEnvelope {
            let captured: JsonObject, bytes: Buffer;
            try {
                bytes = Buffer.from(canonicalJson(value));
                captured = parseCanonicalJson(bytes) as JsonObject;
                validateEnvelope(captured);
            } catch {
                throw new NodeSdkError("validation", "not-submitted");
            }
            const child = spawnSync(executable, ["sdk-validate-envelope"], {
                input: bytes,
                timeout: 2000,
                killSignal: "SIGKILL",
                maxBuffer: 16 * 1024,
                stdio: ["pipe", "pipe", "ignore"],
            });
            if (child.error || child.status !== 0)
                throw new NodeSdkError("configuration", "not-submitted");
            try {
                const result = parseCanonicalJson(child.stdout, 16 * 1024);
                if (
                    !object(result, ["profile", "valid", "request"]) ||
                    result.profile !== "connect-sdk-validation/1" ||
                    typeof result.valid !== "boolean"
                )
                    throw new Error();
                if (!result.valid) {
                    if (result.request !== null) throw new Error();
                    throw new NodeSdkError("validation", "not-submitted");
                }
                return token(captured, intent(result.request, captured));
            } catch (error) {
                if (error instanceof NodeSdkError) throw error;
                helper.fence("protocol");
                throw new NodeSdkError("protocol", "not-submitted");
            }
        },
        request(
            envelope: SdkValidatedEnvelope,
            options: CanonicalRequestOptions,
        ): Promise<readonly SdkValidatedEnvelope[]> {
            const saved = tokens.get(envelope);
            if (!saved) throw new NodeSdkError("validation", "not-submitted");
            if (saved.intent.error)
                throw new NodeSdkError(
                    saved.intent.error.kind,
                    saved.intent.error.certainty,
                    saved.intent.error.operation,
                );
            return helper.request(
                { type: "raw_request", envelope: saved.value },
                options,
                (value) => batch(value, saved.value),
                saved.intent.key,
                !!saved.intent.key,
            );
        },
    });
}
