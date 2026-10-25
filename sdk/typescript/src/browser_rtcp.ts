// Bounded RFC 3550 SR/SDES primitives. The transport owner must independently
// bind every report to a current allocated handle, SSRC and peer lifetime.
import { ConnectError } from "./errors.js";
import { MEDIA_LIMITS } from "./browser_rtp.js";

const maximumNtp = (1n << 64n) - 1n;
const invalid = (): never => {
    throw new ConnectError("protocol", "Invalid bounded RTCP clock compound");
};
const u32 = (n: number): boolean =>
    Number.isInteger(n) && n >= 0 && n <= 0xffffffff;
const ntp = (n: bigint): boolean =>
    typeof n === "bigint" && n >= 0n && n <= maximumNtp;
export interface RtcpSenderReport {
    ssrc: number;
    ntp: bigint;
    rtpTimestamp: number;
    packets: number;
    octets: number;
    cname: string;
}
const cnameBytes = (value: string): Uint8Array => {
    if (
        typeof value !== "string" ||
        !value.length ||
        /[\u0000-\u001f\u007f]/u.test(value)
    )
        return invalid();
    // TextEncoder replaces lone surrogates; clock identity must never silently change.
    for (const character of value) {
        const point = character.codePointAt(0)!;
        if (point >= 0xd800 && point <= 0xdfff) return invalid();
    }
    const encoded = new TextEncoder().encode(value);
    if (encoded.length > 255) return invalid();
    return encoded;
};
/** One sender report and its matching CNAME; no receive report blocks. */
export function encodeRtcpSenderReport(
    report: Readonly<RtcpSenderReport>,
): Uint8Array {
    if (
        ![
            report.ssrc,
            report.rtpTimestamp,
            report.packets,
            report.octets,
        ].every(u32) ||
        !ntp(report.ntp)
    )
        return invalid();
    const name = cnameBytes(report.cname);
    const sdesBytes = Math.ceil((4 + 4 + 2 + name.length + 1) / 4) * 4;
    const bytes = new Uint8Array(28 + sdesBytes),
        view = new DataView(bytes.buffer);
    bytes[0] = 0x80;
    bytes[1] = 200;
    view.setUint16(2, 6);
    view.setUint32(4, report.ssrc);
    view.setBigUint64(8, report.ntp);
    view.setUint32(16, report.rtpTimestamp);
    view.setUint32(20, report.packets);
    view.setUint32(24, report.octets);
    bytes[28] = 0x81;
    bytes[29] = 202;
    view.setUint16(30, sdesBytes / 4 - 1);
    view.setUint32(32, report.ssrc);
    bytes[36] = 1;
    bytes[37] = name.length;
    bytes.set(name, 38);
    return bytes;
}
/** Parse a non-reduced-size clock compound. Unknown bounded RTCP packets are
 * ignored; they cannot invoke feedback, media or application control here. */
export function decodeRtcpSenderReport(bytes: Uint8Array): RtcpSenderReport {
    if (
        bytes.length < 40 ||
        bytes.length > MEDIA_LIMITS.rtpBytes ||
        bytes.length % 4
    )
        return invalid();
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    let sender: Omit<RtcpSenderReport, "cname"> | undefined;
    const names = new Map<number, string>();
    let offset = 0,
        packets = 0;
    while (offset < bytes.length) {
        if (
            ++packets > 16 ||
            offset + 4 > bytes.length ||
            bytes[offset] >>> 6 !== 2
        )
            return invalid();
        const count = bytes[offset] & 31,
            type = bytes[offset + 1];
        const length = (view.getUint16(offset + 2) + 1) * 4,
            end = offset + length;
        if (
            end > bytes.length ||
            type < 192 ||
            type > 223 ||
            (offset === 0 && type !== 200)
        )
            return invalid();
        let effective = end;
        if (bytes[offset] & 32) {
            const padding = bytes[end - 1];
            if (end !== bytes.length || !padding || padding > length - 4)
                return invalid();
            effective -= padding;
        }
        if (type === 200) {
            if (sender || effective - offset !== 28 + count * 24)
                return invalid();
            sender = {
                ssrc: view.getUint32(offset + 4),
                ntp: view.getBigUint64(offset + 8),
                rtpTimestamp: view.getUint32(offset + 16),
                packets: view.getUint32(offset + 20),
                octets: view.getUint32(offset + 24),
            };
        } else if (type === 201) {
            if (effective - offset !== 8 + count * 24) return invalid();
        } else if (type === 202) {
            let at = offset + 4;
            for (let n = 0; n < count; n++) {
                if (at + 4 > effective) return invalid();
                const ssrc = view.getUint32(at);
                at += 4;
                let terminated = false;
                while (at < effective) {
                    const item = bytes[at++];
                    if (!item) {
                        terminated = true;
                        break;
                    }
                    if (at >= effective) return invalid();
                    const size = bytes[at++];
                    if (at + size > effective) return invalid();
                    if (item === 1) {
                        if (!size || names.has(ssrc)) return invalid();
                        let name: string;
                        try {
                            name = new TextDecoder("utf-8", {
                                fatal: true,
                            }).decode(bytes.subarray(at, at + size));
                        } catch {
                            return invalid();
                        }
                        cnameBytes(name);
                        names.set(ssrc, name);
                    }
                    at += size;
                }
                if (!terminated) return invalid();
                while ((at - offset) % 4) {
                    if (at >= effective || bytes[at++] !== 0) return invalid();
                }
            }
            if (at !== effective) return invalid();
        } else if (type === 203) {
            // A BYE compound is not a usable live sender-clock sample. The
            // owning media lifecycle handles retirement independently.
            return invalid();
        }
        offset = end;
    }
    if (!sender || !names.has(sender.ssrc)) return invalid();
    return { ...sender, cname: names.get(sender.ssrc)! };
}
/** The profile's fixed Opus/VP8 payload types do not collide with RFC 5761's
 * RTCP range. Classification alone performs no parsing, routing or admission. */
export function isRtcpDatagram(packet: Uint8Array): boolean {
    return (
        packet.length >= 12 &&
        packet.length <= MEDIA_LIMITS.rtpBytes + 8 &&
        packet[0] === 1 &&
        (packet[2] !== 0 || packet[3] !== 0) &&
        packet[8] >>> 6 === 2 &&
        packet[9] >= 192 &&
        packet[9] <= 223
    );
}
export function wrapRtcpSenderReport(
    handle: number,
    sequence: number,
    report: Readonly<RtcpSenderReport>,
): Uint8Array {
    if (
        !Number.isInteger(handle) ||
        handle < 1 ||
        handle > 65535 ||
        !u32(sequence)
    )
        return invalid();
    const body = encodeRtcpSenderReport(report),
        packet = new Uint8Array(body.length + 8),
        view = new DataView(packet.buffer);
    packet[0] = 1;
    view.setUint16(2, handle);
    view.setUint32(4, sequence);
    packet.set(body, 8);
    return packet;
}
/** Convert Unix microseconds into the wrapping NTP64 era. */
export function unixMicrosecondsToNtp(value: bigint): bigint {
    if (
        typeof value !== "bigint" ||
        value < 0n ||
        value > BigInt(Number.MAX_SAFE_INTEGER)
    )
        return invalid();
    return BigInt.asUintN(
        64,
        ((value / 1000000n + 2208988800n) << 32n) +
            ((value % 1000000n) << 32n) / 1000000n,
    );
}
/** Sampling-time interpolation only. A zero report clock explicitly stays
 * unavailable; neither this helper nor a caller's timing data grants media IO. */
export function ntpAtRtpTimestamp(
    report: Readonly<RtcpSenderReport>,
    timestamp: number,
    clockRate: 8000 | 48000 | 90000,
): bigint | undefined {
    if (
        !u32(timestamp) ||
        !u32(report.rtpTimestamp) ||
        !ntp(report.ntp) ||
        ![8000, 48000, 90000].includes(clockRate)
    )
        return invalid();
    if (report.ntp === 0n) return undefined;
    const difference = (timestamp - report.rtpTimestamp) | 0;
    if (difference === -2147483648) return invalid();
    const numerator = BigInt(difference) << 32n,
        rate = BigInt(clockRate);
    let increment = numerator / rate;
    if (numerator < 0n && numerator % rate) increment--;
    return BigInt.asUintN(64, report.ntp + increment);
}
