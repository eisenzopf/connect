import { ConnectError } from "./errors.js";

export type Json =
    null | boolean | number | string | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };
export const MAX_ENVELOPE_BYTES = 1024 * 1024;
const fail = (): never => {
    throw new ConnectError("protocol", "Invalid canonical JSON");
};

function scalar(value: string): string {
    for (let i = 0; i < value.length; i++) {
        const code = value.charCodeAt(i);
        if (code >= 0xd800 && code <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff)) fail();
        } else if (code >= 0xdc00 && code <= 0xdfff) fail();
    }
    return value;
}

function limits(
    bytes: number,
    depth: number,
    nodes: number,
    ceiling = 40,
): void {
    if (
        !Number.isSafeInteger(bytes) ||
        bytes < 1 ||
        bytes > 2 * MAX_ENVELOPE_BYTES ||
        !Number.isSafeInteger(depth) ||
        depth < 1 ||
        depth > ceiling ||
        !Number.isSafeInteger(nodes) ||
        nodes < 1 ||
        nodes > 1100000
    )
        fail();
}
/** RFC 8785: UTF-16 key ordering, finite ECMAScript numbers, scalar Unicode. */
export function canonicalJson(
    value: Json,
    maximumBytes = MAX_ENVELOPE_BYTES,
    maximumDepth = 32,
    maximumNodes = 100000,
): string {
    return serialize(value, maximumBytes, maximumDepth, maximumNodes, 40);
}
function serialize(
    value: Json,
    maximumBytes: number,
    maximumDepth: number,
    maximumNodes: number,
    ceiling: number,
): string {
    limits(maximumBytes, maximumDepth, maximumNodes, ceiling);
    let nodes = 0;
    const visit = (item: Json, depth: number): string => {
        if (++nodes > maximumNodes || depth > maximumDepth) return fail();
        if (item === null || typeof item === "boolean")
            return JSON.stringify(item);
        if (typeof item === "string") return JSON.stringify(scalar(item));
        if (typeof item === "number") {
            if (!Number.isFinite(item)) return fail();
            return JSON.stringify(item);
        }
        if (Array.isArray(item)) {
            if (Object.keys(item).length !== item.length) return fail();
            return "[" + item.map((v) => visit(v, depth + 1)).join(",") + "]";
        }
        if (typeof item !== "object" || item === null) return fail();
        const proto = Object.getPrototypeOf(item);
        if (proto !== Object.prototype && proto !== null) return fail();
        return (
            "{" +
            Object.keys(item)
                .sort()
                .map(
                    (key) =>
                        JSON.stringify(scalar(key)) +
                        ":" +
                        visit(item[key], depth + 1),
                )
                .join(",") +
            "}"
        );
    };
    const result = visit(value, 0);
    if (new TextEncoder().encode(result).length > maximumBytes) fail();
    return result;
}

/** Reject duplicate decoded keys before JSON.parse can erase signature input. */
export function parseCanonicalJson(
    bytes: Uint8Array,
    maximumBytes = MAX_ENVELOPE_BYTES,
    maximumDepth = 32,
    maximumNodes = 100000,
): Json {
    return parse(bytes, maximumBytes, maximumDepth, maximumNodes, 40);
}
function parse(
    bytes: Uint8Array,
    maximumBytes: number,
    maximumDepth: number,
    maximumNodes: number,
    ceiling: number,
): Json {
    limits(maximumBytes, maximumDepth, maximumNodes, ceiling);
    if (
        !Number.isSafeInteger(maximumBytes) ||
        maximumBytes < 1 ||
        maximumBytes > 2 * MAX_ENVELOPE_BYTES ||
        !bytes.length ||
        bytes.length > maximumBytes
    )
        return fail();
    let text: string;
    try {
        text = new TextDecoder("utf-8", {
            fatal: true,
            ignoreBOM: true,
        }).decode(bytes);
    } catch {
        return fail();
    }
    let pos = 0,
        nodes = 0;
    const space = () => {
        while (/[\x20\x09\x0a\x0d]/.test(text[pos] ?? "!")) pos++;
    };
    const string = (): string => {
        const start = pos++;
        for (; pos < text.length; pos++) {
            const c = text[pos];
            if (c === "\\") {
                pos++;
                continue;
            }
            if (c === '"') {
                try {
                    return scalar(
                        JSON.parse(text.slice(start, ++pos)) as string,
                    );
                } catch {
                    return fail();
                }
            }
        }
        return fail();
    };
    const value = (depth: number): Json => {
        if (++nodes > maximumNodes || depth > maximumDepth) return fail();
        space();
        const c = text[pos];
        if (c === '"') return string();
        if (c === "{" || c === "[") {
            pos++;
            space();
            const object: JsonObject = Object.create(null) as JsonObject;
            const array: Json[] = [],
                keys = new Set<string>();
            const end = c === "{" ? "}" : "]";
            if (text[pos] === end) {
                pos++;
                return c === "{" ? object : array;
            }
            for (;;) {
                if (c === "{") {
                    if (text[pos] !== '"') return fail();
                    const key = string();
                    if (keys.has(key)) return fail();
                    keys.add(key);
                    space();
                    if (text[pos++] !== ":") return fail();
                    object[key] = value(depth + 1);
                } else array.push(value(depth + 1));
                space();
                if (text[pos] === end) {
                    pos++;
                    return c === "{" ? object : array;
                }
                if (text[pos++] !== ",") return fail();
                space();
            }
        }
        const token = text
            .slice(pos)
            .match(
                /^(?:true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/,
            )?.[0];
        if (!token) return fail();
        pos += token.length;
        const result = JSON.parse(token) as Json;
        if (typeof result === "number" && !Number.isFinite(result))
            return fail();
        return result;
    };
    const result = value(0);
    space();
    if (pos !== text.length) return fail();
    return result;
}

/** Frozen wire parsing matches serde_json's127 permitted nested containers.
 * General application JSON keeps its existing32-depth/100000-node defaults.
 */
function wireDepth(value: Json, parents = 0): void {
    if (value === null || typeof value !== "object") return;
    if (parents >= 127) fail();
    for (const child of Array.isArray(value) ? value : Object.values(value))
        wireDepth(child, parents + 1);
}
export function canonicalWireJson(value: Json): string {
    const result = serialize(value, MAX_ENVELOPE_BYTES, 128, 1100000, 128);
    wireDepth(value);
    return result;
}
export function parseWireJson(bytes: Uint8Array): Json {
    const result = parse(bytes, MAX_ENVELOPE_BYTES, 128, 1100000, 128);
    wireDepth(result);
    return result;
}
export function jsonObject(value: Json | undefined): value is JsonObject {
    return value !== null && typeof value === "object" && !Array.isArray(value);
}
