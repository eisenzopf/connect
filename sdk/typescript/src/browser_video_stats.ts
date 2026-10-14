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
const workerFields = [
    "observedFrames",
    "observedKeyframes",
    "keyframeRequests",
    "keyframeCompletions",
    "keyframeFailures",
    "keyframeRequestPending",
    "pendingRequestSeconds",
];
const pendingSnapshots = new WeakMap<
    Worker,
    Promise<VideoWorkerDiagnostics | undefined>
>();
function workerSnapshot(
    worker?: Worker,
): Promise<VideoWorkerDiagnostics | undefined> {
    if (!worker) return Promise.resolve(undefined);
    const pending = pendingSnapshots.get(worker);
    if (pending) return pending;
    const snapshot = new Promise<VideoWorkerDiagnostics | undefined>(
        (resolve) => {
            const channel = new MessageChannel();
            let finished = false;
            const finish = (value?: VideoWorkerDiagnostics) => {
                if (finished) return;
                finished = true;
                clearTimeout(timer);
                channel.port1.onmessage = null;
                channel.port1.onmessageerror = null;
                channel.port1.close();
                channel.port2.close();
                resolve(value);
            };
            const timer = setTimeout(() => finish(), 250);
            channel.port1.onmessageerror = () => finish();
            channel.port1.onmessage = ({ data }) => {
                if (
                    !data ||
                    typeof data !== "object" ||
                    Array.isArray(data) ||
                    Object.keys(data).length !== workerFields.length ||
                    Object.keys(data).some(
                        (key) => !workerFields.includes(key),
                    ) ||
                    typeof data.keyframeRequestPending !== "boolean" ||
                    !workerFields
                        .filter((key) => key !== "keyframeRequestPending")
                        .every(
                            (key) =>
                                typeof data[key] === "number" &&
                                Number.isFinite(data[key]) &&
                                data[key] >= 0 &&
                                (key === "pendingRequestSeconds" ||
                                    Number.isSafeInteger(data[key])),
                        )
                ) {
                    finish();
                    return;
                }
                finish({ ...data } as VideoWorkerDiagnostics);
            };
            try {
                worker.postMessage({ op: "video-diagnostics" }, [
                    channel.port2,
                ]);
            } catch {
                finish();
            }
        },
    );
    pendingSnapshots.set(worker, snapshot);
    void snapshot.then(() => pendingSnapshots.delete(worker));
    return snapshot;
}

export async function videoDiagnostics(
    peer: RTCPeerConnection,
    worker?: Worker,
): Promise<Array<VideoDiagnostics>> {
    const [report, workerObservation] = await Promise.all([
        peer.getStats(),
        workerSnapshot(worker),
    ]);
    const result: Array<VideoDiagnostics> = [];
    report.forEach((stat) => {
        if (
            stat.kind !== "video" ||
            !["inbound-rtp", "outbound-rtp"].includes(stat.type)
        )
            return;
        const incoming = stat.type === "inbound-rtp";
        const source = report.get(stat.mediaSourceId);
        const captured =
            !incoming &&
            source?.type === "media-source" &&
            source.kind === "video"
                ? source
                : undefined;
        const remoteReport = report.get(stat.remoteId);
        const remoteInbound =
            !incoming &&
            remoteReport?.type === "remote-inbound-rtp" &&
            remoteReport.kind === "video" &&
            remoteReport.localId === stat.id
                ? remoteReport
                : undefined;
        const pair = report.get(
            report.get(stat.transportId)?.selectedCandidatePairId,
        );
        const remote =
            pair?.state === "succeeded"
                ? report.get(pair.remoteCandidateId)
                : undefined;
        const local =
            pair?.state === "succeeded"
                ? report.get(pair.localCandidateId)
                : undefined;
        const port = (value: unknown): number | undefined =>
            typeof value === "number" &&
            Number.isInteger(value) &&
            value > 0 &&
            value <= 65535
                ? value
                : undefined;
        const observed = (value: unknown): number | undefined =>
            typeof value === "number" && Number.isFinite(value) && value >= 0
                ? value
                : undefined;
        result.push({
            direction: incoming ? "inbound" : "outbound",
            codec: report.get(stat.codecId)?.mimeType,
            frames: (incoming ? stat.framesDecoded : stat.framesEncoded) ?? 0,
            keyframes:
                (incoming ? stat.keyFramesDecoded : stat.keyFramesEncoded) ?? 0,
            width: stat.frameWidth ?? 0,
            height: stat.frameHeight ?? 0,
            packets: (incoming ? stat.packetsReceived : stat.packetsSent) ?? 0,
            bytes: (incoming ? stat.bytesReceived : stat.bytesSent) ?? 0,
            lost: stat.packetsLost ?? 0,
            jitter: stat.jitter ?? 0,
            timestamp: stat.timestamp,
            capturedFrames: Number.isSafeInteger(captured?.frames)
                ? observed(captured.frames)
                : undefined,
            capturedFramesPerSecond: observed(captured?.framesPerSecond),
            targetBitrate: observed(stat.targetBitrate),
            remotePacketsLost:
                typeof remoteInbound?.packetsLost === "number" &&
                Number.isSafeInteger(remoteInbound.packetsLost)
                    ? remoteInbound.packetsLost
                    : undefined,
            remoteFractionLost:
                observed(remoteInbound?.fractionLost) !== undefined &&
                remoteInbound.fractionLost <= 1
                    ? remoteInbound.fractionLost
                    : undefined,
            remoteRoundTripTimeSeconds: observed(remoteInbound?.roundTripTime),
            availableOutgoingBitrate:
                pair?.state === "succeeded"
                    ? observed(pair.availableOutgoingBitrate)
                    : undefined,
            worker: !incoming ? workerObservation : undefined,
            encodeSeconds: observed(stat.totalEncodeTime),
            packetSendDelaySeconds: observed(stat.totalPacketSendDelay),
            framesPerSecond: observed(stat.framesPerSecond),
            qualityLimit: ["none", "cpu", "bandwidth", "other"].includes(
                stat.qualityLimitationReason,
            )
                ? stat.qualityLimitationReason
                : undefined,
            selectedRemotePort: port(remote?.port),
            selectedLocalPort: port(local?.port),
            selectedTransport: ["udp", "tcp"].includes(remote?.protocol)
                ? remote.protocol
                : undefined,
            selectedCandidateType: ["host", "srflx", "prflx", "relay"].includes(
                remote?.candidateType,
            )
                ? remote.candidateType
                : undefined,
        });
    });
    return result;
}
