/** Native video wire syntax only. Selection, signed notices, current authority
 * and original arrival age must be checked by the physical media owner. */
import { ConnectError } from "./errors.js";
import {
    decodeRtpDatagram,
    MEDIA_LIMITS,
    type RtpDatagram,
} from "./browser_rtp.js";

export const VIDEO_FEEDBACK_PROFILE = "rtcp-pli-epoch/1";
export interface NativeVideoFeedbackPacket {
    readonly handle: number;
    readonly datagramSequence: number;
    readonly senderSsrc: number;
    readonly mediaSsrc: number;
    readonly epoch: string;
}
export interface NativeVideoEpochPacket {
    readonly epoch: string;
    readonly packet: RtpDatagram;
}
const invalid = (): never => {
    throw new ConnectError("protocol", "Invalid native video profile packet");
};
const uint = (value: number, maximum: number, minimum = 0): boolean =>
    Number.isInteger(value) && value >= minimum && value <= maximum;
const validEpoch = (value: string): boolean =>
    typeof value === "string" &&
    /^[0-9a-f]{32}$/.test(value) &&
    !/^0+$/.test(value);
const view = (bytes: Uint8Array): DataView =>
    new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
function readEpoch(bytes: Uint8Array): string {
    const epoch = Array.from(bytes, (v) =>
        v.toString(16).padStart(2, "0"),
    ).join("");
    return validEpoch(epoch) ? epoch : invalid();
}

/** Ordinary native RTP never accepts a flagged epoch or retries RTCP as RTP.
 * The generic RTP parser remains available for other explicitly framed uses. */
export function decodeNativeRtpDatagram(bytes: Uint8Array): RtpDatagram {
    if (bytes[1] !== 0 || (bytes[9] >= 192 && bytes[9] <= 223))
        return invalid();
    return decodeRtpDatagram(bytes);
}

/** Call only for an explicitly selected video profile. This parser does not
 * join the opaque packet tag to a signed notice or grant decoder admission. */
export function decodeNativeVideoEpoch(
    bytes: Uint8Array,
): NativeVideoEpochPacket {
    if (
        bytes.length < 36 ||
        bytes.length > MEDIA_LIMITS.rtpBytes + 24 ||
        bytes[0] !== 1 ||
        bytes[1] !== 1
    )
        return invalid();
    const epoch = readEpoch(bytes.subarray(8, 24));
    // The original RTP body is preserved, including all extension/padding bytes.
    // Only the native framing is removed before the ordinary strict parser.
    const original = new Uint8Array(bytes.length - 16);
    original.set(bytes.subarray(0, 8));
    original[1] = 0;
    original.set(bytes.subarray(24), 8);
    const packet = decodeNativeRtpDatagram(original);
    if (!packet.ssrc) return invalid();
    return Object.freeze({ epoch, packet });
}

/** Exactly one reduced-size PLI and matching CNEP APP member (48 bytes).
 * Bare PLI, padding, extra members and foreign APP senders are refused. */
export function decodeNativeVideoFeedback(
    bytes: Uint8Array,
): NativeVideoFeedbackPacket {
    if (
        bytes.length !== 48 ||
        bytes[0] !== 1 ||
        bytes[1] !== 0 ||
        bytes[8] !== 0x81 ||
        bytes[9] !== 206 ||
        bytes[10] !== 0 ||
        bytes[11] !== 2 ||
        bytes[20] !== 0x80 ||
        bytes[21] !== 204 ||
        bytes[22] !== 0 ||
        bytes[23] !== 6 ||
        bytes[28] !== 67 ||
        bytes[29] !== 78 ||
        bytes[30] !== 69 ||
        bytes[31] !== 80
    )
        return invalid();
    const input = view(bytes);
    const handle = input.getUint16(2),
        senderSsrc = input.getUint32(12),
        mediaSsrc = input.getUint32(16);
    if (
        !handle ||
        !senderSsrc ||
        !mediaSsrc ||
        input.getUint32(24) !== senderSsrc
    )
        return invalid();
    return Object.freeze({
        handle,
        datagramSequence: input.getUint32(4),
        senderSsrc,
        mediaSsrc,
        epoch: readEpoch(bytes.subarray(32)),
    });
}

/** Syntax only; the owner must capture the exact current target before queueing
 * and consume its sequence even when write completion is uncertain. */
export function encodeNativeVideoFeedback(
    value: NativeVideoFeedbackPacket,
): Uint8Array {
    if (
        !uint(value.handle, 65535, 1) ||
        !uint(value.datagramSequence, 0xffffffff) ||
        !uint(value.senderSsrc, 0xffffffff, 1) ||
        !uint(value.mediaSsrc, 0xffffffff, 1) ||
        !validEpoch(value.epoch)
    )
        return invalid();
    const bytes = new Uint8Array(48),
        output = view(bytes);
    bytes[0] = 1;
    output.setUint16(2, value.handle);
    output.setUint32(4, value.datagramSequence);
    bytes.set([0x81, 206, 0, 2], 8);
    output.setUint32(12, value.senderSsrc);
    output.setUint32(16, value.mediaSsrc);
    bytes.set([0x80, 204, 0, 6], 20);
    output.setUint32(24, value.senderSsrc);
    bytes.set([67, 78, 69, 80], 28);
    for (let i = 0; i < 16; i++)
        bytes[32 + i] = parseInt(value.epoch.slice(i * 2, i * 2 + 2), 16);
    return bytes;
}
