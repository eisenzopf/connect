import { ConnectError } from "./errors.js";
export const MAX_ENVELOPE_BYTES = 1024 * 1024;
const fail = () => {
    throw new ConnectError("protocol", "Invalid canonical JSON");
};
function scalar(value) {
    for (let i = 0; i < value.length; i++) {
        const code = value.charCodeAt(i);
        if (code >= 0xd800 && code <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff))
                fail();
        }
        else if (code >= 0xdc00 && code <= 0xdfff)
            fail();
    }
    return value;
}
/** RFC 8785: UTF-16 key ordering, finite ECMAScript numbers, scalar Unicode. */
export function canonicalJson(value) {
    let nodes = 0;
    const visit = (item, depth) => {
        if (++nodes > 100000 || depth > 32)
            return fail();
        if (item === null || typeof item === "boolean")
            return JSON.stringify(item);
        if (typeof item === "string")
            return JSON.stringify(scalar(item));
        if (typeof item === "number") {
            if (!Number.isFinite(item))
                return fail();
            return JSON.stringify(item);
        }
        if (Array.isArray(item)) {
            if (Object.keys(item).length !== item.length)
                return fail();
            return "[" + item.map((v) => visit(v, depth + 1)).join(",") + "]";
        }
        if (typeof item !== "object" || item === null)
            return fail();
        const proto = Object.getPrototypeOf(item);
        if (proto !== Object.prototype && proto !== null)
            return fail();
        return ("{" +
            Object.keys(item)
                .sort()
                .map((key) => JSON.stringify(scalar(key)) +
                ":" +
                visit(item[key], depth + 1))
                .join(",") +
            "}");
    };
    const result = visit(value, 0);
    if (new TextEncoder().encode(result).length > MAX_ENVELOPE_BYTES)
        fail();
    return result;
}
/** Reject duplicate decoded keys before JSON.parse can erase signature input. */
export function parseCanonicalJson(bytes) {
    if (!bytes.length || bytes.length > MAX_ENVELOPE_BYTES)
        return fail();
    let text;
    try {
        text = new TextDecoder("utf-8", {
            fatal: true,
            ignoreBOM: true,
        }).decode(bytes);
    }
    catch {
        return fail();
    }
    let pos = 0, nodes = 0;
    const space = () => {
        while (/[\x20\x09\x0a\x0d]/.test(text[pos] ?? "!"))
            pos++;
    };
    const string = () => {
        const start = pos++;
        for (; pos < text.length; pos++) {
            const c = text[pos];
            if (c === "\\") {
                pos++;
                continue;
            }
            if (c === '"') {
                try {
                    return scalar(JSON.parse(text.slice(start, ++pos)));
                }
                catch {
                    return fail();
                }
            }
        }
        return fail();
    };
    const value = (depth) => {
        if (++nodes > 100000 || depth > 32)
            return fail();
        space();
        const c = text[pos];
        if (c === '"')
            return string();
        if (c === "{" || c === "[") {
            pos++;
            space();
            const object = Object.create(null);
            const array = [], keys = new Set();
            const end = c === "{" ? "}" : "]";
            if (text[pos] === end) {
                pos++;
                return c === "{" ? object : array;
            }
            for (;;) {
                if (c === "{") {
                    if (text[pos] !== '"')
                        return fail();
                    const key = string();
                    if (keys.has(key))
                        return fail();
                    keys.add(key);
                    space();
                    if (text[pos++] !== ":")
                        return fail();
                    object[key] = value(depth + 1);
                }
                else
                    array.push(value(depth + 1));
                space();
                if (text[pos] === end) {
                    pos++;
                    return c === "{" ? object : array;
                }
                if (text[pos++] !== ",")
                    return fail();
                space();
            }
        }
        const token = text
            .slice(pos)
            .match(/^(?:true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/)?.[0];
        if (!token)
            return fail();
        pos += token.length;
        const result = JSON.parse(token);
        if (typeof result === "number" && !Number.isFinite(result))
            return fail();
        return result;
    };
    const result = value(0);
    space();
    if (pos !== text.length)
        return fail();
    return result;
}
export function jsonObject(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value);
}
