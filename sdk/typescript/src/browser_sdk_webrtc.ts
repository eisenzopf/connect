import { captureAudioNode } from "./browser_audio_source.js";
import {
    boundedCanvasVideoCapture,
    captureVideoCanvas,
    type CanvasVideoCapture,
} from "./browser_video_capture.js";
import {
    observeWebRtcDecodedReadiness,
    type WebRtcPreparedDecoder,
} from "./browser_webrtc_readiness.js";
import {
    replacementRoot,
    type BrowserReplacementRoot,
} from "./browser_replacement.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { BrowserTasks } from "./browser_tasks.js";
import { CanonicalWebRtcSignaling } from "./browser_webrtc_signaling.js";
import { browserIceConfiguration } from "./browser_ice.js";
import { captureDevices, validateBrowserAnswer } from "./browser_capture.js";
import { configureReceiverBuffer } from "./browser_receiver.js";
import {
    startVideoTransform,
    stopVideoWorker,
    constrainVideoCapture,
    configureVideoSender,
} from "./browser_video_setup.js";
import { ConnectError } from "./errors.js";
import type { BrowserConnectionOptions } from "./browser_media.js";
import type {
    BrowserCaptureHandle,
    BrowserCaptureInput,
    BrowserRenderHandle,
    BrowserRenderRetirement,
} from "./browser_sdk.js";
import type { BrowserMediaCaptureOptions } from "./browser_media_capture.js";

export interface BrowserRtcConfiguration {
    readonly iceServers?: readonly RTCIceServer[];
    readonly iceTransportPolicy?: RTCIceTransportPolicy;
}
export type BrowserRtcCaptureInput = BrowserCaptureInput;

const unsupported = () =>
    new ConnectError(
        "unsupported-media-profile",
        "The original WebRTC media profile is unavailable",
    );
const closed = () => new ConnectError("closed", "WebRTC media owner retired");
function timeout(options: RequestOptions): number {
    const n = options.timeoutMs ?? 10000;
    if (!Number.isFinite(n) || n < 1 || n > 60000)
        throw new ConnectError(
            "invalid-request",
            "Use a bounded media deadline",
        );
    return n;
}
/** One current physical's RTC owner. No capture permission is requested by offer. */
export class BrowserRtcConnection {
    private readonly tasks = new BrowserTasks();
    private stopped = false;
    private readonly stop = new AbortController();
    private readonly retired = new Set<(error?: Error) => void>();
    private readonly senders = new Map<string, RTCRtpTransceiver>();
    private readonly remote = new Map<string, MediaStreamTrack>();
    private signaling?: CanonicalWebRtcSignaling;
    private worker?: Worker;
    private stopControl?: () => void;
    private stopEvents?: () => void;
    private captureOwner?: BrowserCaptureHandle;
    private renderOwner?: BrowserRenderHandle;
    private preparedDecoder?: WebRtcPreparedDecoder;
    private replacementCompletion?: Promise<unknown>;
    private replacementVideoSource?: MediaStreamTrack;
    private replacementVideoPresented = false;
    private readonly peer: RTCPeerConnection;
    private constructor(
        private readonly client: BrowserClient,
        readonly input: BrowserConnectionOptions,
        participantId: string,
        configuration: BrowserRtcConfiguration,
        replacement?: BrowserReplacementRoot,
    ) {
        const video = input.video ?? input.screen;
        if (client.closed) throw closed();
        if (
            client.controlTransport !== "websocket" ||
            typeof RTCPeerConnection !== "function" ||
            typeof RTCRtpSender === "undefined" ||
            typeof RTCRtpSender.getCapabilities !== "function" ||
            !input.audio ||
            (input.screen !== undefined && input.video !== undefined) ||
            input.data?.length ||
            (input.receiveVideo !== undefined &&
                (!input.video ||
                    input.screen ||
                    input.receiveVideo === input.video ||
                    input.receiveVideo === input.audio))
        )
            throw unsupported();
        const ice = browserIceConfiguration(
            configuration.iceServers,
            configuration.iceTransportPolicy,
        );
        this.peer = new RTCPeerConnection(ice);
        try {
            if (
                ice?.iceTransportPolicy === "relay" &&
                this.peer.getConfiguration().iceTransportPolicy !== "relay"
            )
                throw unsupported();
            const audio = this.peer.addTransceiver("audio", {
                direction: "sendrecv",
            });
            const audioCodecs = RTCRtpSender.getCapabilities(
                "audio",
            )?.codecs.filter((v) => v.mimeType.toLowerCase() === "audio/opus");
            if (!audioCodecs?.length) throw unsupported();
            audio.setCodecPreferences(audioCodecs);
            this.senders.set(input.audio, audio);
            if (video) {
                const transceiver = this.peer.addTransceiver("video", {
                    direction: "sendrecv",
                    sendEncodings: [
                        {
                            maxFramerate: 30,
                            maxBitrate: 1_000_000,
                            scaleResolutionDownBy: 1,
                        },
                    ],
                });
                const encoding = transceiver.sender.getParameters().encodings;
                if (
                    transceiver.sender.track !== null ||
                    encoding?.length !== 1 ||
                    encoding[0].maxFramerate !== 30 ||
                    encoding[0].maxBitrate !== 1_000_000 ||
                    encoding[0].scaleResolutionDownBy !== 1
                )
                    throw unsupported();
                this.senders.set(video, transceiver);
                this.worker = startVideoTransform(this.peer, transceiver);
                this.worker.onerror = () => this.close(unsupported());
                this.worker.onmessage = () => this.close(unsupported());
            }
            configureReceiverBuffer(this.peer);
            this.peer.ontrack = (event) => {
                if (this.stopped) return;
                const selected = [...this.senders].find(
                    ([, v]) => v === event.transceiver,
                );
                if (
                    !selected ||
                    event.track.kind !==
                        (selected[0] === input.audio ? "audio" : "video") ||
                    (this.remote.has(selected[0]) &&
                        this.remote.get(selected[0]) !== event.track)
                ) {
                    this.close(
                        new ConnectError(
                            "protocol",
                            "Remote track exceeds original Connection scope",
                        ),
                    );
                    return;
                }
                const receiveId =
                    selected[0] === input.audio
                        ? selected[0]
                        : (input.receiveVideo ?? selected[0]);
                if (
                    this.remote.has(receiveId) &&
                    this.remote.get(receiveId) !== event.track
                ) {
                    this.close(
                        new ConnectError(
                            "protocol",
                            "Remote track changed the original receive port",
                        ),
                    );
                    return;
                }
                this.remote.set(receiveId, event.track);
            };
            this.signaling = new CanonicalWebRtcSignaling(client, this.peer, {
                conversationId: input.conversationId,
                sessionId: input.sessionId,
                connectionId: input.connectionId,
                operationId: input.operationId,
                expectedRevision: 0,
                purpose: input.purpose,
                replacement,
                participantId,
                capabilities: {
                    audio_codecs: [
                        {
                            name: "opus",
                            params: { sample_rate: 48000, channels: 1 },
                        },
                    ],
                    video_codecs: video
                        ? [
                              {
                                  name: "vp8",
                                  params: { sample_rate: 90000, channels: 0 },
                              },
                          ]
                        : [],
                    max_streams_per_connection: video
                        ? input.receiveVideo
                            ? 3
                            : 2
                        : 1,
                    identity_assurance_offered: "identified",
                },
                streams: [
                    {
                        id: input.audio,
                        kind: "audio",
                        direction: "sendrecv",
                        codec_preferences: ["opus"],
                    },
                    ...(video
                        ? [
                              {
                                  id: video,
                                  kind: input.screen
                                      ? ("screen" as const)
                                      : ("video" as const),
                                  direction: input.receiveVideo
                                      ? ("sendonly" as const)
                                      : ("sendrecv" as const),
                                  codec_preferences: ["vp8"],
                              },
                          ]
                        : []),
                    ...(input.receiveVideo
                        ? [
                              {
                                  id: input.receiveVideo,
                                  kind: "video" as const,
                                  direction: "recvonly" as const,
                                  codec_preferences: ["vp8"],
                              },
                          ]
                        : []),
                ],
            });
            void this.signaling.finished.then(
                () => this.close(),
                (error) =>
                    this.close(error instanceof Error ? error : closed()),
            );
            this.stopControl = client.onClose((error) => this.close(error));
            this.stopEvents = client.onEvent((frame) => {
                if (
                    frame.cid === input.conversationId &&
                    (frame.type === "conversation.closed" ||
                        (frame.sid === input.sessionId &&
                            (frame.type === "session.ended" ||
                                (frame.type === "connection.end" &&
                                    frame.connid === input.connectionId))))
                )
                    this.close();
            });
        } catch (error) {
            this.close();
            throw error;
        }
    }
    static begin(
        client: BrowserClient,
        input: BrowserConnectionOptions,
        participantId: string,
        configuration: BrowserRtcConfiguration,
        options: RequestOptions,
    ) {
        const budget = new BrowserDeadline(timeout(options));
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Media offer cancelled before preparation",
            );
        const captured = { ...input, data: input.data?.map((v) => ({ ...v })) };
        const owner = new BrowserRtcConnection(
            client,
            captured,
            participantId,
            configuration,
        );
        const signal = AbortSignal.any([
            owner.stop.signal,
            ...(options.signal ? [options.signal] : []),
        ]);
        const accepted = owner
            .signaling!.offer({ timeoutMs: budget.remaining(), signal })
            .then(() => {
                owner.check();
                validateBrowserAnswer(
                    owner.peer.remoteDescription?.sdp ?? "",
                    !!(input.video ?? input.screen),
                );
            });
        const ready = accepted
            .then(async () => {
                await owner.connected(budget, signal);
                owner.check();
                return owner;
            })
            .catch((error) => {
                owner.close(error instanceof Error ? error : closed());
                throw error;
            });
        void accepted.catch(() => {});
        void ready.catch(() => {});
        return {
            accepted,
            ready,
            owner,
            get submissionAttempted() {
                return (
                    owner.signaling?.savedOffer?.submissionAttempted ?? false
                );
            },
        };
    }
    /** Prepare a second physical without retiring the source. Completion needs
     * this fresh decoder's evidence and the original committed movement receipt. */
    static beginReplacement(
        client: BrowserClient,
        input: BrowserConnectionOptions,
        participantId: string,
        configuration: BrowserRtcConfiguration,
        original: BrowserReplacementRoot,
        options: RequestOptions,
    ) {
        const root = replacementRoot(original);
        if (
            input.purpose !== undefined ||
            !input.audio ||
            input.screen ||
            input.data?.length ||
            input.operationId !== root.operationId ||
            input.conversationId !== root.conversationId ||
            input.sessionId !== root.sessionId ||
            input.connectionId !== root.targetConnectionId ||
            (input.video !== undefined) !== (input.receiveVideo !== undefined)
        )
            throw new ConnectError(
                "invalid-request",
                "Use the original target and directional replacement Video ports",
            );
        const budget = new BrowserDeadline(timeout(options));
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Replacement cancelled before physical preparation",
            );
        const owner = new BrowserRtcConnection(
            client,
            { ...input },
            participantId,
            configuration,
            root,
        );
        const signal = AbortSignal.any([
            owner.stop.signal,
            ...(options.signal ? [options.signal] : []),
        ]);
        const prepared = owner.tasks.track(
            owner
                .signaling!.offer({ timeoutMs: budget.remaining(), signal })
                .then(async () => {
                    owner.check();
                    validateBrowserAnswer(
                        owner.peer.remoteDescription?.sdp ?? "",
                        !!input.video,
                    );
                    await owner.connected(budget, signal);
                    owner.check();
                    return owner;
                })
                .catch((error) => {
                    owner.close(error instanceof Error ? error : closed());
                    throw error;
                }),
        );
        const completed = owner.tasks.track(
            prepared
                .then(async () => {
                    const audio = owner.remote.get(input.audio!);
                    const video = input.receiveVideo
                        ? owner.remote.get(input.receiveVideo)
                        : undefined;
                    if (!audio || (input.receiveVideo && !video))
                        throw unsupported();
                    const evidence = await observeWebRtcDecodedReadiness(
                        owner.peer,
                        { id: input.audio!, track: audio },
                        video
                            ? { id: input.receiveVideo!, track: video }
                            : undefined,
                        budget,
                        signal,
                        (work, cleanup) => owner.tasks.track(work, cleanup),
                        (release) => owner.retainMutedDecoderSinks(release),
                    );
                    owner.check();
                    return owner.signaling!.completeReplacement(evidence, {
                        timeoutMs: budget.remaining(),
                        signal,
                    });
                })
                .catch((error) => {
                    // Once ready was submitted, a missing reply can mean the original
                    // move committed. Keep its target alive for original-root recovery.
                    if (!owner.replacementMayHaveCommitted)
                        owner.close(error instanceof Error ? error : closed());
                    throw error;
                }),
        );
        // This resolves only after completeReplacement validated the original
        // completed/ready Root and its exact target/generation/stream binding.
        owner.replacementCompletion = completed;
        void prepared.catch(() => {});
        void completed.catch(() => {});
        return {
            prepared,
            completed,
            owner,
            get submissionAttempted() {
                return (
                    owner.signaling?.savedOffer?.submissionAttempted ?? false
                );
            },
        };
    }
    get diagnostics() {
        return Object.freeze({
            closed: this.stopped,
            peerConnectionState: this.peer.connectionState,
            remoteTracks: this.remote.size,
            liveRemoteTracks: [...this.remote.values()].filter(
                (track) => track.readyState === "live",
            ).length,
            captureActive: !!this.captureOwner && !this.captureOwner.closed,
            renderActive:
                !!this.renderOwner && !this.renderOwner.diagnostics.closed,
        });
    }
    get replacementMayHaveCommitted(): boolean {
        return this.signaling?.replacementMayHaveCommitted ?? false;
    }
    private retainMutedDecoderSinks(decoder: WebRtcPreparedDecoder): void {
        this.check();
        if (this.preparedDecoder)
            throw new ConnectError("conflict", "Decoder sinks already owned");
        this.preparedDecoder = decoder;
    }
    private retireMutedDecoderSinks(): void {
        const decoder = this.preparedDecoder;
        this.preparedDecoder = undefined;
        if (decoder)
            void this.tasks.track(decoder.release(), true).catch(() => {});
    }
    private check() {
        if (this.stopped || this.client.closed) throw closed();
    }
    private async connected(
        budget: BrowserDeadline,
        signal: AbortSignal,
    ): Promise<void> {
        await budget.run(
            () =>
                new Promise<void>((resolve, reject) => {
                    const check = () => {
                        if (
                            signal.aborted ||
                            this.stopped ||
                            ["failed", "closed"].includes(
                                this.peer.connectionState,
                            )
                        ) {
                            cleanup();
                            reject(closed());
                        } else if (this.peer.connectionState === "connected") {
                            cleanup();
                            resolve();
                        }
                    };
                    const cleanup = () => {
                        this.peer.removeEventListener(
                            "connectionstatechange",
                            check,
                        );
                        signal.removeEventListener("abort", check);
                    };
                    this.peer.addEventListener("connectionstatechange", check);
                    signal.addEventListener("abort", check, { once: true });
                    check();
                }),
            signal,
        );
    }
    onClose(fn: (error?: Error) => void): () => void {
        return this.onRetire(fn);
    }
    restartIce(
        operationId: string,
        expectedStreamRevision: number,
        expectedIceGeneration: number,
        options: RequestOptions = {},
    ) {
        this.check();
        if (!this.signaling) throw closed();
        return this.signaling.restartIce(
            operationId,
            expectedStreamRevision,
            expectedIceGeneration,
            options,
        );
    }
    get savedIceRestart() {
        return this.signaling?.savedIceRestart;
    }
    private onRetire(fn: (error?: Error) => void): () => void {
        this.check();
        if (this.retired.size >= 16) throw unsupported();
        this.retired.add(fn);
        return () => this.retired.delete(fn);
    }
    async capture(
        input: BrowserRtcCaptureInput,
        options: RequestOptions,
    ): Promise<BrowserCaptureHandle> {
        this.check();
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Capture cancelled before permission",
            );
        if (this.captureOwner && !this.captureOwner.closed)
            throw new ConnectError("conflict", "Capture already owned");
        const canvasSource = captureVideoCanvas(input, input.video),
            selected = {
                ...input,
                audioNode: undefined,
                videoCanvas: undefined,
            },
            audio = !!selected.audio,
            video = selected.video !== undefined;
        let canvasCapture: CanvasVideoCapture | undefined;
        if (
            (!audio && !video) ||
            (audio && !this.input.audio) ||
            (video &&
                !(selected.video === "screen"
                    ? this.input.screen
                    : this.input.video))
        )
            throw unsupported();
        let graphSource = captureAudioNode(input, audio);
        let graphContext = graphSource?.context as AudioContext | undefined;
        let graphDestination: MediaStreamAudioDestinationNode | undefined,
            graphConnected = false;
        const devices = captureDevices(selected, video, !!selected.stream);
        if (selected.video === "screen" && selected.videoDeviceId !== undefined)
            throw new ConnectError(
                "invalid-request",
                "Screen choice requires browser permission",
            );
        const budget = new BrowserDeadline(timeout(options)),
            abort = new AbortController(),
            signal = AbortSignal.any([
                abort.signal,
                this.stop.signal,
                ...(options.signal ? [options.signal] : []),
            ]);
        let stopped = false,
            paused = false,
            replacing = false;
        const streams = new Set<MediaStream>(),
            enabled = new Map<MediaStreamTrack, boolean>(),
            events = new AbortController();
        let detach: () => void = () => {};
        const close = (error?: Error) => {
            if (stopped) return;
            stopped = true;
            abort.abort();
            detach();
            events.abort();
            graphContext?.removeEventListener("statechange", graphState);
            disconnectGraph();
            graphSource = undefined;
            graphDestination = undefined;
            graphContext = undefined;
            document.removeEventListener("visibilitychange", visibility);
            if (canvasCapture) {
                streams.delete(canvasCapture.stream);
                canvasCapture.close();
                canvasCapture = undefined;
            }
            for (const stream of streams)
                for (const track of stream.getTracks()) {
                    track.stop();
                }
            streams.clear();
            enabled.clear();
            if (this.captureOwner === handle) this.captureOwner = undefined;
            selected.state?.("closed", error);
        };
        const pause = () => {
            if (stopped || paused) return;
            paused = true;
            disconnectGraph();
            for (const stream of streams)
                for (const track of stream.getTracks()) {
                    enabled.set(track, track.enabled);
                    track.enabled = false;
                }
            selected.state?.("paused");
        };
        const disconnectGraph = () => {
            if (!graphConnected) return;
            graphConnected = false;
            if (graphSource && graphDestination) {
                try {
                    graphSource.disconnect(graphDestination);
                } catch {
                    /* Caller may already have removed this edge. */
                }
            }
        };
        const graphState = () => {
            if (!graphContext || stopped || graphContext.state === "running")
                return;
            if (graphContext.state === "closed")
                close(
                    new ConnectError(
                        "media-capture",
                        "Caller audio graph closed",
                    ),
                );
            else pause();
        };
        const visibility = () => {
            if (document.visibilityState === "hidden") pause();
        };
        const handle: BrowserCaptureHandle = Object.freeze({
            get closed() {
                return stopped;
            },
            pause,
            resume: async () => {
                this.check();
                if (stopped) throw closed();
                if (document.visibilityState === "hidden") return;
                if (graphContext && graphContext.state !== "running")
                    throw new ConnectError(
                        "media-capture",
                        "Resume the caller audio context before capture",
                    );
                if (graphSource && graphDestination && !graphConnected) {
                    graphSource.connect(graphDestination);
                    graphConnected = true;
                }
                for (const [track, was] of enabled) track.enabled = was;
                enabled.clear();
                paused = false;
                selected.state?.("active");
            },
            close: () => close(),
        });
        const cancelled = () =>
            close(new ConnectError("cancelled", "Capture cancelled"));
        signal.addEventListener("abort", cancelled, { once: true });
        const own = (stream: MediaStream) => {
            if (stopped || signal.aborted) {
                for (const track of stream.getTracks()) track.stop();
                throw closed();
            }
            streams.add(stream);
        };
        const permission = async (work: Promise<MediaStream>) => {
            void work.then(
                (stream) => {
                    if (stopped || signal.aborted)
                        for (const track of stream.getTracks()) track.stop();
                },
                () => {},
            );
            own(await budget.run(() => work, signal));
        };
        try {
            detach = this.onRetire(close);
            this.captureOwner = handle;
            if (signal.aborted) cancelled();
            if (stopped) throw closed();
            if (graphSource && graphContext) {
                graphContext.addEventListener("statechange", graphState);
                if (graphContext.state !== "running") throw unsupported();
                graphDestination = graphContext.createMediaStreamDestination();
                graphDestination.channelCount = 1;
                own(graphDestination.stream);
                graphSource.connect(graphDestination);
                graphConnected = true;
            }
            if (selected.stream) own(selected.stream);
            else {
                if (selected.video === "screen")
                    await permission(
                        navigator.mediaDevices.getDisplayMedia({
                            video: {
                                width: { ideal: 640, max: 640 },
                                height: { ideal: 360, max: 360 },
                                frameRate: { ideal: 30, max: 30 },
                            },
                            audio: false,
                        }),
                    );
                if (
                    (audio && !graphSource) ||
                    (selected.video === "camera" && !canvasSource)
                )
                    await permission(
                        navigator.mediaDevices.getUserMedia({
                            audio:
                                audio && !graphSource
                                    ? {
                                          sampleRate: { ideal: 48000 },
                                          channelCount: { ideal: 1, max: 2 },
                                          ...(devices.audioDeviceId
                                              ? {
                                                    deviceId: {
                                                        exact: devices.audioDeviceId,
                                                    },
                                                }
                                              : {}),
                                      }
                                    : false,
                            video:
                                selected.video === "camera" && !canvasSource
                                    ? {
                                          width: { ideal: 640, max: 640 },
                                          height: { ideal: 360, max: 360 },
                                          frameRate: { ideal: 30, max: 30 },
                                          ...(devices.videoDeviceId
                                              ? {
                                                    deviceId: {
                                                        exact: devices.videoDeviceId,
                                                    },
                                                }
                                              : {}),
                                      }
                                    : false,
                        }),
                    );
            }
            if (canvasSource) {
                canvasCapture = boundedCanvasVideoCapture(
                    new MediaStream(),
                    canvasSource,
                    close,
                );
                own(canvasCapture.stream);
            }
            const tracks = [...streams].flatMap((stream) => stream.getTracks());
            if (
                tracks.length !== Number(audio) + Number(video) ||
                tracks.filter((t) => t.kind === "audio").length !==
                    Number(audio) ||
                tracks.filter((t) => t.kind === "video").length !==
                    Number(video) ||
                tracks.some((t) => t.readyState !== "live")
            )
                throw unsupported();
            if (video && !canvasCapture) {
                const stream = [...streams].find(
                    (v) => v.getVideoTracks().length,
                )!;
                await budget.run(
                    () => constrainVideoCapture(stream, budget.remaining()),
                    signal,
                );
            }
            for (const track of tracks) {
                track.addEventListener("ended", () => close(closed()), {
                    once: true,
                    signal: events.signal,
                });
                const id =
                    track.kind === "audio"
                        ? this.input.audio
                        : selected.video === "screen"
                          ? this.input.screen
                          : this.input.video;
                replacing = true;
                await budget.run(
                    () =>
                        this.tasks.track(
                            this.senders.get(id!)!.sender.replaceTrack(track),
                        ),
                    signal,
                );
                replacing = false;
                if (stopped || signal.aborted) throw closed();
            }
            if (video)
                await budget.run(() => configureVideoSender(this.peer), signal);
            this.check();
            if (stopped) throw closed();
            document.addEventListener("visibilitychange", visibility);
            visibility();
            selected.state?.(paused ? "paused" : "active");
            return handle;
        } catch (error) {
            const failure =
                error instanceof ConnectError
                    ? error
                    : new ConnectError(
                          error instanceof DOMException &&
                              error.name === "NotAllowedError"
                              ? "media-denied"
                              : "media",
                          "Owned WebRTC capture failed",
                      );
            close(failure);
            if (replacing) this.close(failure);
            throw failure;
        } finally {
            if (stopped) signal.removeEventListener("abort", cancelled);
        }
    }
    async render(
        input: {
            audio?: boolean;
            video?: Readonly<Record<string, HTMLCanvasElement>>;
            volume?: number;
        },
        options: RequestOptions,
    ): Promise<BrowserRenderHandle> {
        this.check();
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Playback cancelled before startup",
            );
        if (this.renderOwner && !this.renderOwner.diagnostics.closed)
            throw new ConnectError("conflict", "Renderer already owned");
        const volume = input.volume ?? 1,
            audio = input.audio !== false,
            video = { ...input.video };
        if (
            !Number.isFinite(volume) ||
            volume < 0 ||
            volume > 1 ||
            Object.keys(video).length > 8
        )
            throw new ConnectError(
                "invalid-request",
                "Use bounded render options",
            );
        const budget = new BrowserDeadline(timeout(options)),
            stop = new AbortController(),
            signal = AbortSignal.any([
                stop.signal,
                this.stop.signal,
                ...(options.signal ? [options.signal] : []),
            ]);
        let stopped = false,
            terminalFailure: Error | undefined,
            retainPendingDecoder = false,
            paused = false,
            count = 0,
            context: AudioContext | undefined,
            source: MediaStreamAudioSourceNode | undefined,
            gain: GainNode | undefined,
            playout: HTMLAudioElement | undefined,
            detach = () => {};
        const outputs: {
            element: HTMLVideoElement;
            callback?: number;
            rejectFirst?: (error: Error) => void;
            canvas: CanvasRenderingContext2D;
        }[] = [];
        let finish!: () => void, fail!: (error: Error) => void;
        let retired!: (receipt: BrowserRenderRetirement) => void,
            retirementFailed!: (error: unknown) => void;
        const retirement = new Promise<BrowserRenderRetirement>(
            (resolve, reject) => {
                retired = resolve;
                retirementFailed = reject;
            },
        );
        void retirement.catch(() => {});
        const finished = new Promise<void>((yes, no) => {
            finish = yes;
            fail = no;
        });
        void finished.catch(() => {});
        const close = (error?: Error) => {
            if (stopped) return;
            // Preserve the FIRST causal terminal error before internal cleanup
            // aborts the same startup budget. Caller abort still supplies its
            // original cancelled error; a deadline still supplies timeout.
            terminalFailure ??= error;
            stopped = true;
            // A valid pending original-root read creates NO public output.
            // Its original target still owns the unadopted private decoder.
            if (!retainPendingDecoder) this.retireMutedDecoderSinks();
            detach();
            stop.abort();
            document.removeEventListener("visibilitychange", visibility);
            if (playout) {
                playout.pause();
                playout.srcObject = null;
            }
            for (const output of outputs) {
                output.rejectFirst?.(error ?? closed());
                output.rejectFirst = undefined;
                if (output.callback !== undefined)
                    output.element.cancelVideoFrameCallback(output.callback);
                output.element.pause();
                output.element.srcObject = null;
            }
            source?.disconnect();
            gain?.disconnect();
            if (this.renderOwner === handle) this.renderOwner = undefined;
            if (context) {
                const cleanup = context.close();
                this.tasks.track(cleanup, true);
                void cleanup.then(
                    () => {
                        retired(
                            Object.freeze({
                                connectionId: this.input.connectionId,
                                scope: "renderer-local",
                                audioGraph: "closed",
                                outputsDetached: true,
                            }),
                        );
                        error ? fail(error) : finish();
                    },
                    (cleanupError) => {
                        retirementFailed(cleanupError);
                        fail(
                            new ConnectError(
                                "media",
                                "Playback graph cleanup failed",
                            ),
                        );
                    },
                );
            } else {
                retired(
                    Object.freeze({
                        connectionId: this.input.connectionId,
                        scope: "renderer-local",
                        audioGraph: "not-created",
                        outputsDetached: true,
                    }),
                );
                error ? fail(error) : finish();
            }
        };
        const pause = () => {
            if (stopped || paused) return;
            paused = true;
            void context?.suspend().catch(() => {});
            playout?.pause();
            for (const output of outputs) output.element.pause();
        };
        const visibility = () => {
            if (document.visibilityState === "hidden") pause();
        };
        const handle: BrowserRenderHandle = Object.freeze({
            finished,
            retirement,
            get diagnostics() {
                return Object.freeze({
                    closed: stopped,
                    paused,
                    audio: 0,
                    video: count,
                });
            },
            pause,
            resume: async () => {
                this.check();
                if (stopped) throw closed();
                if (document.visibilityState === "hidden") return;
                const resuming = new BrowserDeadline(timeout(options));
                try {
                    if (context)
                        await resuming.run(() => context!.resume(), signal);
                    await playAudio(resuming);
                } catch (error) {
                    const failure = error instanceof Error ? error : closed();
                    close(failure);
                    throw failure;
                }
                for (const output of outputs)
                    void output.element.play().catch(() => {
                        if (!stopped)
                            close(
                                new ConnectError(
                                    "media",
                                    "Playback could not resume",
                                ),
                            );
                    });
                paused = false;
            },
            close: () => close(),
        });
        const cancelled = () =>
            close(new ConnectError("cancelled", "Playback cancelled"));
        const playAudio = (deadline: BrowserDeadline) =>
            playout
                ? deadline.run(async () => {
                      try {
                          await playout!.play();
                      } catch {
                          throw new ConnectError(
                              "media",
                              "Audio playback failed",
                          );
                      }
                  }, signal)
                : Promise.resolve();
        signal.addEventListener("abort", cancelled, { once: true });
        try {
            detach = this.onRetire(close);
            this.renderOwner = handle;
            if (signal.aborted) cancelled();
            if (stopped) throw closed();
            if (audio) {
                const track = this.remote.get(this.input.audio!);
                if (!track || track.readyState !== "live") throw unsupported();
                const prepared = this.preparedDecoder;
                const adopted = prepared?.takeAudio(track);
                if (prepared && !adopted && !Object.keys(video).length) this.retireMutedDecoderSinks();
                if (adopted) {
                    // Transfer synchronously before an await: after take(),
                    // only this existing render owner may close the graph.
                    context = adopted.context;
                    source = adopted.source;
                    playout = adopted.playout;
                } else {
                    context = new AudioContext({
                        sampleRate: 48000,
                        latencyHint: "interactive",
                    });
                    source = context.createMediaStreamSource(
                        new MediaStream([track]),
                    );
                    // Chrome starts the RTC decoder through this owned muted sink.
                    playout = document.createElement("audio");
                    playout.muted = true;
                    playout.srcObject = new MediaStream([track]);
                }
                gain = context.createGain();
                gain.gain.value = adopted ? 0 : volume;
                source.connect(gain);
                gain.connect(context.destination);
                if (adopted) {
                    source.disconnect(adopted.mutedGain);
                    adopted.mutedGain.disconnect();
                }
                await budget.run(() => context!.resume(), signal);
                await playAudio(budget);
                gain.gain.value = volume;
            }
            // Preserve the original synchronous audio source.connect discovery
            // hook. Only VIDEO ownership/draw is gated here; audio behavior and
            // the observer's synchronous graph scope remain unchanged.
            if (Object.keys(video).length && this.replacementCompletion) {
                await budget.run(async () => {
                    try {
                        await this.replacementCompletion!;
                    } catch (error) {
                        if (!this.signaling) throw error;
                        // Read the SAME original Root after uncertainty. This
                        // cannot resubmit readiness or adopt a foreign result.
                        const receipt = await this.tasks.track(
                            this.signaling.recoverReplacementCompletion(error, {
                                timeoutMs: budget.remaining(), signal,
                            }),
                        );
                        if (!receipt) {
                            retainPendingDecoder = !signal.aborted && !this.stopped;
                            throw new ConnectError("conflict", "Original WebRTC replacement is not completed");
                        }
                        // A late read may settle after CONNECTION retirement;
                        // it must not repopulate that owner's cleared roots.
                        this.check();
                        this.replacementCompletion = Promise.resolve(receipt);
                    }
                }, signal);
                this.check();
                if (stopped) throw closed();
            }
            for (const [id, canvas] of Object.entries(video)) {
                if (
                    id !==
                    (this.input.receiveVideo ??
                        this.input.video ??
                        this.input.screen)
                )
                    throw new ConnectError(
                        "invalid-request",
                        "Render this Connection's receive Stream",
                    );
                const track = this.remote.get(id);
                if (!track || track.readyState !== "live") throw unsupported();
                canvas.width = 640;
                canvas.height = 360;
                const out = canvas.getContext("2d");
                if (!out) throw unsupported();
                const adopted = this.preparedDecoder?.takeVideo(track);
                const replacement = !!this.replacementCompletion;
                if (replacement && !adopted && this.replacementVideoSource !== track) throw unsupported();
                // Consuming a valid lease establishes only exact-track retry
                // eligibility, NEVER successful first public presentation.
                if (adopted) this.replacementVideoSource = track;
                const requirePresentationFloor = !!adopted ||
                    (replacement && !this.replacementVideoPresented);
                const element = adopted?.element ?? document.createElement("video");
                if (!adopted) {
                    element.muted = true;
                    element.autoplay = true;
                    element.playsInline = true;
                    element.srcObject = new MediaStream([track]);
                }
                let firstResolve: (() => void) | undefined;
                let firstReject: ((error: Error) => void) | undefined;
                const first = requirePresentationFloor ? new Promise<void>((resolve, reject) => {
                    firstResolve = resolve;
                    firstReject = reject;
                }) : undefined;
                void first?.catch(() => {});
                const output = {
                    element,
                    canvas: out,
                    callback: undefined as number | undefined,
                    rejectFirst: firstReject,
                };
                outputs.push(output);
                let presented = adopted?.presentedFloor;
                let firstPainted = false;
                const paint: VideoFrameRequestCallback = (_now, metadata) => {
                    if (stopped) return;
                    if (requirePresentationFloor) {
                        if (!Number.isSafeInteger(metadata.presentedFrames) || metadata.presentedFrames <= 0) {
                            close(new ConnectError("media", "Original video presentation counter unavailable"));
                            return;
                        }
                        if (presented === undefined) {
                            // Retried consumed lease or deliberately retired
                            // audio-only picture: this NEW same-track element's
                            // first callback is a baseline only, never a draw.
                            presented = metadata.presentedFrames;
                            output.callback = element.requestVideoFrameCallback(paint);
                            return;
                        }
                        if (metadata.presentedFrames <= presented) {
                            if (firstPainted) {
                                close(new ConnectError("media", "Original video presentation counter did not advance"));
                                return;
                            }
                            output.callback = element.requestVideoFrameCallback(paint);
                            return;
                        }
                        presented = metadata.presentedFrames;
                    }
                    if (!paused && element.readyState >= 2) {
                        out.drawImage(element, 0, 0, 640, 360);
                        count++;
                        firstPainted = true;
                        if (replacement) this.replacementVideoPresented = true;
                        firstResolve?.();
                        firstResolve = undefined;
                        output.rejectFirst = undefined;
                    }
                    output.callback = element.requestVideoFrameCallback(paint);
                };
                output.callback = element.requestVideoFrameCallback(paint);
                void element.play().then(
                    () => {
                        if (!stopped) this.retireMutedDecoderSinks();
                    },
                    () => {
                        if (!stopped) close(new ConnectError("media", "Video playback failed"));
                    },
                );
                // Only the replacement path waits for its first advancing
                // presentation, under the SAME original render budget.
                if (first) await budget.run(() => first, signal);
            }
            this.check();
            if (stopped) throw closed();
            document.addEventListener("visibilitychange", visibility);
            visibility();
            if (!outputs.length) {
                // Deliberate audio-only retirement may permit a later video
                // retry on this EXACT valid prepared track, but it has not
                // established ANY successful public video presentation.
                const id = this.input.receiveVideo ?? this.input.video ?? this.input.screen;
                const track = id ? this.remote.get(id) : undefined;
                if (this.replacementCompletion && track && this.preparedDecoder?.canTakeVideo(track))
                    this.replacementVideoSource = track;
                this.retireMutedDecoderSinks();
            }
            return handle;
        } catch (error) {
            const failure = terminalFailure ?? (
                error instanceof Error
                    ? error
                    : new ConnectError("media", "Playback startup failed"));
            close(failure);
            if (!retainPendingDecoder) this.retireMutedDecoderSinks();
            throw failure;
        }
    }
    close(error?: Error): void {
        if (this.stopped) return;
        this.stopped = true;
        this.replacementVideoSource = undefined;
        this.replacementVideoPresented = false;
        this.replacementCompletion = undefined;
        this.retireMutedDecoderSinks();
        this.stopControl?.();
        this.stopEvents?.();
        for (const fn of [...this.retired])
            try {
                fn(error);
            } catch {
                /* Retire every owner. */
            }
        this.retired.clear();
        this.stop.abort();
        this.peer.ontrack = null;
        this.signaling?.close();
        // A peer-owned receiver can be refused before its ontrack event enters
        // the accepted remote map. Snapshot only this exact physical's actual
        // receivers, never an arbitrary track supplied by a rejected event.
        const remoteTracks = new Set([
            ...this.remote.values(),
            ...this.peer.getTransceivers().map((value) => value.receiver.track),
        ]);
        this.peer.close();
        if (this.worker) stopVideoWorker(this.worker);
        for (const track of remoteTracks) track.stop();
        this.remote.clear();
    }
    async shutdown(): Promise<void> {
        this.close();
        await this.signaling?.shutdown();
        await this.tasks.join();
    }
}
