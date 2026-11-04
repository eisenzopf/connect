import { type RtpDatagram } from "./browser_rtp.js";
export declare const VIDEO_FEEDBACK_PROFILE = "rtcp-pli-epoch/1";
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
/** Ordinary native RTP never accepts a flagged epoch or retries RTCP as RTP.
 * The generic RTP parser remains available for other explicitly framed uses. */
export declare function decodeNativeRtpDatagram(bytes: Uint8Array): RtpDatagram;
/** Call only for an explicitly selected video profile. This parser does not
 * join the opaque packet tag to a signed notice or grant decoder admission. */
export declare function decodeNativeVideoEpoch(bytes: Uint8Array): NativeVideoEpochPacket;
/** Exactly one reduced-size PLI and matching CNEP APP member (48 bytes).
 * Bare PLI, padding, extra members and foreign APP senders are refused. */
export declare function decodeNativeVideoFeedback(bytes: Uint8Array): NativeVideoFeedbackPacket;
/** Syntax only; the owner must capture the exact current target before queueing
 * and consume its sequence even when write completion is uncertain. */
export declare function encodeNativeVideoFeedback(value: NativeVideoFeedbackPacket): Uint8Array;
