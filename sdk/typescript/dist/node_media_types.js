/** Captured negotiated media and typed data. No capture, renderer or grant. */
import { inspect } from "node:util";
import { NodeSdkError } from "./sdk_helper.js";
import { canonicalJson, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export function mediaInvalid() {
    throw new NodeSdkError("validation", "not-submitted");
}
export function mediaFrozen(value) {
    if (value && typeof value === "object") {
        for (const v of Object.values(value))
            mediaFrozen(v);
        Object.freeze(value);
    }
    return value;
}
/** Charge the output budget before serializing/copying caller-owned values. */
export function mediaJson(value, maximumBytes) {
    let budget = maximumBytes, nodes = 100000;
    const charge = (n) => {
        budget -= n;
        if (budget < 0)
            throw new Error();
    };
    const visit = (v, depth) => {
        if (--nodes < 0 || depth > 32)
            throw new Error();
        if (typeof v === "string") {
            if (Buffer.byteLength(v) > budget)
                throw new Error();
            charge(Buffer.byteLength(JSON.stringify(v)));
            return v;
        }
        if (v === null || typeof v === "boolean" || typeof v === "number") {
            if (typeof v === "number" && !Number.isFinite(v))
                throw new Error();
            charge(JSON.stringify(v).length);
            return v;
        }
        if (Array.isArray(v)) {
            charge(2 + Math.max(0, v.length - 1));
            if (Object.keys(v).length !== v.length)
                throw new Error();
            const result = [];
            for (let i = 0; i < v.length; i++) {
                if (!Object.hasOwn(v, i))
                    throw new Error();
                result.push(visit(v[i], depth + 1));
            }
            return result;
        }
        if (!v ||
            typeof v !== "object" ||
            ![Object.prototype, null].includes(Object.getPrototypeOf(v)))
            throw new Error();
        const entries = Object.entries(v), result = Object.create(null);
        charge(2 + Math.max(0, entries.length - 1) + entries.length);
        for (const [k, x] of entries) {
            visit(k, depth + 1);
            result[k] = visit(x, depth + 1);
        }
        return result;
    };
    return Buffer.from(canonicalJson(visit(value, 0), maximumBytes));
}
export function uint(value, bits = 32) {
    if (!Number.isSafeInteger(value) ||
        Number(value) < 0 ||
        Number(value) >= 2 ** bits)
        mediaInvalid();
    return value;
}
export function decimal(value) {
    if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(value))
        throw new Error();
    const parsed = BigInt(value);
    if (parsed >= 2n ** 64n)
        throw new Error();
    return parsed;
}
export function captureBinding(v) {
    if (!v ||
        [v.conversationId, v.sessionId, v.connectionId].some((x) => !identifier(x)))
        mediaInvalid();
    return Object.freeze({
        conversationId: v.conversationId,
        sessionId: v.sessionId,
        connectionId: v.connectionId,
    });
}
export function bindingWire(v) {
    return {
        conversation_id: v.conversationId,
        session_id: v.sessionId,
        connection_id: v.connectionId,
    };
}
const rtpBytes = new WeakMap();
export class SdkRtpPacket {
    kind = "rtp";
    datagramSequence;
    header;
    #start;
    #end;
    constructor(sequence, bytes) {
        uint(sequence);
        if (!(bytes instanceof Uint8Array) ||
            bytes.byteLength < 12 ||
            bytes.byteLength > 16396)
            mediaInvalid();
        const raw = Buffer.from(bytes);
        if (raw[0] >>> 6 !== 2)
            mediaInvalid();
        let start = 12 + (raw[0] & 15) * 4, end = raw.length;
        if (start > raw.length)
            mediaInvalid();
        if (raw[0] & 16) {
            if (start + 4 > raw.length)
                mediaInvalid();
            start += 4 + raw.readUInt16BE(start + 2) * 4;
        }
        if (raw[0] & 32) {
            const padding = raw[raw.length - 1];
            if (!padding)
                mediaInvalid();
            end -= padding;
        }
        if (start > end)
            mediaInvalid();
        this.datagramSequence = sequence;
        this.#start = start;
        this.#end = end;
        this.header = Object.freeze({
            payloadType: raw[1] & 127,
            sequenceNumber: raw.readUInt16BE(2),
            timestamp: raw.readUInt32BE(4),
            ssrc: raw.readUInt32BE(8),
            marker: !!(raw[1] & 128),
        });
        rtpBytes.set(this, raw);
        Object.freeze(this);
    }
    get bytes() {
        return Uint8Array.from(rtpBytes.get(this));
    }
    get payload() {
        return Uint8Array.from(rtpBytes.get(this).subarray(this.#start, this.#end));
    }
    [inspect.custom]() {
        return `<Connect RTP packet sequence=${this.datagramSequence} bytes=${rtpBytes.get(this).length}>`;
    }
}
export function capturedRtp(value) {
    const bytes = rtpBytes.get(value);
    if (!bytes)
        mediaInvalid();
    return Buffer.from(bytes);
}
export function clockWire(v) {
    if (!v)
        mediaInvalid();
    for (const n of [v.ssrc, v.rtp, v.packets, v.octets])
        uint(n);
    if (typeof v.ntp !== "bigint" ||
        v.ntp < 0n ||
        v.ntp >= 2n ** 64n ||
        typeof v.cname !== "string" ||
        !v.cname.length ||
        Buffer.byteLength(v.cname) > 255)
        mediaInvalid();
    // Canonical JSON rejects invalid scalar Unicode, without echoing it.
    try {
        canonicalJson(v.cname, 2048);
    }
    catch {
        mediaInvalid();
    }
    return {
        ssrc: v.ssrc,
        ntp: v.ntp.toString(),
        rtp: v.rtp,
        packets: v.packets,
        octets: v.octets,
        cname: v.cname,
    };
}
const dataBytes = new WeakMap();
export class SdkTextData {
    value;
    kind = "text";
    constructor(value) {
        this.value = value;
        try {
            if (typeof value !== "string" || Buffer.byteLength(value) > 65536)
                mediaInvalid();
            canonicalJson(value, 2 * 1024 * 1024);
        }
        catch {
            mediaInvalid();
        }
        const bytes = Buffer.from(value);
        if (bytes.length > 65536)
            mediaInvalid();
        dataBytes.set(this, bytes);
        Object.freeze(this);
    }
    [inspect.custom]() {
        return `<Connect text data bytes=${dataBytes.get(this).length}>`;
    }
}
export class SdkJsonData {
    kind = "json";
    value;
    constructor(value) {
        try {
            const bytes = mediaJson(value, 65536);
            this.value = mediaFrozen(parseCanonicalJson(bytes, 65536));
            dataBytes.set(this, bytes);
        }
        catch {
            mediaInvalid();
        }
        Object.freeze(this);
    }
    [inspect.custom]() {
        return `<Connect JSON data bytes=${dataBytes.get(this).length}>`;
    }
}
export class SdkBinaryData {
    kind = "binary";
    constructor(bytes) {
        if (!(bytes instanceof Uint8Array) || bytes.length > 65536)
            mediaInvalid();
        dataBytes.set(this, Buffer.from(bytes));
        Object.freeze(this);
    }
    get value() {
        return Uint8Array.from(dataBytes.get(this));
    }
    [inspect.custom]() {
        return `<Connect binary data bytes=${dataBytes.get(this).length}>`;
    }
}
export function capturedData(value) {
    const bytes = dataBytes.get(value);
    if (!bytes)
        mediaInvalid();
    return Buffer.from(bytes);
}
export function receivedData(format, bytes) {
    if (format === "binary")
        return new SdkBinaryData(bytes);
    if (format === "text")
        return new SdkTextData(new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes));
    if (format === "json") {
        const value = mediaFrozen(parseCanonicalJson(bytes, 65536));
        // Preserve the received representation; a second serializer can expand
        // a valid full-sized value. Construction remains local and captured.
        const result = Object.create(SdkJsonData.prototype);
        Object.defineProperties(result, {
            kind: { value: "json", enumerable: true },
            value: { value, enumerable: true },
        });
        dataBytes.set(result, Buffer.from(bytes));
        return Object.freeze(result);
    }
    throw new Error();
}
