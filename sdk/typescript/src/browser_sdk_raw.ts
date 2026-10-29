import { validateBrowserCanonical } from "./browser_canonical_validation.js";
import { checkScope } from "./canonical_validation.js";
import {
    FROZEN_FINITE_KINDS,
    FROZEN_READ_KINDS,
} from "./browser_canonical_schema.js";
import { canonicalWireJson, parseWireJson, type Json } from "./uctp_json.js";
import type { CanonicalErrorKind, Certainty } from "./canonical_error.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserSdkRequestOptions } from "./browser_sdk.js";
import type { Envelope, Scope } from "./uctp_wire.js";

declare const validated: unique symbol;
/** Fixed current schema only. Local validation authenticates no signature or grant. */
export interface BrowserValidatedEnvelope {
    readonly value: Json;
    readonly [validated]: "validated-current-canonical-envelope";
}
export interface BrowserRawCanonical {
    validate(value: Json): BrowserValidatedEnvelope;
    request(
        value: BrowserValidatedEnvelope,
        options: BrowserSdkRequestOptions,
    ): Promise<readonly BrowserValidatedEnvelope[]>;
}
interface Batch {
    readonly requestId: string;
    readonly frames: readonly Envelope[];
    check(): void;
    retire(): void;
}
interface Callbacks {
    error(
        kind: CanonicalErrorKind,
        certainty: Certainty,
        key?: BrowserOperationReference,
    ): Error;
    perform(
        value: Envelope,
        scope: Scope,
        options: BrowserSdkRequestOptions,
        key?: BrowserOperationReference,
    ): Promise<Batch>;
}
const tokens = new WeakMap<object, Envelope>(),
    finite = new Set(FROZEN_FINITE_KINDS),
    readKinds = new Set(FROZEN_READ_KINDS);
const allowed = new Set([
    "v",
    "id",
    "ts",
    "type",
    "payload",
    "signature",
    "cid",
    "sid",
    "connid",
    "operation_id",
    "expected_revision",
]);
function freeze<T>(value: T): T {
    if (value && typeof value === "object") {
        Object.values(value).forEach(freeze);
        Object.freeze(value);
    }
    return value;
}
function capture(value: Json): Envelope {
    const copy = parseWireJson(
        new TextEncoder().encode(canonicalWireJson(value)),
    );
    validateBrowserCanonical(copy);
    return freeze(copy);
}
function token(value: Envelope): BrowserValidatedEnvelope {
    const result = Object.freeze({
        value,
    }) as unknown as BrowserValidatedEnvelope;
    tokens.set(result, value);
    return result;
}
export function createBrowserRaw(callbacks: Callbacks): BrowserRawCanonical {
    return Object.freeze({
        validate(value: Json): BrowserValidatedEnvelope {
            try {
                return token(capture(value));
            } catch {
                throw callbacks.error("validation", "not-submitted");
            }
        },
        async request(
            value: BrowserValidatedEnvelope,
            options: BrowserSdkRequestOptions,
        ): Promise<readonly BrowserValidatedEnvelope[]> {
            const saved = tokens.get(value);
            if (!saved) throw callbacks.error("validation", "not-submitted");
            const read = readKinds.has(saved.type),
                scope: Scope = {
                    ...(saved.cid === undefined
                        ? {}
                        : { cid: saved.cid as string }),
                    ...(saved.sid === undefined
                        ? {}
                        : { sid: saved.sid as string }),
                    ...(saved.connid === undefined
                        ? {}
                        : { connid: saved.connid as string }),
                    ...(saved.operation_id === undefined
                        ? {}
                        : { operation_id: saved.operation_id as string }),
                    ...(saved.expected_revision === undefined
                        ? {}
                        : {
                              expected_revision:
                                  saved.expected_revision as number,
                          }),
                },
                key =
                    read || saved.operation_id === undefined
                        ? undefined
                        : freeze({
                              operationId: saved.operation_id as string,
                              command: saved.type,
                              scope: {
                                  ...(scope.cid === undefined
                                      ? {}
                                      : { conversationId: scope.cid }),
                                  ...(scope.sid === undefined
                                      ? {}
                                      : { sessionId: scope.sid }),
                                  ...(scope.connid === undefined
                                      ? {}
                                      : { connectionId: scope.connid }),
                              },
                          });
            const fail = (kind: CanonicalErrorKind): never => {
                throw callbacks.error(
                    kind,
                    key ? "not-submitted" : "read-only",
                    key,
                );
            };
            if (
                Object.keys(saved).some((name) => !allowed.has(name)) ||
                !finite.has(saved.type) ||
                (saved.type === "conversation.subscribe" &&
                    saved.payload.live !== false)
            )
                fail("unsupported");
            if (
                (!read && !key) ||
                (scope.sid !== undefined && scope.cid === undefined) ||
                (scope.connid !== undefined && scope.sid === undefined)
            )
                fail("validation");
            const batch = await callbacks.perform(saved, scope, options, key),
                output: BrowserValidatedEnvelope[] = [],
                ids = new Set<string>();
            try {
                if (!batch.frames.length || batch.frames.length > 128)
                    throw new Error();
                for (const original of batch.frames) {
                    const frame = capture(original);
                    if (
                        ids.has(frame.id) ||
                        frame.in_reply_to !== batch.requestId ||
                        frame.type === "error" ||
                        (output.length > 0 &&
                            (output.at(-1)!.value as Envelope).type === "ack")
                    )
                        throw new Error();
                    checkScope(
                        frame,
                        scope,
                        saved.type === "conversation.create" ||
                            saved.type === "conversation.list",
                    );
                    ids.add(frame.id);
                    output.push(token(frame));
                }
                if ((output.at(-1)!.value as Envelope).type !== "ack")
                    throw new Error();
            } catch {
                batch.retire();
                throw callbacks.error(
                    "protocol",
                    key ? "uncertain" : "read-only",
                    key,
                );
            }
            batch.check();
            return Object.freeze(output);
        },
    });
}
