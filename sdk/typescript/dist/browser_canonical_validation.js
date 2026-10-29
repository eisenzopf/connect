import { FROZEN_ENVELOPE_SCHEMA_JSON, FROZEN_CANONICAL_REGISTRY, } from "./browser_canonical_schema.js";
import { canonicalWireJson, jsonObject } from "./uctp_json.js";
import { canonicalTimestamp } from "./uctp_datetime.js";
import { identifier } from "./uctp_wire.js";
// Only the generated, source-checked schema is parsed; callers cannot supply one.
const schema = JSON.parse(FROZEN_ENVELOPE_SCHEMA_JSON);
const registry = new Map(FROZEN_CANONICAL_REGISTRY.map(([kind, operation, revision]) => [
    kind,
    { operation, revision },
]));
const same = (left, right) => {
    if (typeof left !== typeof right ||
        (left === null) !== (right === null) ||
        Array.isArray(left) !== Array.isArray(right))
        return false;
    return left === null || typeof left !== "object"
        ? left === right
        : canonicalWireJson(left) === canonicalWireJson(right);
};
function bounded(value, minimum, maximum) {
    return ((minimum === undefined || value >= minimum) &&
        (maximum === undefined || value <= maximum));
}
function base64Size(value) {
    if (/[^A-Za-z0-9+/=]/.test(value))
        return Infinity;
    if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value))
        return Infinity;
    const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", padding = value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0;
    if (padding &&
        (alphabet.indexOf(value[value.length - padding - 1]) &
            (padding === 2 ? 15 : 3)) !==
            0)
        return Infinity;
    return (value.length / 4) * 3 - padding;
}
/** Full fixed schema/registry validation only; no signature, authorization or media claim. */
export function validateBrowserCanonical(value) {
    canonicalWireJson(value);
    if (!jsonObject(value) || !jsonObject(value.payload))
        throw new Error("Invalid frozen envelope");
    const entry = typeof value.type === "string" ? registry.get(value.type) : undefined;
    if (!entry ||
        value.v !== 2 ||
        ["id", "cid", "sid", "connid", "in_reply_to", "operation_id"].some((name) => Object.hasOwn(value, name) && !identifier(value[name])) ||
        (value.in_reply_to === undefined &&
            ((entry.operation && value.operation_id === undefined) ||
                (entry.revision && value.expected_revision === undefined))))
        throw new Error("Invalid frozen envelope");
    let steps = 0;
    function matches(rule, item, depth = 0) {
        if (++steps > 1000000 || depth > 128)
            throw new Error("Fixed schema validation bound");
        const visit = (child, part = item) => matches(child, part, depth + 1);
        if (rule.$ref !== undefined)
            return visit(schema.$defs[rule.$ref.slice(8)]);
        if (Object.hasOwn(rule, "const") && !same(rule.const, item))
            return false;
        if (rule.enum && !rule.enum.some((choice) => same(choice, item)))
            return false;
        if (rule.allOf && !rule.allOf.every((child) => visit(child)))
            return false;
        if (rule.anyOf && !rule.anyOf.some((child) => visit(child)))
            return false;
        if (rule.oneOf &&
            rule.oneOf.filter((child) => visit(child)).length !== 1)
            return false;
        if (rule.not && visit(rule.not))
            return false;
        if (rule.if) {
            const branch = visit(rule.if) ? rule.then : rule.else;
            if (branch && !visit(branch))
                return false;
        }
        if (rule.type &&
            !(rule.type === "object"
                ? jsonObject(item)
                : rule.type === "array"
                    ? Array.isArray(item)
                    : rule.type === "integer"
                        ? typeof item === "number" &&
                            Number.isSafeInteger(item) &&
                            item >= 0
                        : rule.type === "null"
                            ? item === null
                            : typeof item === rule.type))
            return false;
        if (jsonObject(item)) {
            if (rule.additionalProperties === false &&
                Object.keys(item).some((name) => !Object.hasOwn(rule.properties ?? {}, name)))
                return false;
            if (rule.required?.some((name) => !Object.hasOwn(item, name)))
                return false;
            if (rule.properties &&
                !Object.entries(rule.properties).every(([name, child]) => !Object.hasOwn(item, name) || visit(child, item[name])))
                return false;
        }
        if (Array.isArray(item)) {
            if (!bounded(item.length, rule.minItems, rule.maxItems))
                return false;
            if (rule.uniqueItems &&
                new Set(item.map((value) => canonicalWireJson(value))).size !==
                    item.length)
                return false;
            if (rule.items && !item.every((value) => visit(rule.items, value)))
                return false;
        }
        if (typeof item === "string") {
            let count = 0;
            for (const _ of item)
                count++;
            if (!bounded(count, rule.minLength, rule.maxLength))
                return false;
            if (rule.pattern &&
                !(rule.pattern === "^[A-Za-z0-9._~-]+$"
                    ? item.length > 0 && !/[^A-Za-z0-9._~-]/.test(item)
                    : /^(sip|sips|tel):[!-~]+$/.test(item) &&
                        !/[^\x21-\x7e]/.test(item)))
                return false;
            if (rule.format === "date-time" &&
                !Number.isFinite(canonicalTimestamp(item)))
                return false;
        }
        if (typeof item === "number" &&
            !bounded(item, rule.minimum, rule.maximum))
            return false;
        return true;
    }
    if (!matches(schema, value))
        throw new Error("Invalid frozen payload");
    if (value.type === "message.send") {
        const body = value.payload.body;
        if (typeof body !== "string" ||
            (value.payload.body_encoding === "base64"
                ? base64Size(body)
                : new TextEncoder().encode(body).length) > 65536)
            throw new Error("Invalid frozen message body");
    }
}
