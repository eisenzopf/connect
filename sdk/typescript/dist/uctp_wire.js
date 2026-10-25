import { ConnectError } from "./errors.js";
import { canonicalJson, jsonObject, MAX_ENVELOPE_BYTES, parseCanonicalJson, } from "./uctp_json.js";
export function identifier(value) {
    return typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
}
export function envelope(type, payload, scope = {}) {
    const frame = {
        v: 2,
        type,
        id: "env_" + crypto.randomUUID().replaceAll("-", ""),
        ts: new Date().toISOString(),
        // Capture the intention before signing or a queued write can await.
        payload: parseCanonicalJson(new TextEncoder().encode(canonicalJson(payload))),
    };
    for (const key of [
        "cid",
        "sid",
        "connid",
        "operation_id",
        "expected_revision",
        "in_reply_to",
    ])
        if (scope[key] !== undefined)
            frame[key] = scope[key];
    validateEnvelope(frame);
    return frame;
}
export function validateEnvelope(frame) {
    if (!jsonObject(frame) || frame.v !== 2)
        throw new ConnectError("protocol", "Expected UCTP v2 envelope");
    const value = frame;
    if (!identifier(value.id) ||
        typeof value.type !== "string" ||
        !/^[a-z][a-z.-]{1,63}$/.test(value.type) ||
        typeof value.ts !== "string" ||
        !Number.isFinite(Date.parse(value.ts)) ||
        !jsonObject(value.payload))
        throw new ConnectError("protocol", "Malformed UCTP envelope");
    for (const key of ["cid", "sid", "connid", "operation_id", "in_reply_to"])
        if (key in value && !identifier(value[key]))
            throw new ConnectError("protocol", "Invalid envelope scope");
    if ("expected_revision" in value &&
        (!Number.isSafeInteger(value.expected_revision) ||
            Number(value.expected_revision) < 0))
        throw new ConnectError("protocol", "Invalid resource revision");
}
export function encodeEnvelope(frame) {
    validateEnvelope(frame);
    const body = new TextEncoder().encode(canonicalJson(frame));
    const output = new Uint8Array(4 + body.length);
    new DataView(output.buffer).setUint32(0, body.length);
    output.set(body, 4);
    return output;
}
/** Incremental length framing with one bounded frame under assembly. */
export class EnvelopeReader {
    header = new Uint8Array(4);
    headerSize = 0;
    body;
    bodySize = 0;
    *push(chunk) {
        let offset = 0;
        while (offset < chunk.length) {
            if (!this.body) {
                const size = Math.min(4 - this.headerSize, chunk.length - offset);
                this.header.set(chunk.subarray(offset, offset + size), this.headerSize);
                this.headerSize += size;
                offset += size;
                if (this.headerSize !== 4)
                    continue;
                const length = new DataView(this.header.buffer).getUint32(0);
                if (!length || length > MAX_ENVELOPE_BYTES)
                    throw new ConnectError("protocol", "Envelope length exceeds bound");
                this.body = new Uint8Array(length);
            }
            const size = Math.min(this.body.length - this.bodySize, chunk.length - offset);
            this.body.set(chunk.subarray(offset, offset + size), this.bodySize);
            this.bodySize += size;
            offset += size;
            if (this.bodySize === this.body.length) {
                const value = parseCanonicalJson(this.body);
                validateEnvelope(value);
                this.body = undefined;
                this.bodySize = this.headerSize = 0;
                yield value;
            }
        }
    }
    end() {
        if (this.body || this.headerSize)
            throw new ConnectError("protocol", "Truncated envelope at EOF");
    }
}
