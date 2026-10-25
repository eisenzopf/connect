export interface RtcpSenderReport {
    ssrc: number;
    ntp: bigint;
    rtpTimestamp: number;
    packets: number;
    octets: number;
    cname: string;
}
/** One sender report and its matching CNAME; no receive report blocks. */
export declare function encodeRtcpSenderReport(report: Readonly<RtcpSenderReport>): Uint8Array;
/** Parse a non-reduced-size clock compound. Unknown bounded RTCP packets are
 * ignored; they cannot invoke feedback, media or application control here. */
export declare function decodeRtcpSenderReport(bytes: Uint8Array): RtcpSenderReport;
/** The profile's fixed Opus/VP8 payload types do not collide with RFC 5761's
 * RTCP range. Classification alone performs no parsing, routing or admission. */
export declare function isRtcpDatagram(packet: Uint8Array): boolean;
export declare function wrapRtcpSenderReport(handle: number, sequence: number, report: Readonly<RtcpSenderReport>): Uint8Array;
/** Convert Unix microseconds into the wrapping NTP64 era. */
export declare function unixMicrosecondsToNtp(value: bigint): bigint;
/** Sampling-time interpolation only. A zero report clock explicitly stays
 * unavailable; neither this helper nor a caller's timing data grants media IO. */
export declare function ntpAtRtpTimestamp(report: Readonly<RtcpSenderReport>, timestamp: number, clockRate: 8000 | 48000 | 90000): bigint | undefined;
