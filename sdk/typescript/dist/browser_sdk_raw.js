import { validateBrowserCanonical } from "./browser_canonical_validation.js";
import { checkScope } from "./canonical_validation.js";
import { FROZEN_FINITE_KINDS, FROZEN_READ_KINDS, } from "./browser_canonical_schema.js";
import { canonicalWireJson, parseWireJson } from "./uctp_json.js";
const tokens = new WeakMap(), finite = new Set(FROZEN_FINITE_KINDS), readKinds = new Set(FROZEN_READ_KINDS);
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
function freeze(value) {
    if (value && typeof value === "object") {
        Object.values(value).forEach(freeze);
        Object.freeze(value);
    }
    return value;
}
function capture(value) {
    const copy = parseWireJson(new TextEncoder().encode(canonicalWireJson(value)));
    validateBrowserCanonical(copy);
    return freeze(copy);
}
function token(value) {
    const result = Object.freeze({
        value,
    });
    tokens.set(result, value);
    return result;
}
export function createBrowserRaw(callbacks) {
    return Object.freeze({
        validate(value) {
            try {
                return token(capture(value));
            }
            catch {
                throw callbacks.error("validation", "not-submitted");
            }
        },
        async request(value, options) {
            const saved = tokens.get(value);
            if (!saved)
                throw callbacks.error("validation", "not-submitted");
            const read = readKinds.has(saved.type), scope = {
                ...(saved.cid === undefined
                    ? {}
                    : { cid: saved.cid }),
                ...(saved.sid === undefined
                    ? {}
                    : { sid: saved.sid }),
                ...(saved.connid === undefined
                    ? {}
                    : { connid: saved.connid }),
                ...(saved.operation_id === undefined
                    ? {}
                    : { operation_id: saved.operation_id }),
                ...(saved.expected_revision === undefined
                    ? {}
                    : {
                        expected_revision: saved.expected_revision,
                    }),
            }, key = read || saved.operation_id === undefined
                ? undefined
                : freeze({
                    operationId: saved.operation_id,
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
            const fail = (kind) => {
                throw callbacks.error(kind, key ? "not-submitted" : "read-only", key);
            };
            if (Object.keys(saved).some((name) => !allowed.has(name)) ||
                !finite.has(saved.type) ||
                (saved.type === "conversation.subscribe" &&
                    saved.payload.live !== false))
                fail("unsupported");
            if ((!read && !key) ||
                (scope.sid !== undefined && scope.cid === undefined) ||
                (scope.connid !== undefined && scope.sid === undefined))
                fail("validation");
            const batch = await callbacks.perform(saved, scope, options, key), output = [], ids = new Set();
            try {
                if (!batch.frames.length || batch.frames.length > 128)
                    throw new Error();
                for (const original of batch.frames) {
                    const frame = capture(original);
                    if (ids.has(frame.id) ||
                        frame.in_reply_to !== batch.requestId ||
                        frame.type === "error" ||
                        (output.length > 0 &&
                            output.at(-1).value.type === "ack"))
                        throw new Error();
                    checkScope(frame, scope, saved.type === "conversation.create" ||
                        saved.type === "conversation.list");
                    ids.add(frame.id);
                    output.push(token(frame));
                }
                if (output.at(-1).value.type !== "ack")
                    throw new Error();
            }
            catch {
                batch.retire();
                throw callbacks.error("protocol", key ? "uncertain" : "read-only", key);
            }
            batch.check();
            return Object.freeze(output);
        },
    });
}
