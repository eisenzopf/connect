import {
    videoPublication,
    completeVideoPublication,
    closeVideoPublication,
    publicationLookup,
    publicationConnection,
    publicationInput,
    takeEncodedPublication,
    currentPublicationOutput,
    publicationPackets,
    currentPublicationPacket,
    settlePublicationPacket,
    type VideoPublication,
} from "./browser_video_publication.js";
import {
    nativeEgressIdentity,
    sameNativeIdentity,
    type NativeEgressIdentity,
    type NativeDecodedObservation,
    type NativeActivationBoundary,
} from "./browser_native_activation.js";
import {
    BrowserNativeReadiness,
    type CurrentNativePreparation,
    type NativeDecodeIdentity,
} from "./browser_native_readiness.js";
import {
    BrowserVideoOrder,
    type OrderedVideoFrame,
} from "./browser_video_order.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { BrowserTasks } from "./browser_tasks.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import {
    BrowserOpusCodec,
    BrowserVp8Codec,
    browserCodecSupport,
    type EncodedMedia,
} from "./browser_codecs.js";
import {
    MediaBindings,
    MediaSendQueue,
    RtpSender,
    Vp8Assembler,
    MEDIA_LIMITS,
    type MediaBinding,
} from "./browser_rtp.js";
import {
    decodeNativeRtpDatagram,
    decodeNativeVideoEpoch,
    decodeNativeVideoFeedback,
    VIDEO_FEEDBACK_PROFILE,
} from "./browser_video_wire.js";
import {
    BrowserVideoEpochs,
    videoAllocation,
    videoNotice,
    type VideoNotice,
    type VideoFeedbackWrite,
} from "./browser_video_epochs.js";
import type { RtpDatagram } from "./browser_rtp.js";
import {
    decodeRtcpSenderReport,
    isRtcpDatagram,
    wrapRtcpSenderReport,
    RtcpCaptureClock,
    RtcpReceiveClock,
    rtpTimestampFromMicroseconds,
} from "./browser_rtcp.js";
import { ConnectError } from "./errors.js";
import {
    canonicalJson,
    jsonObject,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier, type Envelope, type Scope } from "./uctp_wire.js";
import {
    replacementRoot,
    replacementFrame,
    replacementReceipt,
    type BrowserReplacementRoot,
    type BrowserReplacementReceipt,
} from "./browser_replacement.js";

export type BrowserDataProtocol = "text" | "json" | "binary";
export type BrowserDataBody =
    | { type: "text"; value: string }
    | { type: "json"; value: Json }
    | { type: "binary"; value: Uint8Array };
export interface BrowserConnectionOptions {
    conversationId: string;
    sessionId: string;
    connectionId: string;
    operationId: string;
    /** Target-owned audio stays isolated until an attributed handoff commits. */
    purpose?: "handoff-preparation";
    /** Opt into authenticated native Video epochs and bounded keyframe feedback. */
    nativeVideoFeedback?: boolean;
    audio?: string;
    video?: string;
    /** Separate receiver port for movement; when present, video is sendonly. */
    receiveVideo?: string;
    screen?: string;
    data?: { id: string; protocol: BrowserDataProtocol }[];
}
export interface BrowserMediaCallbacks {
    /** Borrowed synchronously; timestamps use the local performance.now() timeline.
     * The SDK closes each decoded frame after this callback. */
    audio?: (
        binding: Readonly<MediaBinding>,
        frame: AudioData,
        native?: NativeDecodedObservation,
    ) => void;
    /** Same local receive timeline and borrowed lifetime as decoded audio. */
    video?: (
        binding: Readonly<MediaBinding>,
        frame: VideoFrame,
        native?: NativeDecodedObservation,
    ) => void;
    data?: (message: {
        conversationId: string;
        sessionId: string;
        connectionId: string;
        streamId: string;
        operationId: string;
        messageId: string;
        from: string;
        body: BrowserDataBody;
    }) => void;
    error?: (error: Error) => void;
}
interface Route {
    publication?: VideoPublication;
    nativeEgress?: NativeEgressIdentity;
    decodedObservation?: NativeDecodedObservation;
    receiveEpoch?: VideoNotice;
    audioSource?: NativeDecodeIdentity;
    binding: Readonly<MediaBinding>;
    sender?: RtpSender;
    opus?: BrowserOpusCodec;
    vp8?: BrowserVp8Codec;
    clock: RtcpReceiveClock;
    captureClock: RtcpCaptureClock;
    lastReport: number;
    clockProfile: boolean;
    assembler?: Vp8Assembler;
    order?: BrowserVideoOrder;
    video?: BrowserVideoEpochs;
    publisherSsrc?: number;
    observed: {
        encoded: number;
        keys: number;
        received: number;
        complete: number;
        submitted: number;
        skipped: number;
        decoded: number;
        expired: number;
        clockReports: number;
        synchronized: number;
        unsynchronized: number;
    };
}
interface ConnectionState {
    nativeReadiness?: BrowserNativeReadiness;
    nativeReplacementCompleted?: boolean;
    scope: Scope & { cid: string; sid: string; connid: string };
    revision: number;
    connectionGeneration: number;
    caps: JsonObject;
    captureClock: RtcpCaptureClock;
    receiveClock: RtcpReceiveClock;
    routes: Map<string, Route>;
    data: Map<string, BrowserDataProtocol>;
    allowed: Map<string, { kind: Json; codec: Json; direction?: Json }>;
    received: Set<string>;
    selectedKinds: Set<string>;
    reconciling?: Promise<void>;
    controlling?: Promise<void>;
    pendingServer?: Envelope;
    prepared?: Plan;
    generations: Map<string, number>;
    closed: Set<() => void>;
}
interface Transition {
    scope: Scope;
    requestId: string;
    revision: number;
    accept: (frame: Envelope) => void;
    fail: (error: unknown) => void;
}
interface ReplacementPreparation {
    audio: string;
    video?: string;
    root: BrowserReplacementRoot;
    prepared: (connection: BrowserMediaConnection) => void;
    committed: (receipt: BrowserReplacementReceipt) => void;
}
interface Plan {
    nativeEgress: Map<number, NativeEgressIdentity>;
    connectionGeneration: number;
    feedback: Map<number, ReturnType<typeof captureAllocation>>;
    notices: Map<string, VideoNotice>;
    clockProfile: boolean;
    media: MediaBinding[];
    data: Map<string, BrowserDataProtocol>;
    revision: number;
    received: Set<string>;
}
const captureAllocation = (
    value: NonNullable<ReturnType<typeof videoAllocation>>,
) => value;
const fail = (message: string): never => {
    throw new ConnectError("protocol", message);
};
const invert = (value: Json | undefined): MediaBinding["direction"] => {
    if (value === "sendonly") return "recvonly";
    if (value === "recvonly") return "sendonly";
    if (value === "sendrecv") return "sendrecv";
    return fail("Invalid negotiated media direction");
};

/** One datagram owner per authenticated physical client; protocol/media state has no UI. */
export class BrowserMediaPeer {
    private readonly tasks = new BrowserTasks();
    private readonly reader: ReadableStreamDefaultReader<Uint8Array>;
    private readonly writer: WritableStreamDefaultWriter<Uint8Array>;
    private readonly registry = new MediaBindings();
    private readonly routes = new Map<number, Route>();
    private readonly connections = new Map<string, ConnectionState>();
    private readonly transitions = new Map<string, Transition>();
    private readonly queue = new MediaSendQueue();
    private readonly feedbackWrites = new Map<Route, VideoFeedbackWrite>();
    private readonly stopEvents: () => void;
    private readonly stopLifetime: () => void;
    private readonly timer: ReturnType<typeof setInterval>;
    private stopped = false;
    private sending = false;
    private readonly mtu: number;
    private counters = {
        sent: 0,
        received: 0,
        rejected: 0,
        decodedAudio: 0,
        decodedVideo: 0,
    };
    constructor(
        private readonly client: BrowserClient,
        private readonly callbacks: BrowserMediaCallbacks = {},
    ) {
        publicationLookup(this, (id, stream) =>
            this.stopped
                ? undefined
                : this.connections.get(id)?.routes.get(stream)?.publication,
        );
        const datagrams = client.datagrams;
        if (
            datagrams.readable.locked ||
            datagrams.writable.locked ||
            !(datagrams.maxDatagramSize >= 64)
        )
            throw new ConnectError(
                "unsupported-media-profile",
                "Media requires exclusively owned negotiated datagrams",
            );
        this.mtu = Math.min(
            datagrams.maxDatagramSize,
            MEDIA_LIMITS.rtpBytes + 8,
        );
        datagrams.incomingMaxAge = datagrams.outgoingMaxAge = 200;
        datagrams.incomingHighWaterMark = datagrams.outgoingHighWaterMark = 64;
        this.reader = datagrams.readable.getReader();
        this.writer = datagrams.writable.getWriter();
        this.stopEvents = client.onEvent((frame) => this.event(frame));
        this.stopLifetime = client.onClose(() => this.close());
        this.timer = setInterval(() => {
            for (const route of this.routes.values()) {
                const expired = route.assembler?.expire() ?? 0;
                route.observed.expired += expired;
                if (expired) route.vp8?.lost();
                route.video?.expire(performance.now());
                this.requestVideoRecovery(route);
            }
        }, 50);
        this.tasks.track(
            this.receive().catch((error: unknown) => this.die(error)),
        );
    }
    get diagnostics(): Readonly<
        typeof this.counters & {
            queuedPackets: number;
            queuedBytes: number;
            droppedPackets: number;
            activeBindings: number;
        }
    > {
        return {
            ...this.counters,
            queuedPackets: this.queue.queuedPackets,
            queuedBytes: this.queue.queuedBytes,
            droppedPackets: this.queue.droppedPackets,
            activeBindings: this.routes.size,
        };
    }
    get routeDiagnostics(): ReadonlyArray<{
        connectionId: string;
        streamId: string;
        direction: MediaBinding["direction"];
        kind: MediaBinding["kind"];
        counters: Readonly<Route["observed"]>;
        clockProfile: boolean;
        incompleteFrames: number;
        fragments: number;
        recovery?: BrowserVp8Codec["decodeDiagnostics"];
        encoding?: BrowserVp8Codec["encodeDiagnostics"];
        audioCodec?: BrowserOpusCodec["diagnostics"];
        ordering?: BrowserVideoOrder["diagnostics"];
    }> {
        return [...this.routes.values()].map((r) => ({
            connectionId: r.binding.connectionId,
            streamId: r.binding.streamId,
            direction: r.binding.direction,
            kind: r.binding.kind,
            counters: { ...r.observed },
            clockProfile: r.clockProfile,
            incompleteFrames: r.assembler?.queuedFrames ?? 0,
            fragments: r.assembler?.queuedFragments ?? 0,
            recovery: r.vp8?.decodeDiagnostics,
            encoding: r.vp8?.encodeDiagnostics,
            audioCodec: r.opus?.diagnostics,
            ordering: r.order?.diagnostics,
        }));
    }
    private check(): void {
        if (this.stopped || this.client.closed)
            throw new ConnectError("closed", "Browser media peer closed");
    }
    private die(value: unknown): void {
        if (this.stopped) return;
        const error =
            value instanceof Error
                ? value
                : new ConnectError("media", "Browser media failed");
        this.close();
        this.callbacks.error?.(error);
    }
    private videoControlling(route: Route): boolean {
        const state = this.connections.get(route.binding.connectionId);
        return (
            this.stopped ||
            !state ||
            this.routes.get(route.binding.handle) !== route ||
            !!state.controlling ||
            !!state.reconciling ||
            [...this.transitions.values()].some(
                (t) =>
                    t.scope.connid === state.scope.connid &&
                    !(
                        state.nativeReadiness &&
                        t.scope.operation_id ===
                            state.nativeReadiness.root.operationId
                    ),
            )
        );
    }
    private installVideoNotice(route: Route, notice: VideoNotice): void {
        if (!route.video?.observe(notice)) return;
        if (notice.role === "receiver") {
            route.receiveEpoch = notice;
            route.assembler?.clear();
            route.order?.clear();
            route.vp8?.resetReceiveHistory();
            for (const item of route.video.drain(performance.now()))
                this.deliverRtp(route, item.packet, item.arrived);
            this.requestVideoRecovery(route);
        }
    }
    private requestVideoRecovery(route: Route): void {
        if (
            !route.video ||
            !route.vp8?.decodeDiagnostics.waitingForKey ||
            this.videoControlling(route) ||
            this.feedbackWrites.has(route)
        )
            return;
        const write = route.video.request(performance.now());
        if (!write) return;
        this.feedbackWrites.set(route, write);
        this.tasks.track(this.drain().catch((error) => this.die(error)));
    }
    private async writeVideoFeedback(
        route: Route,
        write: VideoFeedbackWrite,
    ): Promise<void> {
        const current = () =>
            !this.videoControlling(route) &&
            route.video?.current(write, performance.now());
        if (!current()) return;
        const remaining = 200 - (performance.now() - write.started);
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
            await Promise.race([
                (async () => {
                    await this.writer.ready;
                    if (!current()) return;
                    // This sequence was consumed on capture. An uncertain write
                    // closes the owner; no requeue, retarget or retry follows.
                    await this.writer.write(write.bytes);
                })(),
                new Promise<never>((_, reject) => {
                    timer = setTimeout(
                        () =>
                            reject(
                                new ConnectError(
                                    "timeout",
                                    "Video feedback writer exceeded original age",
                                ),
                            ),
                        remaining,
                    );
                }),
            ]);
        } finally {
            clearTimeout(timer);
        }
    }
    private applyVideoControl(
        state: ConnectionState,
        type: string,
        payload: JsonObject,
        completed: JsonObject,
    ): void {
        const routes = [...state.routes.values()].filter((r) => r.video);
        if (!routes.length) return;
        if (type === "stream.unsubscribe") {
            if (
                !Array.isArray(payload.strm_ids) ||
                !payload.strm_ids.every(identifier)
            )
                return fail("Invalid original video unsubscribe");
            for (const route of routes)
                if (payload.strm_ids.includes(route.binding.streamId)) {
                    route.video!.unselect();
                    route.assembler?.clear();
                    route.order?.clear();
                    route.vp8?.resetReceiveHistory();
                }
        }
        if (
            type !== "connection.update" ||
            !["hold", "resume", "mute", "unmute"].includes(
                String(payload.action),
            )
        )
            return;
        const result = completed.result;
        if (
            !jsonObject(result) ||
            result.connection_id !== state.scope.connid ||
            result.connection_generation !== state.connectionGeneration ||
            result.action !== payload.action ||
            result.resources_changed !== false ||
            !Array.isArray(result.gate_streams) ||
            !result.gate_streams.length ||
            result.gate_streams.length > 32
        )
            return fail("Invalid current video gate result");
        const gates = result.gate_streams as JsonObject[],
            ids = new Set<string>();
        for (const gate of gates) {
            if (
                !jsonObject(gate) ||
                !jsonObject(gate.binding) ||
                !identifier(gate.binding.stream_id) ||
                ids.has(gate.binding.stream_id)
            )
                return fail("Invalid video gate binding");
            ids.add(gate.binding.stream_id as string);
        }
        const expected = Array.isArray(payload.streams)
            ? payload.streams
            : [...state.routes.keys(), ...state.data.keys()];
        if (
            expected.length !== ids.size ||
            !expected.every((id) => typeof id === "string" && ids.has(id))
        )
            fail("Video gate result changed original targets");
        for (const route of routes) {
            const gate = gates.find(
                (g) =>
                    (g.binding as JsonObject).stream_id ===
                    route.binding.streamId,
            );
            if (!gate) continue;
            const binding = gate.binding as JsonObject,
                b = route.binding;
            if (
                binding.medium !== b.kind ||
                binding.direction !== invert(b.direction) ||
                binding.stream_local_id !== b.handle ||
                binding.stream_generation !== b.generation ||
                binding.connection_generation !== 1 ||
                !jsonObject(binding.codec) ||
                binding.codec.name !== "vp8" ||
                !jsonObject(binding.codec.params) ||
                binding.codec.params.sample_rate !== 90000 ||
                binding.codec.params.channels !== 0 ||
                binding.codec.params.payload_type !== b.payloadType ||
                !["active", "paused"].includes(String(gate.state)) ||
                typeof gate.source_muted !== "boolean" ||
                (gate.held_direction !== null &&
                    !["sendonly", "recvonly", "sendrecv", "inactive"].includes(
                        String(gate.held_direction),
                    ))
            )
                fail("Video gate result changed current allocation");
            const direction =
                gate.held_direction === null
                    ? b.direction
                    : gate.held_direction === "inactive"
                      ? "inactive"
                      : invert(gate.held_direction);
            const send =
                gate.state === "active" &&
                !gate.source_muted &&
                b.direction !== "recvonly" &&
                (direction === "sendonly" || direction === "sendrecv");
            const receive =
                gate.state === "active" &&
                b.direction !== "sendonly" &&
                (direction === "recvonly" || direction === "sendrecv");
            route.video!.gates(send, receive);
            if (!receive) {
                route.assembler?.clear();
                route.order?.clear();
                route.vp8?.resetReceiveHistory();
            }
        }
    }
    private event(frame: Envelope): void {
        if (frame.type === "stream.epoch") {
            try {
                const notice = videoNotice(frame);
                const state = this.connections.get(notice.root.connid);
                if (
                    !state ||
                    state.scope.cid !== notice.root.cid ||
                    state.scope.sid !== notice.root.sid
                )
                    return;
                const route = state.routes.get(notice.stream);
                if (route?.video?.matches(notice))
                    this.installVideoNotice(route, notice);
                else if (state.prepared) {
                    const binding = state.prepared.media.find(
                        (b) => b.handle === notice.handle,
                    );
                    const allocation = state.prepared.feedback.get(
                        notice.handle,
                    );
                    if (
                        binding &&
                        allocation &&
                        new BrowserVideoEpochs(
                            state.scope,
                            binding,
                            allocation,
                        ).matches(notice)
                    ) {
                        const key = notice.stream + ":" + notice.role,
                            old = state.prepared.notices.get(key);
                        if (!old || old.revision < notice.revision) {
                            if (!old && state.prepared.notices.size >= 64)
                                fail("Prepared epoch notice bound exceeded");
                            state.prepared.notices.set(key, notice);
                        }
                    }
                }
            } catch (error) {
                this.die(error);
            }
            return;
        }
        if (
            frame.type === "connection.offer" &&
            typeof frame.operation_id === "string"
        ) {
            const pending = this.transitions.get(frame.operation_id);
            if (pending) {
                try {
                    for (const field of ["cid", "sid", "connid"] as const)
                        if (frame[field] !== pending.scope[field])
                            fail("Media parameter scope mismatch");
                    const setup = frame.payload.substrate_setup;
                    if (
                        frame.payload.substrate !== "webtransport" ||
                        !jsonObject(setup) ||
                        setup.kind !== "native-prepared/1" ||
                        setup.offer_id !== pending.requestId ||
                        setup.revision !== pending.revision + 1
                    )
                        fail(
                            "Media parameters do not bind the current request/revision",
                        );
                    pending.accept(frame);
                } catch (error) {
                    pending.fail(error);
                }
            } else {
                const state =
                    typeof frame.connid === "string"
                        ? this.connections.get(frame.connid)
                        : undefined;
                if (
                    state &&
                    frame.cid === state.scope.cid &&
                    frame.sid === state.scope.sid
                )
                    this.serverOffer(state, frame);
            }
        } else if (frame.type === "message.send") this.dataReceived(frame);
        else if (
            frame.type === "connection.end" ||
            frame.type === "session.ended" ||
            frame.type === "conversation.closed"
        ) {
            for (const [id, c] of this.connections)
                if (
                    frame.cid === c.scope.cid &&
                    (frame.type === "conversation.closed" ||
                        (frame.sid === c.scope.sid &&
                            (frame.type === "session.ended" ||
                                frame.connid === id)))
                )
                    this.retire(id);
        }
    }
    private serverOffer(state: ConnectionState, frame: Envelope): void {
        if (this.stopped || this.connections.get(state.scope.connid) !== state)
            return;
        if (
            state.reconciling ||
            (state.nativeReadiness?.mayHaveCommitted &&
                !state.nativeReplacementCompleted) ||
            [...this.transitions.values()].some(
                (t) => t.scope.connid === state.scope.connid,
            )
        ) {
            if (state.pendingServer)
                return this.die(
                    new ConnectError(
                        "backpressure",
                        "Repeated server media negotiation",
                    ),
                );
            state.pendingServer = frame;
            return;
        }
        const work = this.reconcile(state, frame);
        state.reconciling = work;
        this.tasks.track(
            work
                .catch((error: unknown) => {
                    if (!(
                        error instanceof ConnectError &&
                        error.code === "closed" &&
                        this.connections.get(state.scope.connid) !== state
                    ))
                        this.die(error);
                })
                .finally(() => {
                    state.reconciling = undefined;
                    const next = state.pendingServer;
                    state.pendingServer = undefined;
                    if (
                        next &&
                        !this.stopped &&
                        this.connections.get(state.scope.connid) === state
                    )
                        this.serverOffer(state, next);
                }),
        );
    }
    private async reconcile(
        state: ConnectionState,
        frame: Envelope,
    ): Promise<void> {
        const setup = frame.payload.substrate_setup;
        if (
            !identifier(frame.operation_id) ||
            frame.payload.substrate !== "webtransport" ||
            !jsonObject(setup) ||
            setup.kind !== "native-prepared/1" ||
            !identifier(setup.offer_id) ||
            setup.revision !== state.revision + 1 ||
            !state.selectedKinds.size
        )
            return fail("Unbound server reconciliation parameters");
        const plan = this.plan(frame, state, state.selectedKinds);
        state.prepared = plan;
        await this.answer(state, frame, frame.operation_id, {
            timeoutMs: 10000,
        });
        await this.completed(
            state.scope.cid,
            frame.operation_id,
            performance.now() + 10000,
        );
        this.install(state, plan);
    }
    private async stable(state: ConnectionState): Promise<void> {
        while (state.reconciling) await state.reconciling;
        this.check();
    }
    private answer(
        state: ConnectionState,
        frame: Envelope,
        operationId: string,
        options: RequestOptions,
    ): Promise<string> {
        const setup = frame.payload.substrate_setup as JsonObject;
        return this.client.send(
            "connection.answer",
            {
                by_participant: frame.payload.by_participant,
                substrate: "webtransport",
                capabilities: state.caps,
                streams_answered: (setup.streams as JsonObject[]).map((s) => ({
                    id: s.strm_id,
                    kind: s.kind,
                    direction: invert(s.direction),
                    codec: s.codec,
                })),
                substrate_setup: setup,
            },
            {
                ...state.scope,
                operation_id: operationId,
                in_reply_to: frame.id,
            },
            options,
        );
    }
    private plan(
        frame: Envelope,
        state: ConnectionState,
        selectedKinds: Set<string>,
    ): Plan {
        const setup = frame.payload.substrate_setup;
        if (
            !jsonObject(setup) ||
            !Array.isArray(setup.streams) ||
            !setup.streams.length ||
            setup.streams.length > 32 ||
            !Number.isSafeInteger(setup.revision)
        )
            return fail("Invalid native media setup");
        const remote = jsonObject(frame.payload.capabilities)
            ? frame.payload.capabilities
            : {};
        let connectionGeneration = state.connectionGeneration;
        const feedback: Plan["feedback"] = new Map();
        const nativeEgress: Plan["nativeEgress"] = new Map();
        const media: MediaBinding[] = [],
            data = new Map<string, BrowserDataProtocol>(),
            received = new Set<string>(),
            ids = new Set<string>(),
            handles = new Set<number>();
        for (const value of setup.streams) {
            if (
                !jsonObject(value) ||
                !identifier(value.strm_id) ||
                ids.has(value.strm_id) ||
                !jsonObject(value.codec) ||
                !jsonObject(value.codec.params) ||
                !Number.isInteger(value.stream_local_id) ||
                Number(value.stream_local_id) < 1 ||
                Number(value.stream_local_id) > 65535 ||
                handles.has(Number(value.stream_local_id))
            )
                return fail("Invalid or repeated negotiated stream");
            ids.add(value.strm_id);
            handles.add(Number(value.stream_local_id));
            const codec = value.codec,
                params = codec.params as JsonObject;
            const allowed = state.allowed.get(value.strm_id);
            const direction = invert(value.direction);
            if (allowed) {
                if (
                    allowed.kind !== value.kind ||
                    allowed.codec !== codec.name ||
                    (allowed.direction !== undefined &&
                        allowed.direction !== direction)
                )
                    return fail(
                        "Server changed the requested logical Stream profile",
                    );
            } else {
                // Explicit subscriptions create receiver ports named after source
                // Streams. They never authorize another local publisher.
                if (
                    direction !== "recvonly" ||
                    !(
                        state.received.has(value.strm_id) ||
                        selectedKinds.has(String(value.kind))
                    )
                )
                    return fail("Unrequested publisher or receiver Stream");
                received.add(value.strm_id);
            }
            if (value.kind === "data") {
                if (
                    !["text", "json", "binary"].includes(String(codec.name)) ||
                    params.reliability !== "reliable_ordered" ||
                    !Array.isArray(state.caps.data_protocols) ||
                    !state.caps.data_protocols.includes(codec.name)
                )
                    return fail("Unsupported reliable data format");
                data.set(value.strm_id, codec.name as BrowserDataProtocol);
                continue;
            }
            if (
                !(
                    (value.kind === "audio" &&
                        codec.name === "opus" &&
                        params.sample_rate === 48000 &&
                        params.channels === 1) ||
                    ((value.kind === "video" || value.kind === "screen") &&
                        codec.name === "vp8" &&
                        params.sample_rate === 90000)
                ) ||
                !Number.isInteger(params.payload_type) ||
                Number(params.payload_type) < 0 ||
                Number(params.payload_type) > 127
            )
                return fail("Server selected an unsupported media profile");
            const declared =
                value.kind === "audio"
                    ? state.caps.audio_codecs
                    : state.caps.video_codecs;
            if (
                !Array.isArray(declared) ||
                !declared.some((c) => jsonObject(c) && c.name === codec.name)
            )
                return fail("Receiver codec was never declared");
            const nativeIdentity =
                direction !== "sendonly"
                    ? nativeEgressIdentity(value.native_egress)
                    : undefined;
            if (nativeIdentity) {
                if (
                    connectionGeneration !== 0 &&
                    nativeIdentity.connection_generation !==
                        connectionGeneration
                )
                    fail("Native original generation changed");
                connectionGeneration = nativeIdentity.connection_generation;
                nativeEgress.set(Number(value.stream_local_id), nativeIdentity);
            }
            const prior = state.routes.get(value.strm_id);
            const sameBinding =
                prior &&
                prior.binding.handle === Number(value.stream_local_id) &&
                prior.binding.kind === value.kind &&
                prior.binding.codec === codec.name &&
                prior.binding.payloadType === Number(params.payload_type) &&
                prior.binding.direction === direction;
            const generation =
                nativeIdentity?.stream_generation ??
                (state.caps.video_feedback === VIDEO_FEEDBACK_PROFILE &&
                value.kind === "video"
                    ? sameBinding
                        ? prior.binding.generation
                        : (state.generations.get(value.strm_id) ?? 0) + 1
                    : 1);
            // A replacement starts without an observed physical generation.
            // Bind it once from the signed, original-operation parameter offer,
            // require every Video port to agree, then seal against its receipt.
            // Ordinary Connections still require generation one.
            if (
                connectionGeneration === 0 &&
                value.kind === "video" &&
                state.caps.video_feedback === VIDEO_FEEDBACK_PROFILE
            ) {
                const declared = jsonObject(value.video_feedback)
                    ? value.video_feedback.connection_generation
                    : undefined;
                if (!Number.isSafeInteger(declared) || Number(declared) < 2)
                    return fail("Invalid replacement Video generation");
                connectionGeneration = Number(declared);
            }
            const allocation = videoAllocation(
                value,
                state.caps,
                remote,
                frame.operation_id,
                setup.offer_id,
                generation,
                sameBinding ? prior.video?.allocation : undefined,
                connectionGeneration,
            );
            if (allocation)
                feedback.set(Number(value.stream_local_id), allocation);
            media.push({
                handle: Number(value.stream_local_id),
                streamId: value.strm_id,
                connectionId: state.scope.connid,
                sessionId: state.scope.sid,
                generation,
                kind: value.kind as MediaBinding["kind"],
                codec: codec.name as MediaBinding["codec"],
                payloadType: Number(params.payload_type),
                direction,
            });
        }
        for (const id of state.allowed.keys())
            if (!ids.has(id))
                return fail("Server omitted a requested local Stream");
        const declaredClock = jsonObject(frame.payload.capabilities)
            ? frame.payload.capabilities.source_clock
            : undefined;
        if (declaredClock !== undefined && declaredClock !== "rtcp-sr-sdes/1")
            return fail("Unsupported source clock profile");
        return {
            connectionGeneration,
            feedback,
            notices: new Map(),
            nativeEgress,
            media,
            data,
            revision: Number(setup.revision),
            received,
            clockProfile: declaredClock === "rtcp-sr-sdes/1",
        };
    }
    private requireState(state: ConnectionState): void {
        this.check();
        if (this.connections.get(state.scope.connid) !== state)
            throw new ConnectError(
                "closed",
                "Connection retired before media installation",
            );
    }
    private install(state: ConnectionState, plan: Plan): void {
        this.requireState(state);
        const retained = new Set(plan.media.map((b) => b.handle));
        const current = new Map<string, Route>();
        for (const binding of plan.media) {
            let route = this.routes.get(binding.handle);
            if (route) {
                if (
                    !sameNativeIdentity(
                        route.nativeEgress,
                        plan.nativeEgress.get(binding.handle),
                    )
                )
                    fail("Native original writer incarnation changed");
                if (
                    canonicalJson(
                        (route.video?.allocation as unknown as Json) ?? null,
                    ) !==
                    canonicalJson(
                        (plan.feedback.get(
                            binding.handle,
                        ) as unknown as Json) ?? null,
                    )
                )
                    fail("Current video feedback allocation changed");
                if (route.clockProfile !== plan.clockProfile)
                    fail("Current media clock profile changed");
                if (
                    canonicalJson(route.binding as unknown as JsonObject) !==
                    canonicalJson(binding as unknown as JsonObject)
                )
                    fail("Physical handle changed its binding");
            } else {
                this.registry.register(binding);
                const clock = state.receiveClock;
                clock.register(
                    binding.handle,
                    binding.kind === "audio" ? 48000 : 90000,
                );
                route = {
                    nativeEgress: plan.nativeEgress.get(binding.handle),
                    binding: Object.freeze({ ...binding }),
                    clock,
                    captureClock: state.captureClock,
                    lastReport: -Infinity,
                    clockProfile: plan.clockProfile,
                    observed: {
                        encoded: 0,
                        keys: 0,
                        received: 0,
                        complete: 0,
                        submitted: 0,
                        skipped: 0,
                        decoded: 0,
                        expired: 0,
                        clockReports: 0,
                        synchronized: 0,
                        unsynchronized: 0,
                    },
                };
                const allocation = plan.feedback.get(binding.handle);
                if (allocation)
                    route.video = new BrowserVideoEpochs(
                        state.scope,
                        route.binding,
                        allocation,
                    );
                this.routes.set(binding.handle, route);
                if (binding.direction !== "recvonly")
                    route.sender = new RtpSender(
                        binding,
                        route.video ? this.mtu - 16 : this.mtu,
                    );
                if (
                    route.sender &&
                    binding.kind !== "audio" &&
                    state.nativeReadiness &&
                    !state.nativeReplacementCompleted
                )
                    route.publication = videoPublication();
                const encoded = (frame: EncodedMedia) => {
                    if (
                        this.stopped ||
                        this.routes.get(binding.handle) !== route ||
                        !route!.sender
                    )
                        return;
                    const publication = route!.publication,
                        input = takeEncodedPublication(frame.bytes);
                    if (
                        publication &&
                        !currentPublicationOutput(
                            publication,
                            input,
                            frame.timestampUs,
                            frame.key,
                        )
                    )
                        return;
                    this.registry.require(binding.handle, "send");
                    route!.observed.encoded++;
                    if (frame.key) route!.observed.keys++;
                    const before = this.queue.droppedPackets;
                    const packets = route!.sender.packetize(
                        frame.bytes,
                        rtpTimestampFromMicroseconds(
                            frame.timestampUs,
                            binding.kind === "audio" ? 48000 : 90000,
                        ),
                    );
                    if (packets.length)
                        route!.publisherSsrc = new DataView(
                            packets[0].buffer,
                            packets[0].byteOffset,
                        ).getUint32(16);
                    const queuedAt = performance.now();
                    if (publication && input)
                        publicationPackets(input, frame.key, packets, queuedAt);
                    const accepted = this.queue.push(
                        packets,
                        binding.kind === "audio",
                        queuedAt,
                    );
                    if (!accepted)
                        for (const packet of packets)
                            settlePublicationPacket(packet, false);
                    if (!accepted || before !== this.queue.droppedPackets)
                        for (const r of this.routes.values())
                            r.vp8?.requestKeyFrame();
                    this.tasks.track(
                        this.drain().catch((e: unknown) => this.die(e)),
                    );
                };
                if (binding.kind === "audio")
                    route.opus = new BrowserOpusCodec({
                        encoded,
                        decoded: (frame) => {
                            this.counters.decodedAudio++;
                            route!.observed.decoded++;
                            const observation = route!.decodedObservation;
                            route!.decodedObservation = undefined;
                            this.callbacks.audio?.(
                                route!.binding,
                                frame,
                                observation,
                            );
                        },
                        error: (e) => this.die(e),
                    });
                else {
                    route.assembler = new Vp8Assembler();
                    route.order = new BrowserVideoOrder((frames, expired) => {
                        if (
                            this.stopped ||
                            this.routes.get(binding.handle) !== route
                        )
                            return;
                        try {
                            this.registry.require(
                                binding.handle,
                                "receive",
                                binding.payloadType,
                            );
                            if (expired) route!.vp8?.lost();
                            route!.observed.expired += expired;
                            this.deliverVideoFrames(route!, frames);
                        } catch (error) {
                            this.die(error);
                        }
                    });
                    route.vp8 = new BrowserVp8Codec({
                        encoded,
                        decoded: (frame) => {
                            this.counters.decodedVideo++;
                            route!.observed.decoded++;
                            const observation = route!.decodedObservation;
                            route!.decodedObservation = undefined;
                            this.callbacks.video?.(
                                route!.binding,
                                frame,
                                observation,
                            );
                        },
                        error: (e) => this.die(e),
                    });
                }
            }
            current.set(binding.streamId, route);
            if (route.video) {
                if (
                    !state.generations.has(binding.streamId) &&
                    state.generations.size >= 128
                )
                    fail("Video generation history bound exceeded");
                state.generations.set(binding.streamId, binding.generation);
            }
        }
        for (const old of state.routes.values())
            if (!retained.has(old.binding.handle)) this.remove(old);
        state.routes = current;
        state.prepared = undefined;
        for (const notice of plan.notices.values()) {
            const route = current.get(notice.stream);
            if (route) this.installVideoNotice(route, notice);
        }
        state.data = plan.data;
        state.received = plan.received;
        state.revision = plan.revision;
        state.connectionGeneration = plan.connectionGeneration;
    }
    private async drain(): Promise<void> {
        if (this.sending || this.stopped) return;
        this.sending = true;
        try {
            for (;;) {
                const feedback = this.feedbackWrites.entries().next().value;
                if (feedback) {
                    const [route, write] = feedback;
                    this.feedbackWrites.delete(route);
                    await this.writeVideoFeedback(route, write);
                    continue;
                }
                const bytes = this.queue.pop();
                if (!bytes || this.stopped) {
                    if (bytes) settlePublicationPacket(bytes, false);
                    return;
                }
                const packetHandle = new DataView(
                    bytes.buffer,
                    bytes.byteOffset,
                ).getUint16(2);
                if (
                    !currentPublicationPacket(
                        bytes,
                        this.routes.get(packetHandle)?.publication,
                    )
                ) {
                    settlePublicationPacket(bytes, false);
                    continue;
                }
                try {
                    this.registry.require(
                        new DataView(bytes.buffer, bytes.byteOffset).getUint16(
                            2,
                        ),
                        "send",
                    );
                } catch {
                    settlePublicationPacket(bytes, false);
                    continue;
                }
                const started = performance.now();
                let timer: ReturnType<typeof setTimeout> | undefined;
                let publicationSent = false;
                try {
                    await Promise.race([
                        this.writer.write(bytes),
                        new Promise<never>((_, reject) => {
                            timer = setTimeout(
                                () =>
                                    reject(
                                        new ConnectError(
                                            "timeout",
                                            "Datagram writer stalled beyond media age",
                                        ),
                                    ),
                                200,
                            );
                        }),
                    ]);
                    settlePublicationPacket(bytes, true);
                    publicationSent = true;
                    this.counters.sent++;
                    const handle = new DataView(
                        bytes.buffer,
                        bytes.byteOffset,
                    ).getUint16(2);
                    const route = this.routes.get(handle);
                    if (route?.sender && route.clockProfile && !this.stopped) {
                        const stats = route.sender.submitted(bytes);
                        const now = performance.now();
                        const sample = route.captureClock.sample(
                            route.binding.kind === "audio" ? 48000 : 90000,
                        );
                        if (sample && now - route.lastReport >= 1000) {
                            this.registry.require(handle, "send");
                            const report = wrapRtcpSenderReport(
                                handle,
                                route.sender.nextDatagramSequence(),
                                {
                                    ...stats,
                                    ...sample,
                                    cname: route.captureClock.cname,
                                },
                            );
                            if (report.length <= this.mtu) {
                                clearTimeout(timer);
                                const remaining =
                                    200 - (performance.now() - started);
                                if (remaining <= 0)
                                    throw new ConnectError(
                                        "timeout",
                                        "Media submission age expired",
                                    );
                                await Promise.race([
                                    this.writer.write(report),
                                    new Promise<never>((_, reject) => {
                                        timer = setTimeout(
                                            () =>
                                                reject(
                                                    new ConnectError(
                                                        "timeout",
                                                        "Clock datagram writer stalled",
                                                    ),
                                                ),
                                            remaining,
                                        );
                                    }),
                                ]);
                                route.lastReport = now;
                            }
                        }
                    }
                } finally {
                    if (!publicationSent) settlePublicationPacket(bytes, false);
                    clearTimeout(timer);
                }
            }
        } finally {
            this.sending = false;
        }
    }
    private async receive(): Promise<void> {
        let burst = 0;
        while (!this.stopped) {
            const { value, done } = await this.reader.read();
            if (done) {
                if (!this.stopped)
                    throw new ConnectError("transport", "Media datagram EOF");
                return;
            }
            const arrived = performance.now();
            // A stream with queued chunks can resolve every read as a microtask.
            // Give WebCodecs output/dequeue tasks a turn within its eight-frame
            // decode bound, rather than treating task starvation as media loss.
            if (++burst === 8) {
                burst = 0;
                await new Promise<void>((resolve) => setTimeout(resolve, 0));
                if (this.stopped) return;
            }
            if (value.length === 48 && value[1] === 0 && value[9] === 206) {
                try {
                    const feedback = decodeNativeVideoFeedback(value),
                        route = this.routes.get(feedback.handle);
                    if (!route?.video || this.videoControlling(route))
                        throw new Error();
                    route.video.feedback(feedback, arrived);
                } catch {
                    this.counters.rejected++;
                }
                continue;
            }
            if (value[1] === 1) {
                try {
                    const packet = decodeNativeVideoEpoch(value),
                        route = this.routes.get(packet.packet.handle);
                    if (!route?.video) throw new Error();
                    this.registry.require(
                        packet.packet.handle,
                        "receive",
                        packet.packet.payloadType,
                    );
                    const count = [...this.routes.values()].reduce(
                        (n, r) => n + (r.video?.pendingPackets ?? 0),
                        0,
                    );
                    const admitted = route.video.packet(
                        packet,
                        arrived,
                        performance.now(),
                        count < 128,
                    );
                    if (admitted) this.deliverRtp(route, admitted, arrived);
                } catch {
                    this.counters.rejected++;
                }
                continue;
            }
            if (isRtcpDatagram(value)) {
                try {
                    const handle = new DataView(
                        value.buffer,
                        value.byteOffset,
                    ).getUint16(2);
                    this.registry.require(handle, "receive");
                    const route = this.routes.get(handle)!;
                    if (!route.clockProfile)
                        throw new ConnectError(
                            "unsupported-media-profile",
                            "Sender clock profile was not negotiated",
                        );
                    if (
                        !route.clock.report(
                            handle,
                            decodeRtcpSenderReport(value.subarray(8)),
                        )
                    )
                        throw new ConnectError(
                            "protocol",
                            "Unusable sender clock",
                        );
                    route.observed.clockReports++;
                } catch {
                    this.counters.rejected++;
                }
                continue;
            }
            let packet, route;
            try {
                packet = decodeNativeRtpDatagram(value);
                this.registry.require(
                    packet.handle,
                    "receive",
                    packet.payloadType,
                );
                route = this.routes.get(packet.handle)!;
                if (route.video) throw new Error();
            } catch {
                this.counters.rejected++;
                continue;
            }
            this.deliverRtp(route, packet, arrived);
        }
    }
    private deliverRtp(
        route: Route,
        packet: RtpDatagram,
        arrived: number,
    ): void {
        this.counters.received++;
        route.observed.received++;
        const timing = route.clock.timestamp(
            packet.handle,
            packet.ssrc,
            packet.timestamp,
        );
        if (timing.synchronized) route.observed.synchronized++;
        else route.observed.unsynchronized++;
        if (route.opus) {
            const state = this.connections.get(route.binding.connectionId),
                readiness = state?.nativeReadiness;
            const identity = {
                connectionId: route.binding.connectionId,
                streamId: route.binding.streamId,
                generation: route.binding.generation,
                ssrc: packet.ssrc,
            };
            route.audioSource = identity;
            route.opus.decode(packet.payload, timing.timestampUs, (frame) => {
                route.decodedObservation = route.nativeEgress
                    ? Object.freeze({
                          profile: "connect/native-decoded-sequence/1",
                          identity: route.nativeEgress,
                          datagramSequences: Object.freeze([
                              packet.datagramSequence,
                          ]),
                          ssrc: packet.ssrc,
                          rtpTimestamp: packet.timestamp,
                          rtpSequence: packet.sequence,
                          ...(timing.sourceNtp
                              ? {
                                    sourceNtp: timing.sourceNtp,
                                    sourceClock: timing.sourceClock,
                                }
                              : {}),
                      })
                    : undefined;
                if (
                    readiness?.observing &&
                    !this.stopped &&
                    this.routes.get(route.binding.handle) === route &&
                    state?.routes.get(route.binding.streamId) === route &&
                    route.audioSource?.ssrc === packet.ssrc
                )
                    readiness.audioDecoded({
                        identity,
                        sequence: packet.sequence,
                        timestamp: packet.timestamp,
                        arrived,
                        samples: frame.numberOfFrames,
                    });
            });
        } else {
            let complete;
            try {
                // Complete and incomplete frames share the original eight-frame /
                // 64-fragment budget; no additional queue capacity is introduced.
                if (
                    route.order!.queuedFrames >= MEDIA_LIMITS.frames ||
                    route.order!.queuedFragments >= MEDIA_LIMITS.fragments
                )
                    route.order!.flush(performance.now(), true);
                complete = route.assembler!.push(packet, arrived, {
                    frames: MEDIA_LIMITS.frames - route.order!.queuedFrames,
                    fragments:
                        MEDIA_LIMITS.fragments - route.order!.queuedFragments,
                });
            } catch {
                this.counters.rejected++;
                route.vp8!.lost();
                return;
            }
            if (complete) {
                complete.sourceNtp = timing.sourceNtp;
                complete.sourceClock = timing.sourceClock;
                route.observed.complete++;
                route.order!.push(complete, timing.timestampUs);
            }
        }
        this.requestVideoRecovery(route);
    }

    private deliverVideoFrames(
        route: Route,
        frames: OrderedVideoFrame[],
    ): void {
        for (const { frame, timestampUs } of frames) {
            if (this.stopped || this.routes.get(route.binding.handle) !== route)
                return;
            const state = this.connections.get(route.binding.connectionId),
                readiness = state?.nativeReadiness,
                notice = route.receiveEpoch;
            const identity = {
                connectionId: route.binding.connectionId,
                streamId: route.binding.streamId,
                generation: route.binding.generation,
                ssrc: frame.ssrc,
            };
            if (
                route.vp8!.decode(frame, timestampUs, (decoded) => {
                    route.decodedObservation = route.nativeEgress
                        ? Object.freeze({
                              profile: "connect/native-decoded-sequence/1",
                              identity: route.nativeEgress,
                              datagramSequences: frame.datagramSequences,
                              ssrc: frame.ssrc,
                              rtpTimestamp: frame.timestamp,
                              ...(frame.sourceNtp
                                  ? {
                                        sourceNtp: frame.sourceNtp,
                                        sourceClock: frame.sourceClock,
                                    }
                                  : {}),
                              ...(notice
                                  ? {
                                        videoEpoch: canonicalJson(
                                            notice as unknown as JsonObject,
                                        ),
                                    }
                                  : {}),
                          })
                        : undefined;
                    if (
                        readiness?.observing &&
                        notice &&
                        !this.stopped &&
                        this.routes.get(route.binding.handle) === route &&
                        state?.routes.get(route.binding.streamId) === route &&
                        route.video?.currentReceiver(notice)
                    )
                        readiness.videoDecoded({
                            identity,
                            epoch: canonicalJson(
                                notice as unknown as JsonObject,
                            ),
                            arrived: frame.arrived,
                            key: frame.key,
                            width: decoded.displayWidth,
                            height: decoded.displayHeight,
                        });
                })
            ) {
                if (frame.key)
                    route.assembler!.retireOlderPartialsAfterKeyframe(frame);
                route.observed.submitted++;
            } else route.observed.skipped++;
        }
        this.requestVideoRecovery(route);
    }

    /** Start one original offer. `accepted` means validated current physical
     * parameters arrived; `ready` separately observes installation. Neither
     * promise can retry the mutation or lend ownership after peer retirement. */
    beginOfferConnection(
        input: BrowserConnectionOptions,
        options: RequestOptions = {},
    ): {
        accepted: Promise<void>;
        ready: Promise<BrowserMediaConnection>;
        readonly submissionAttempted: boolean;
    } {
        let accept!: () => void, reject!: (error: unknown) => void;
        const accepted = new Promise<void>((yes, no) => {
            accept = yes;
            reject = no;
        });
        let attempted = false;
        const ready = this.offerConnection(input, options, accept, () => {
            attempted = true;
        });
        void accepted.catch(() => {});
        void ready.catch(reject);
        return {
            accepted,
            ready,
            get submissionAttempted() {
                return attempted;
            },
        };
    }
    /** Prepare a signed target against one existing replacement root. `prepared`
     * permits explicit capture for private readiness; only `completed` observes
     * the original committed move. No phase is retried or silently reconnected. */
    beginReplacementConnection(
        input: BrowserConnectionOptions,
        original: BrowserReplacementRoot,
        options: RequestOptions = {},
    ): {
        prepared: Promise<BrowserMediaConnection>;
        completed: Promise<BrowserReplacementReceipt>;
        readonly submissionAttempted: boolean;
    } {
        const root = replacementRoot(original);
        if (
            !input ||
            input.purpose !== undefined ||
            !input.audio ||
            input.screen ||
            input.operationId !== root.operationId ||
            input.conversationId !== root.conversationId ||
            input.sessionId !== root.sessionId ||
            input.connectionId !== root.targetConnectionId ||
            (input.video !== undefined) !==
                (input.receiveVideo !== undefined) ||
            (input.receiveVideo !== undefined &&
                input.nativeVideoFeedback !== true)
        )
            throw new ConnectError(
                "invalid-request",
                "Use the original target and explicit directional Video ports",
            );
        let prepare!: (connection: BrowserMediaConnection) => void,
            reject!: (error: unknown) => void;
        const prepared = new Promise<BrowserMediaConnection>((yes, no) => {
            prepare = yes;
            reject = no;
        });
        let attempted = false,
            receipt: BrowserReplacementReceipt | undefined;
        const completed = this.tasks.track(
            this.openConnection(
                input,
                options,
                undefined,
                () => {
                    attempted = true;
                },
                {
                    root,
                    audio: input.audio,
                    video: input.receiveVideo,
                    prepared: prepare,
                    committed: (value) => {
                        receipt = value;
                    },
                },
            )
                .then(() => {
                    if (!receipt)
                        return fail(
                            "Replacement completed without its original receipt",
                        );
                    return receipt;
                })
                .catch((error) => {
                    reject(error);
                    throw error;
                }),
        );
        void prepared.catch(() => {});
        void completed.catch(() => {});
        return {
            prepared,
            completed,
            get submissionAttempted() {
                return attempted;
            },
        };
    }
    async offerConnection(
        input: BrowserConnectionOptions,
        options: RequestOptions = {},
        accepted?: () => void,
        attempted?: () => void,
    ): Promise<BrowserMediaConnection> {
        return this.openConnection(input, options, accepted, attempted);
    }
    private async openConnection(
        input: BrowserConnectionOptions,
        options: RequestOptions,
        accepted?: () => void,
        attempted?: () => void,
        replacement?: ReplacementPreparation,
    ): Promise<BrowserMediaConnection> {
        this.check();
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 60000)
            throw new ConnectError("invalid-request", "Invalid media deadline");
        const budget = new BrowserDeadline(timeout);
        if (
            !input ||
            (input.nativeVideoFeedback !== undefined &&
                typeof input.nativeVideoFeedback !== "boolean") ||
            (input.purpose !== undefined &&
                (input.purpose !== "handoff-preparation" ||
                    !input.audio ||
                    input.video ||
                    input.receiveVideo ||
                    input.screen ||
                    input.data?.length)) ||
            (input.data !== undefined &&
                (!Array.isArray(input.data) ||
                    input.data.length > 8 ||
                    input.data.some(
                        (v) =>
                            !v ||
                            !["text", "json", "binary"].includes(v.protocol),
                    )))
        )
            throw new ConnectError(
                "invalid-request",
                "Invalid original native stream intent",
            );
        input = {
            conversationId: input.conversationId,
            sessionId: input.sessionId,
            connectionId: input.connectionId,
            operationId: input.operationId,
            purpose: input.purpose,
            nativeVideoFeedback: input.nativeVideoFeedback,
            audio: input.audio,
            video: input.video,
            receiveVideo: input.receiveVideo,
            screen: input.screen,
            data: input.data?.map((v) => ({ id: v.id, protocol: v.protocol })),
        };
        const streams: JsonObject[] = [];
        for (const kind of ["audio", "video", "screen"] as const)
            if (input[kind])
                streams.push({
                    id: input[kind]!,
                    kind,
                    direction:
                        kind === "video" && input.receiveVideo
                            ? "sendonly"
                            : "sendrecv",
                    codec_preferences: [kind === "audio" ? "opus" : "vp8"],
                });
        if (input.receiveVideo)
            streams.push({
                id: input.receiveVideo,
                kind: "video",
                direction: "recvonly",
                codec_preferences: ["vp8"],
            });
        for (const data of input.data ?? [])
            streams.push({
                id: data.id,
                kind: "data",
                direction: "sendrecv",
                codec_preferences: [data.protocol],
            });
        if (
            ![
                input.conversationId,
                input.sessionId,
                input.connectionId,
                input.operationId,
            ].every(identifier) ||
            !streams.length ||
            streams.length > 8 ||
            streams.some((s) => !identifier(s.id)) ||
            new Set(streams.map((s) => s.id)).size !== streams.length ||
            this.connections.has(input.connectionId) ||
            this.connections.size >= 8
        )
            throw new ConnectError(
                "invalid-request",
                "Use new scoped connection/operation IDs and 1–8 distinct streams",
            );
        const support = await budget.run(browserCodecSupport, options.signal);
        this.check();
        if (
            (input.audio && !support.audio) ||
            ((input.video || input.receiveVideo || input.screen) &&
                !support.video)
        )
            throw new ConnectError(
                "unsupported-media-profile",
                support.missing.join(", "),
            );
        const caps: JsonObject = {
            audio_codecs: input.audio
                ? [
                      {
                          name: "opus",
                          params: { sample_rate: 48000, channels: 1 },
                      },
                  ]
                : [],
            video_codecs:
                input.video || input.receiveVideo || input.screen
                    ? [
                          {
                              name: "vp8",
                              params: { sample_rate: 90000, channels: 0 },
                          },
                      ]
                    : [],
            data_protocols: [
                ...new Set((input.data ?? []).map((s) => s.protocol)),
            ],
            // Retain local publishers plus independently named subscriber ports.
            max_streams_per_connection: 32,
            identity_assurance_offered: "identified",
            source_clock: "rtcp-sr-sdes/1",
            ...(input.nativeVideoFeedback === true
                ? { video_feedback: VIDEO_FEEDBACK_PROFILE }
                : {}),
        };
        const state: ConnectionState = {
            scope: {
                cid: input.conversationId,
                sid: input.sessionId,
                connid: input.connectionId,
            },
            revision: 0,
            connectionGeneration: replacement ? 0 : 1,
            caps,
            captureClock: new RtcpCaptureClock(
                "connect-" + crypto.randomUUID(),
            ),
            receiveClock: new RtcpReceiveClock(),
            routes: new Map(),
            data: new Map(),
            allowed: new Map(
                streams.map((s) => [
                    String(s.id),
                    {
                        kind: s.kind,
                        codec: (s.codec_preferences as Json[])[0],
                        ...(replacement ? { direction: s.direction } : {}),
                    },
                ]),
            ),
            received: new Set(),
            generations: new Map(),
            selectedKinds: new Set(),
            closed: new Set(),
        };
        this.connections.set(input.connectionId, state);
        try {
            if (replacement) {
                const root = replacement.root;
                const observed = await this.replacementStatus(root, {
                    ...options,
                    timeoutMs: budget.remaining(),
                });
                if (
                    observed.state !== "preparing" ||
                    observed.native_replacement_prepared !== true
                )
                    throw new ConnectError(
                        "conflict",
                        "Original replacement preparation is not currently recorded",
                    );
                attempted?.();
                const frames = await this.client.request(
                    "connection.offer",
                    {
                        substrate: "webtransport",
                        capabilities: caps,
                        streams_offered: streams,
                        replacement_preparation: {
                            profile: "connect-replacement-preparation-v1",
                            operation_record_id: root.recordId,
                        },
                    },
                    {
                        ...state.scope,
                        operation_id: root.operationId,
                        expected_revision: 0,
                    },
                    { ...options, timeoutMs: budget.remaining() },
                );
                const allocated = replacementFrame(
                    root,
                    frames,
                    root.targetConnectionId,
                );
                if (
                    allocated.state !== "preparing" ||
                    Number(allocated.revision) < Number(observed.revision)
                )
                    return fail(
                        "Original replacement allocation did not remain preparing",
                    );
            }
            await this.transition(
                state,
                input.operationId,
                "connection.offer",
                {
                    substrate: "webtransport",
                    ...(replacement
                        ? {
                              replacement_preparation: {
                                  profile: "connect-replacement-media-v1",
                                  operation_record_id:
                                      replacement.root.recordId,
                              },
                          }
                        : {}),
                    ...(input.purpose === "handoff-preparation"
                        ? {
                              handoff_preparation: {
                                  profile: "connect-handoff-preparation-v1",
                              },
                          }
                        : {}),
                    capabilities: caps,
                    streams_offered: streams,
                },
                { ...options, timeoutMs: budget.remaining() },
                accepted,
                attempted,
                undefined,
                undefined,
                replacement,
            );
        } catch (error) {
            if (!state.nativeReadiness?.mayHaveCommitted)
                this.retire(input.connectionId);
            if (error instanceof ConnectError && error.code === "protocol")
                this.client.close(error);
            throw error;
        }
        return new BrowserMediaConnection(this, input.connectionId);
    }
    /** Start one original native control while retaining its counteroffer owner. */
    beginControl(
        id: string,
        operationId: string,
        type: "stream.subscribe" | "stream.unsubscribe" | "connection.update",
        payload: JsonObject,
        expectedRevision: number,
        options: RequestOptions,
    ) {
        const timeout = options.timeoutMs ?? 10000;
        if (
            !Number.isFinite(timeout) ||
            timeout < 1 ||
            timeout > 60000 ||
            !Number.isSafeInteger(expectedRevision) ||
            expectedRevision < 0
        )
            throw new ConnectError(
                "invalid-request",
                "Use an original bounded native control",
            );
        const state = this.state(id),
            budget = new BrowserDeadline(timeout);
        const body = parseCanonicalJson(
            new TextEncoder().encode(canonicalJson(payload, 32768)),
            32768,
        ) as JsonObject;
        let resolve!: (frames: Envelope[]) => void,
            reject!: (error: unknown) => void,
            attempted = false;
        const response = new Promise<Envelope[]>((yes, no) => {
            resolve = yes;
            reject = no;
        });
        const previous = state.controlling;
        const ready = (async () => {
            if (previous)
                await budget.run(
                    () => previous.catch(() => {}),
                    options.signal,
                );
            this.requireState(state);
            await this.transition(
                state,
                operationId,
                type,
                body,
                { ...options, timeoutMs: budget.remaining() },
                undefined,
                () => {
                    attempted = true;
                },
                expectedRevision,
                (work) => {
                    void work.then(resolve, reject);
                },
            );
        })().catch((error) => {
            reject(error);
            throw error;
        });
        const settled = Promise.allSettled([
            previous ?? Promise.resolve(),
            ready,
        ]).then(() => {});
        state.controlling = settled;
        void settled.then(() => {
            if (state.controlling === settled) state.controlling = undefined;
        });
        void response.catch(() => {});
        void ready.catch(() => {});
        return {
            response,
            ready,
            get submissionAttempted() {
                return attempted;
            },
        };
    }
    private async transition(
        state: ConnectionState,
        operationId: string,
        type:
            | "connection.offer"
            | "stream.subscribe"
            | "stream.unsubscribe"
            | "connection.update",
        payload: JsonObject,
        options: RequestOptions,
        accepted?: () => void,
        attempted?: () => void,
        requestedRevision?: number,
        startedResponse?: (response: Promise<Envelope[]>) => void,
        replacement?: ReplacementPreparation,
    ): Promise<void> {
        this.check();
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 60000)
            throw new ConnectError("invalid-request", "Invalid media deadline");
        const budget = new BrowserDeadline(timeout);
        await budget.run(() => this.stable(state), options.signal);
        const expected = requestedRevision ?? state.revision;
        // Audience-only commits can advance the server revision without new
        // local parameters. A caller may assert a later observed revision;
        // only the original signed result can advance this physical owner.
        if (expected < state.revision)
            throw new ConnectError(
                "conflict",
                "Native control revision conflicts",
            );
        if (
            !identifier(operationId) ||
            this.transitions.has(operationId) ||
            [...this.transitions.values()].some(
                (t) => t.scope.connid === state.scope.connid,
            )
        )
            throw new ConnectError(
                "invalid-request",
                "Use a distinct stable logical operation ID",
            );
        const until = performance.now() + budget.remaining();
        const remaining = (): RequestOptions => ({
            timeoutMs: Math.max(1, until - performance.now()),
            signal: options.signal,
        });
        let accept!: (frame: Envelope) => void,
            reject!: (error: unknown) => void;
        const parameters = new Promise<Envelope>((yes, no) => {
            accept = yes;
            reject = no;
        });
        void parameters.catch(() => {});
        const scope = {
            ...state.scope,
            operation_id: operationId,
            expected_revision: expected,
        };
        attempted?.();
        for (const route of state.routes.values())
            route.video?.invalidateRequests();
        const started = this.client.begin(type, payload, scope, {
            ...remaining(),
            terminalType:
                type === "connection.offer" ? "connection.answer" : undefined,
        });
        startedResponse?.(started.response);
        this.transitions.set(operationId, {
            scope,
            requestId: started.id,
            revision: expected,
            accept,
            fail: reject,
        });
        void started.response.catch(reject);
        const abort = () => {
            const error = new ConnectError(
                "cancelled",
                "Media negotiation cancelled; retain logical operation ID",
            );
            reject(error);
            this.client.close(error);
        };
        const timer = setTimeout(() => {
            const error = new ConnectError(
                "timeout",
                "Media negotiation deadline; retain logical operation ID",
            );
            reject(error);
            this.client.close(error);
        }, budget.remaining());
        options.signal?.addEventListener("abort", abort, { once: true });
        if (options.signal?.aborted) abort();
        let awaitingNativeReadiness = false;
        try {
            const first = await Promise.race([
                parameters.then((frame) => ({ frame })),
                started.response.then((frames) => ({ frames })),
            ]);
            let frame: Envelope | undefined;
            if ("frame" in first) frame = first.frame;
            else if (first.frames.some((f) => f.type === "connection.answer"))
                frame = await parameters;
            else if (type === "connection.offer") {
                const status = await this.client.operationStatus(
                    state.scope.cid,
                    operationId,
                    remaining(),
                );
                if (status.state === "completed")
                    throw new ConnectError(
                        "historical-operation",
                        "Completed offer has no new physical media parameters; inspect its original operation status",
                    );
                const problem = jsonObject(status.problem)
                    ? status.problem
                    : {};
                throw new ConnectError(
                    "refused",
                    "Media offer did not negotiate: " +
                        String(problem.code ?? status.state) +
                        (typeof problem.message === "string"
                            ? ": " + problem.message
                            : ""),
                );
            }
            let completed: JsonObject;
            const selectedKinds = new Set<string>();
            if (
                type === "stream.subscribe" &&
                Array.isArray(payload.subscriptions)
            )
                for (const sub of payload.subscriptions)
                    if (jsonObject(sub)) {
                        const kinds = Array.isArray(sub.kinds)
                            ? sub.kinds
                            : ["audio", "video", "screen", "data"];
                        for (const kind of kinds)
                            if (typeof kind === "string")
                                selectedKinds.add(kind);
                    }
            if (frame) {
                this.requireState(state);
                const plan = this.plan(frame, state, selectedKinds);
                state.prepared = plan;
                accepted?.();
                await this.answer(state, frame, operationId, remaining());
                const response = await started.response;
                if (replacement) {
                    const answers = response.filter(
                        (value) => value.type === "connection.answer",
                    );
                    if (answers.length !== 1)
                        fail(
                            "Native replacement requires one original signed answer",
                        );
                    state.nativeReadiness = new BrowserNativeReadiness(
                        replacement.root,
                        payload,
                        scope,
                        answers[0],
                        frame,
                        started.id,
                        replacement.audio,
                        replacement.video,
                        plan.connectionGeneration,
                    );
                    plan.connectionGeneration =
                        state.nativeReadiness.targetGeneration;
                    // Parameter negotiation has joined. The original absolute
                    // deadline continues in the proof wait; cancellation there
                    // cannot destroy a target that may already have committed.
                    clearTimeout(timer);
                    options.signal?.removeEventListener("abort", abort);
                    // These are the exact signed target parameters, not a
                    // completed movement. Install before waiting so real target
                    // IO can satisfy the server's private readiness phase.
                    this.install(state, plan);
                    replacement.prepared(
                        new BrowserMediaConnection(this, state.scope.connid),
                    );
                    awaitingNativeReadiness = true;
                    completed = await state.nativeReadiness.finish(
                        this.client,
                        until,
                        () => {
                            this.requireState(state);
                            const audio = state.routes.get(replacement.audio),
                                video = replacement.video
                                    ? state.routes.get(replacement.video)
                                    : undefined;
                            const notice = video?.receiveEpoch;
                            const current: CurrentNativePreparation = {
                                audio: audio?.audioSource,
                                video:
                                    video &&
                                    notice &&
                                    video.video?.currentReceiver(notice)
                                        ? {
                                              identity: {
                                                  connectionId:
                                                      video.binding
                                                          .connectionId,
                                                  streamId:
                                                      video.binding.streamId,
                                                  generation:
                                                      video.binding.generation,
                                                  ssrc: notice.ssrc,
                                              },
                                              epoch: canonicalJson(
                                                  notice as unknown as JsonObject,
                                              ),
                                          }
                                        : undefined,
                            };
                            return current;
                        },
                        options.signal,
                    );
                    awaitingNativeReadiness = false;
                    const receipt = replacementReceipt(
                        replacement.root,
                        completed,
                    );
                    if (
                        state.connectionGeneration !== 0 &&
                        state.connectionGeneration !== receipt.generation
                    )
                        return fail(
                            "Replacement receipt changed the negotiated generation",
                        );
                    const readiness = (completed.result as JsonObject)
                        .readiness as JsonObject;
                    const actualStreams = readiness.streams as string[];
                    if (
                        actualStreams.length !== state.allowed.size ||
                        new Set(actualStreams).size !== actualStreams.length ||
                        actualStreams.some((id) => !state.allowed.has(id))
                    )
                        return fail(
                            "Replacement receipt changed the negotiated target streams",
                        );
                    this.requireState(state);
                    state.revision = receipt.streamRevision;
                    state.connectionGeneration = receipt.generation;
                    // The original completed move transfers these receiver
                    // ports. Later signed reconciliation may update their
                    // source while retaining the negotiated local publishers.
                    for (const stream of state.allowed.values())
                        if (stream.direction !== "sendonly")
                            state.selectedKinds.add(String(stream.kind));
                    const publicationBoundary = performance.now();
                    for (const route of state.routes.values())
                        if (route.publication)
                            completeVideoPublication(
                                route.publication,
                                publicationBoundary,
                            );
                    state.nativeReplacementCompleted = true;
                    replacement.committed(receipt);
                } else {
                    completed = await this.completed(
                        state.scope.cid,
                        operationId,
                        until,
                        options.signal,
                    );
                    try {
                        this.install(state, plan);
                    } catch (error) {
                        if (!(
                            error instanceof ConnectError &&
                            error.code === "closed" &&
                            this.connections.get(state.scope.connid) !== state
                        ))
                            this.die(error);
                        throw error;
                    }
                }
            } else
                completed = await this.completed(
                    state.scope.cid,
                    operationId,
                    until,
                    options.signal,
                );
            if (
                type === "connection.offer" &&
                jsonObject(payload.handoff_preparation)
            ) {
                const result = completed.result;
                if (
                    !jsonObject(result) ||
                    state.revision !== 1 ||
                    result.stream_revision !== 2 ||
                    !jsonObject(result.handoff_preparation) ||
                    result.handoff_preparation.profile !==
                        "connect-handoff-preparation-v1" ||
                    result.handoff_preparation.prepared_muted !== true
                )
                    return fail(
                        "Original handoff preparation did not confirm its muted revision",
                    );
                this.requireState(state);
                state.revision = 2;
            }
            if (
                type !== "connection.offer" &&
                requestedRevision !== undefined
            ) {
                const result = completed.result;
                if (!jsonObject(result))
                    return fail("Missing original native control result");
                if (
                    result.connection_id !== state.scope.connid ||
                    !Number.isSafeInteger(result.stream_revision) ||
                    Number(result.stream_revision) < expected ||
                    Number(result.stream_revision) > expected + 1 ||
                    (frame
                        ? result.stream_revision !== state.revision
                        : result.resources_changed !== false) ||
                    (type === "connection.update" &&
                        payload.action !== "codec-renegotiate" &&
                        (payload.action === "quality"
                            ? result.quality_interval_ms !==
                              payload.quality_interval_ms
                            : result.action !== payload.action))
                )
                    fail(
                        "Native control result does not bind the original revision/profile",
                    );
                this.requireState(state);
                state.revision = Number(result.stream_revision);
            }
            this.applyVideoControl(state, type, payload, completed);
            for (const kind of selectedKinds) state.selectedKinds.add(kind);
        } catch (error) {
            // Signed parameter negotiation already joined. Before any ready
            // continuation attempt, a local decode wait grants no activation.
            // The outer owner retires this exact target without destroying the
            // shared control peer or its unrelated live Connections.
            const localUnsubmittedReadiness =
                replacement !== undefined &&
                awaitingNativeReadiness &&
                state.nativeReadiness !== undefined &&
                !state.nativeReadiness.readinessSubmissionAttempted &&
                error instanceof ConnectError &&
                (error.code === "timeout" || error.code === "cancelled");
            if (
                !localUnsubmittedReadiness &&
                !state.nativeReadiness?.mayHaveCommitted &&
                (!(error instanceof ConnectError) ||
                    (!["refused", "historical-operation"].includes(
                        error.code,
                    ) &&
                        !(
                            error.code === "closed" &&
                            this.connections.get(state.scope.connid) !== state
                        )))
            )
                this.client.close(error);
            throw error;
        } finally {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
            this.transitions.delete(operationId);
            state.prepared = undefined;
            // Keep the one already-owned callback while the original target's
            // public ownership is unresolved. It cannot authorize reconciliation
            // and must not retire potentially committed media as a side effect.
            const unresolved =
                state.nativeReadiness?.mayHaveCommitted &&
                !state.nativeReplacementCompleted;
            const next = unresolved ? undefined : state.pendingServer;
            if (!unresolved) state.pendingServer = undefined;
            if (
                next &&
                !this.stopped &&
                this.connections.get(state.scope.connid) === state
            )
                this.serverOffer(state, next);
        }
    }
    private async completed(
        cid: string,
        operation: string,
        until: number,
        signal?: AbortSignal,
    ): Promise<JsonObject> {
        for (;;) {
            const state = await this.client.operationStatus(cid, operation, {
                timeoutMs: Math.max(1, until - performance.now()),
                signal,
            });
            if (state.state === "completed") return state;
            if (["failed", "cancelled"].includes(String(state.state))) {
                const problem = jsonObject(state.problem) ? state.problem : {};
                throw new ConnectError(
                    "refused",
                    "Media operation " +
                        operation +
                        " did not complete: " +
                        String(problem.code ?? state.state) +
                        (typeof problem.message === "string"
                            ? ": " + problem.message
                            : ""),
                );
            }
            if (performance.now() >= until)
                throw new ConnectError(
                    "timeout",
                    "Media operation remains uncertain",
                );
            await new Promise((r) => setTimeout(r, 20));
        }
    }
    private async replacementStatus(
        root: BrowserReplacementRoot,
        options: RequestOptions,
    ): Promise<JsonObject> {
        const frames = await this.client.request(
            "operation.get",
            { operation_id: root.operationId },
            {
                cid: root.conversationId,
                sid: root.sessionId,
                connid: root.sourceConnectionId,
            },
            options,
        );
        return replacementFrame(root, frames, root.sourceConnectionId);
    }
    private async completedReplacement(
        root: BrowserReplacementRoot,
        until: number,
        signal?: AbortSignal,
    ): Promise<JsonObject> {
        let revision = -1;
        for (;;) {
            const left = until - performance.now();
            if (left < 1)
                throw new ConnectError(
                    "timeout",
                    "Original replacement completion remains uncertain",
                );
            const value = await this.replacementStatus(root, {
                timeoutMs: left,
                signal,
            });
            if (Number(value.revision) < revision)
                return fail("Original replacement revision regressed");
            revision = Number(value.revision);
            if (value.state === "completed") return value;
            if (["failed", "cancelled"].includes(String(value.state)))
                throw new ConnectError(
                    "refused",
                    "Original replacement did not complete",
                );
            if (
                ![
                    "accepted",
                    "preparing",
                    "ready",
                    "committing",
                    "unknown",
                    "interrupted",
                ].includes(String(value.state))
            )
                return fail("Invalid original replacement state");
            await new Promise((resolve) =>
                setTimeout(
                    resolve,
                    Math.min(20, Math.max(0, until - performance.now())),
                ),
            );
        }
    }
    private state(id: string): ConnectionState {
        this.check();
        const state = this.connections.get(id);
        if (!state)
            throw new ConnectError("closed", "Media Connection retired");
        return state;
    }
    nativeActivationBoundary(id: string): NativeActivationBoundary | undefined {
        return this.connections.get(id)?.nativeReadiness?.activationBoundary;
    }
    streams(id: string): Readonly<MediaBinding>[] {
        return [...this.state(id).routes.values()].map((r) => r.binding);
    }
    revision(id: string): number {
        return this.state(id).revision;
    }
    onConnectionClose(id: string, callback: () => void): () => void {
        const state = this.state(id);
        if (state.closed.size >= 16)
            throw new ConnectError(
                "backpressure",
                "Too many Connection resource owners",
            );
        state.closed.add(callback);
        return () => {
            state.closed.delete(callback);
        };
    }
    async subscribe(
        id: string,
        operationId: string,
        participantId: string,
        kinds: ("audio" | "video" | "screen" | "data")[],
        options: RequestOptions,
    ): Promise<void> {
        if (
            !identifier(participantId) ||
            !kinds.length ||
            kinds.some((k) => !["audio", "video", "screen", "data"].includes(k))
        )
            throw new ConnectError(
                "invalid-request",
                "Use explicit participant subscriptions",
            );
        await this.transition(
            this.state(id),
            operationId,
            "stream.subscribe",
            { subscriptions: [{ from_participant: participantId, kinds }] },
            options,
        );
    }
    encodeAudio(
        id: string,
        streamId: string,
        samples: Float32Array,
        timestampUs: number,
    ): boolean {
        const route = this.state(id).routes.get(streamId);
        if (!route?.opus || !route.sender)
            throw new ConnectError(
                "unsupported-media-profile",
                "No negotiated audio sender for this Stream",
            );
        const accepted = route.opus.encode(samples, timestampUs);
        if (accepted) route.captureClock.observe(timestampUs);
        return accepted;
    }
    encodeVideo(id: string, streamId: string, frame: VideoFrame): boolean {
        let route: Route | undefined;
        try {
            route = this.state(id).routes.get(streamId);
        } catch (error) {
            frame.close();
            throw error;
        }
        if (!route?.vp8 || !route.sender) {
            frame.close();
            throw new ConnectError(
                "unsupported-media-profile",
                "No negotiated video sender for this Stream",
            );
        }
        const publication = publicationInput(route.publication, frame);
        if (publication === false) {
            frame.close();
            return false;
        }
        const timestamp = frame.timestamp;
        if (!Number.isSafeInteger(timestamp) || timestamp < 0) {
            frame.close();
            throw new ConnectError("protocol", "Invalid capture timestamp");
        }
        const feedback =
            !this.videoControlling(route) &&
            route.publisherSsrc !== undefined &&
            (route.video?.takeFeedback(
                performance.now(),
                route.publisherSsrc,
            ) ??
                false);
        const accepted = route.vp8.encode(
            frame,
            feedback || !!route.publication?.needed,
        );
        if (accepted) route.captureClock.observe(timestamp);
        return accepted;
    }
    async sendData(
        id: string,
        operationId: string,
        messageId: string,
        streamId: string,
        body: BrowserDataBody,
        options: RequestOptions,
    ): Promise<void> {
        const state = this.state(id);
        await this.stable(state);
        const until = performance.now() + (options.timeoutMs ?? 10000);
        if (
            !identifier(operationId) ||
            !identifier(messageId) ||
            state.data.get(streamId) !== body.type ||
            !state.allowed.has(streamId)
        )
            throw new ConnectError(
                "invalid-request",
                "Use a negotiated data Stream and stable operation/message IDs",
            );
        if (body.type === "text") canonicalJson(body.value);
        const bytes =
            body.type === "binary"
                ? new Uint8Array(body.value)
                : new TextEncoder().encode(
                      body.type === "text"
                          ? body.value
                          : canonicalJson(body.value),
                  );
        if (bytes.length > 65536)
            throw new ConnectError(
                "invalid-request",
                "Inline reliable data exceeds 64 KiB",
            );
        let raw = "";
        if (body.type === "binary")
            for (const value of bytes) raw += String.fromCharCode(value);
        await this.client.request(
            "message.send",
            {
                msg_id: messageId,
                to: "all",
                content_type:
                    body.type === "text"
                        ? "text/plain"
                        : body.type === "json"
                          ? "application/json"
                          : "application/octet-stream",
                label: streamId,
                reliability: { mode: "reliable_ordered" },
                body:
                    body.type === "binary"
                        ? btoa(raw)
                        : new TextDecoder("utf-8", { fatal: true }).decode(
                              bytes,
                          ),
                body_encoding: body.type === "binary" ? "base64" : "utf8",
            },
            {
                ...state.scope,
                operation_id: operationId,
                expected_revision: state.revision,
            },
            options,
        );
        await this.completed(
            state.scope.cid,
            operationId,
            until,
            options.signal,
        );
    }
    private dataReceived(frame: Envelope): void {
        if (typeof frame.connid !== "string") return;
        const state = this.connections.get(frame.connid),
            p = frame.payload;
        if (
            !state ||
            frame.cid !== state.scope.cid ||
            frame.sid !== state.scope.sid
        )
            return;
        const protocol =
            typeof p.label === "string" ? state.data.get(p.label) : undefined;
        if (
            !protocol ||
            !identifier(p.msg_id) ||
            !identifier(frame.operation_id) ||
            !identifier(p.from) ||
            typeof p.body !== "string" ||
            p.body.length > 87384
        )
            return fail("Invalid reliable data binding");
        let body: BrowserDataBody;
        if (protocol === "binary") {
            if (
                p.content_type !== "application/octet-stream" ||
                p.body_encoding !== "base64"
            )
                return fail("Reliable data codec mismatch");
            const raw = atob(p.body);
            if (btoa(raw) !== p.body || raw.length > 65536)
                return fail("Invalid bounded binary body");
            body = {
                type: "binary",
                value: Uint8Array.from(raw, (c) => c.charCodeAt(0)),
            };
        } else {
            const bytes = new TextEncoder().encode(p.body);
            if (
                bytes.length > 65536 ||
                p.body_encoding !== "utf8" ||
                p.content_type !==
                    (protocol === "text" ? "text/plain" : "application/json")
            )
                return fail("Reliable data codec mismatch");
            body =
                protocol === "text"
                    ? { type: "text", value: p.body }
                    : { type: "json", value: parseCanonicalJson(bytes) };
        }
        this.callbacks.data?.({
            conversationId: state.scope.cid,
            sessionId: state.scope.sid,
            connectionId: frame.connid,
            streamId: String(p.label),
            operationId: frame.operation_id,
            messageId: p.msg_id,
            from: p.from,
            body,
        });
    }
    async end(
        id: string,
        operationId: string,
        options: RequestOptions,
    ): Promise<void> {
        const state = this.state(id);
        await this.stable(state);
        await this.client.request(
            "connection.end",
            { reason_code: 200, reason: "normal-clearing" },
            {
                ...state.scope,
                operation_id: operationId,
            },
            options,
        );
        await this.completed(
            state.scope.cid,
            operationId,
            performance.now() + (options.timeoutMs ?? 10000),
            options.signal,
        );
        this.retire(id);
    }
    private remove(route: Route): void {
        route.clock.retire(route.binding.handle);
        this.registry.retire(route.binding.handle);
        this.routes.delete(route.binding.handle);
        route.opus?.close();
        closeVideoPublication(route.publication);
        route.vp8?.close();
        route.assembler?.clear();
        route.order?.clear();
        route.order?.close();
        route.video?.close();
        this.feedbackWrites.delete(route);
    }
    /** Retire local owners after durable end or failed local preparation.
     * Sends no mutation and makes no claim about the remote operation outcome. */
    retireLocalConnection(id: string): void {
        this.retire(id);
    }
    /** Retain a possibly committed original target until authoritative recovery. */
    replacementMayHaveCommitted(id: string): boolean {
        return (
            this.connections.get(id)?.nativeReadiness?.mayHaveCommitted ?? false
        );
    }
    private retire(id: string): void {
        const state = this.connections.get(id);
        if (!state) return;
        state.nativeReadiness?.close();
        for (const route of state.routes.values()) this.remove(route);
        this.connections.delete(id);
        for (const callback of state.closed) {
            try {
                callback();
            } catch {
                /* Retire every resource owner. */
            }
        }
        state.closed.clear();
    }
    close(): void {
        if (this.stopped) return;
        this.stopped = true;
        clearInterval(this.timer);
        this.stopEvents?.();
        this.stopLifetime?.();
        for (const task of this.transitions.values())
            task.fail(
                new ConnectError("closed", "Physical media peer retired"),
            );
        this.transitions.clear();
        for (const id of this.connections.keys()) this.retire(id);
        for (const route of this.routes.values()) this.remove(route);
        this.registry.clear();
        this.queue.clear();
        this.tasks.track(this.reader.cancel());
        this.tasks.track(this.writer.abort());
        this.client.close();
    }
    ownCleanup(work: Promise<unknown>): void {
        this.tasks.track(work, true);
    }
    async shutdown(): Promise<void> {
        this.close();
        try {
            await this.tasks.join();
        } finally {
            this.reader.releaseLock();
            this.writer.releaseLock();
        }
    }
}

export class BrowserMediaConnection {
    constructor(
        private readonly peer: BrowserMediaPeer,
        readonly id: string,
    ) {
        publicationConnection(this, peer, id);
    }
    ownCleanup(work: Promise<unknown>): void {
        this.peer.ownCleanup(work);
    }
    /** Immutable historical native cut; undefined for ordinary or uncompleted peers. */
    get nativeActivationBoundary(): NativeActivationBoundary | undefined {
        return this.peer.nativeActivationBoundary(this.id);
    }
    get streams(): Readonly<MediaBinding>[] {
        return this.peer.streams(this.id);
    }
    get revision(): number {
        return this.peer.revision(this.id);
    }
    onClose(callback: () => void): () => void {
        return this.peer.onConnectionClose(this.id, callback);
    }
    subscribe(
        operationId: string,
        participantId: string,
        kinds: ("audio" | "video" | "screen" | "data")[],
        options: RequestOptions = {},
    ): Promise<void> {
        return this.peer.subscribe(
            this.id,
            operationId,
            participantId,
            kinds,
            options,
        );
    }
    encodeAudio(
        streamId: string,
        samples: Float32Array,
        timestampUs: number,
    ): boolean {
        return this.peer.encodeAudio(this.id, streamId, samples, timestampUs);
    }
    encodeVideo(streamId: string, frame: VideoFrame): boolean {
        return this.peer.encodeVideo(this.id, streamId, frame);
    }
    sendData(
        operationId: string,
        messageId: string,
        streamId: string,
        body: BrowserDataBody,
        options: RequestOptions = {},
    ): Promise<void> {
        return this.peer.sendData(
            this.id,
            operationId,
            messageId,
            streamId,
            body,
            options,
        );
    }
    end(operationId: string, options: RequestOptions = {}): Promise<void> {
        return this.peer.end(this.id, operationId, options);
    }
}
