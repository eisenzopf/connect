export declare const MEDIA_LIMITS: Readonly<{
    rtpBytes: 1180;
    frameBytes: number;
    assemblyMs: 200;
    frames: 8;
    fragments: 64;
    audioPackets: 10;
    videoPackets: 64;
    sendBytes: 76032;
}>;
export interface RtpDatagram {
    flags: number;
    handle: number;
    datagramSequence: number;
    payloadType: number;
    marker: boolean;
    sequence: number;
    timestamp: number;
    ssrc: number;
    csrc: number[];
    extensionProfile?: number;
    padding: number;
    payload: Uint8Array;
}
/** Complete RTP v2 parsing, including bounded RFC 8285 extension elements. */
export declare function decodeRtpDatagram(packet: Uint8Array): RtpDatagram;
export interface MediaBinding {
    handle: number;
    streamId: string;
    connectionId: string;
    sessionId: string;
    generation: number;
    kind: "audio" | "video" | "screen";
    codec: "opus" | "vp8";
    payloadType: number;
    direction: "sendrecv" | "sendonly" | "recvonly";
}
/** Local routing remembers retired physical handles. Server authorization is authoritative. */
export declare class MediaBindings {
    private readonly used;
    private readonly active;
    register(binding: MediaBinding): void;
    retire(handle: number): void;
    retireConnection(id: string): void;
    require(handle: number, direction: "send" | "receive", payloadType?: number): Readonly<MediaBinding>;
    clear(): void;
}
/** Each sender has its own RTP SSRC/sequence and physical datagram sequence. */
export declare class RtpSender {
    private readonly binding;
    private readonly maxDatagramBytes;
    private sequence;
    private datagramSequence;
    private readonly ssrc;
    constructor(binding: Readonly<MediaBinding>, maxDatagramBytes?: number);
    packetize(encoded: Uint8Array, timestamp: number): Uint8Array[];
}
export interface Vp8Frame {
    bytes: Uint8Array;
    timestamp: number;
    ssrc: number;
    key: boolean;
}
/** Loss never produces a partial decoded frame; expired assemblies release all bytes. */
export declare class Vp8Assembler {
    private readonly pending;
    private readonly completed;
    private fragments;
    private remove;
    expire(now?: number): number;
    get queuedFrames(): number;
    get queuedFragments(): number;
    clear(): void;
    push(packet: RtpDatagram, now?: number): Vp8Frame | undefined;
}
/** Drop obsolete media instead of allowing control backpressure to accumulate speech. */
export declare class MediaSendQueue {
    private packets;
    private size;
    droppedPackets: number;
    get queuedBytes(): number;
    get queuedPackets(): number;
    clear(): void;
    private shift;
    push(packets: Uint8Array[], audio: boolean, now?: number): boolean;
    private expire;
    pop(now?: number): Uint8Array | undefined;
}
