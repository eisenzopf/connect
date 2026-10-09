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

export async function voiceDiagnostics(
    peer: RTCPeerConnection,
): Promise<VoiceDiagnostics> {
    const report = await peer.getStats();
    const result: VoiceDiagnostics = {
        connectionState: peer.connectionState,
        iceConnectionState: peer.iceConnectionState,
        audio: [],
    };
    let selectedPairId: string | undefined;
    report.forEach((stat) => {
        if (stat.type === "transport" && stat.selectedCandidatePairId)
            selectedPairId = stat.selectedCandidatePairId;
    });
    report.forEach((stat) => {
        if (
            stat.type === "candidate-pair" &&
            (stat.id === selectedPairId ||
                (!selectedPairId &&
                    stat.nominated &&
                    stat.state === "succeeded"))
        ) {
            const local = report.get(stat.localCandidateId);
            const remote = report.get(stat.remoteCandidateId);
            result.selectedPair = {
                id: stat.id,
                localType: local?.candidateType,
                remoteType: remote?.candidateType,
                protocol: local?.protocol,
            };
        }
        if (stat.kind !== "audio") return;
        if (stat.type === "inbound-rtp" || stat.type === "outbound-rtp") {
            const inbound = stat.type === "inbound-rtp";
            result.audio.push({
                direction: inbound ? "inbound" : "outbound",
                codec: report.get(stat.codecId)?.mimeType,
                packets:
                    (inbound ? stat.packetsReceived : stat.packetsSent) ?? 0,
                bytes: (inbound ? stat.bytesReceived : stat.bytesSent) ?? 0,
            });
        }
    });
    return result;
}
