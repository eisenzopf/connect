/** Numeric codec/decoder observations; excludes SDP, addresses and media. */
export interface VideoDiagnostics {
    direction: "inbound" | "outbound";
    codec?: string;
    frames: number;
    keyframes: number;
    width: number;
    height: number;
    packets: number;
    bytes: number;
    lost: number;
    jitter: number;
    timestamp: number;
    /** Actual encoder/pacer observations; absent when the browser does not report them. */
    encodeSeconds?: number;
    packetSendDelaySeconds?: number;
    framesPerSecond?: number;
    qualityLimit?: "none" | "cpu" | "bandwidth" | "other";
    capturedFrames?: number;
    capturedFramesPerSecond?: number;
    targetBitrate?: number;
    remotePacketsLost?: number;
    remoteFractionLost?: number;
    remoteRoundTripTimeSeconds?: number;
    availableOutgoingBitrate?: number;
    /** Bounded owned-worker metadata; absent if the 250ms snapshot is unavailable. */
    worker?: VideoWorkerDiagnostics;
    /** Selected ICE ports for local path diagnosis; no addresses or credentials. */
    selectedRemotePort?: number;
    selectedLocalPort?: number;
    selectedTransport?: "udp" | "tcp";
    selectedCandidateType?: "host" | "srflx" | "prflx" | "relay";
}
export interface VideoWorkerDiagnostics {
    observedFrames: number;
    observedKeyframes: number;
    keyframeRequests: number;
    keyframeCompletions: number;
    keyframeFailures: number;
    keyframeRequestPending: boolean;
    pendingRequestSeconds: number;
}
export declare function videoDiagnostics(peer: RTCPeerConnection, worker?: Worker): Promise<Array<VideoDiagnostics>>;
