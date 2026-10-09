/** Safe WebRTC path evidence, excluding credentials, SDP and network addresses. */
export interface VoiceDiagnostics {
    connectionState: RTCPeerConnectionState;
    iceConnectionState: RTCIceConnectionState;
    selectedPair?: {
        id: string;
        localType?: string;
        remoteType?: string;
        protocol?: string;
    };
    audio: Array<{
        direction: "inbound" | "outbound";
        codec?: string;
        packets: number;
        bytes: number;
    }>;
}
export declare function voiceDiagnostics(peer: RTCPeerConnection): Promise<VoiceDiagnostics>;
