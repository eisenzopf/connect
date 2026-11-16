import { BrowserDeadline } from "./browser_deadline.js";
import { ConnectError } from "./errors.js";
import type { WebRtcPreparedVideo } from "./browser_webrtc_readiness.js";
import { WebRtcReceiveRoles, type WebRtcReceiveRoleBinding } from "./browser_webrtc_receive_roles.js";
import { WebRtcPublicVideoPhaseOwner, type WebRtcPublicVideoPhase, type WebRtcPublicVideoBinding } from "./browser_webrtc_public_video_phase.js";

type TrackTask = <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>;
export interface WebRtcPublicVideoReadyMarker {
    readonly profile: "connect-replacement-public-video-ready-v1";
    readonly operation_record_id: string;
    readonly public_video_readiness: Readonly<{
        reservation_id: string;
        source_cut_id: string;
        nonce: string;
        binding: Readonly<WebRtcPublicVideoBinding>;
        decoded_frames: number;
        decoded_keyframes: number | null;
        decoder_startup: "fresh-receiver-first-decoded-frame";
    }>;
}
export interface WebRtcPreparedPublicVideo {
    readonly readiness: WebRtcPublicVideoReadyMarker;
    assertCurrent(): void;
    take(activated: Readonly<WebRtcPublicVideoPhase>, track: MediaStreamTrack): WebRtcPreparedVideo | undefined;
    release(): Promise<void>;
}
interface OriginalAttempt {
    peer: WeakRef<RTCPeerConnection>;
    roles: WeakRef<WebRtcReceiveRoles>;
    signal: WeakRef<AbortSignal>;
    budget: BrowserDeadline;
    phase: Readonly<WebRtcPublicVideoPhase>;
    promise: Promise<WebRtcPreparedPublicVideo>;
}
const attempts = new WeakMap<WebRtcPublicVideoPhaseOwner, OriginalAttempt>();
const unsupported = () => new ConnectError("unsupported-media-profile", "Original reserved B decoded observation is unavailable");
const changed = () => new ConnectError("protocol", "Original reserved B receiver binding changed");
const nonnegative = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;

/** One original B observation, reached only after the owning workflow observes
 * the original signed cut-bound-post-f phase and its exact native role. */
export function observeWebRtcPublicVideoReadiness(
    owner: WebRtcPublicVideoPhaseOwner,
    peer: RTCPeerConnection,
    roles: WebRtcReceiveRoles,
    budget: BrowserDeadline,
    signal: AbortSignal,
    trackTask: TrackTask,
): Promise<WebRtcPreparedPublicVideo> {
    try {
        owner.check();
        const previous = attempts.get(owner);
        if (previous) {
            if (previous.peer.deref() !== peer || previous.roles.deref() !== roles || previous.signal.deref() !== signal || previous.budget !== budget || !owner.accepts(previous.phase)) throw changed();
            return previous.promise;
        }
        const phase = owner.cutBound();
        if (!owner.exactLocalPeer(peer)) throw changed();
        const observation = new OriginalVideoObservation(owner, peer, roles, phase, budget, signal, trackTask);
        let resolve!: (value: WebRtcPreparedPublicVideo) => void, reject!: (error: unknown) => void;
        const promise = new Promise<WebRtcPreparedPublicVideo>((yes, no) => {resolve = yes; reject = no;});
        // Reserve this SAME attempt before any native getter/play callback can
        // reenter. Even failure/release retains a tombstone, never a new lease.
        attempts.set(owner, {peer: new WeakRef(peer), roles: new WeakRef(roles), signal: new WeakRef(signal), budget, phase, promise});
        void promise.catch(() => {});
        void observation.start().then(resolve, reject);
        return promise;
    } catch (error) { return Promise.reject(error); }
}

class OriginalVideoObservation implements WebRtcPreparedPublicVideo {
    private picture?: HTMLVideoElement;
    private callback?: number;
    private original?: Readonly<WebRtcReceiveRoleBinding>;
    private marker?: WebRtcPublicVideoReadyMarker;
    private failure?: Error;
    private stopped = false;
    private adopted = false;
    private presented = 0;
    private frameCount = 0;
    private detachOwner?: () => void;
    constructor(
        private owner: WebRtcPublicVideoPhaseOwner | undefined,
        private peer: RTCPeerConnection | undefined,
        private roles: WebRtcReceiveRoles | undefined,
        private readonly phase: Readonly<WebRtcPublicVideoPhase>,
        private readonly budget: BrowserDeadline,
        private signal: AbortSignal | undefined,
        private trackTask: TrackTask | undefined,
    ) {}
    get readiness(): WebRtcPublicVideoReadyMarker {
        if (!this.marker) throw new ConnectError("conflict", "Original B has not decoded a current frame");
        return this.marker;
    }
    assertCurrent(): void {
        if (!this.marker || this.adopted || !this.picture) throw changed();
        this.check();
    }
    private check(): void {
        if (this.failure) throw this.failure;
        if (this.signal?.aborted) throw new ConnectError("cancelled", "Original reserved B observation cancelled");
        if (this.stopped || !this.owner || !this.peer || !this.roles || !this.owner.accepts(this.phase) || !this.owner.exactLocalPeer(this.peer) || ["closed", "failed"].includes(this.peer.connectionState)) throw new ConnectError("closed", "Original reserved B owner retired");
        this.owner.check();
        // The original deadline also owns the retained, unadopted picture.
        // Resolving readiness never creates a new activation/transfer budget.
        try { this.budget.remaining(); } catch (error) {
            this.failure = error instanceof Error ? error : unsupported();
            this.stopFromOwner();
            throw error;
        }
        const current = this.roles.binding("public");
        if (!current || current.streamId !== this.phase.binding.stream_id || current.mid !== this.phase.binding.mid || current.track.kind !== "video" || current.track.readyState !== "live" || (this.original && (current.transceiver !== this.original.transceiver || current.receiver !== this.original.receiver || current.track !== this.original.track))) throw changed();
        this.original ??= current;
        if (typeof current.receiver.getStats !== "function") throw unsupported();
    }
    private run<T>(work: () => Promise<T>): Promise<T> {
        this.check();
        const track = this.trackTask!;
        return this.budget.run(() => track(work()), this.signal);
    }
    private stopFromOwner = () => {
        this.failure ??= new ConnectError("closed", "Original reserved B phase retired");
        const track = this.trackTask;
        const work = this.release();
        if (track) void track(work, true).catch(() => {}); else void work.catch(() => {});
    };
    private cancel = () => {
        this.failure ??= new ConnectError("cancelled", "Original reserved B observation cancelled");
        const track = this.trackTask;
        const work = this.release();
        if (track) void track(work, true).catch(() => {}); else void work.catch(() => {});
    };
    private frame: VideoFrameRequestCallback = (_now, metadata) => {
        if (this.stopped || this.adopted || !this.picture) return;
        try {
            this.check();
            if (metadata.width > 0 && metadata.height > 0 && Number.isFinite(metadata.mediaTime) && Number.isSafeInteger(metadata.presentedFrames) && metadata.presentedFrames > this.presented) {
                this.presented = metadata.presentedFrames;
                if (!this.marker) this.frameCount++;
            } else if (this.presented > 0) throw new ConnectError("media", "Original B presentation counter became unavailable");
            this.callback = this.picture.requestVideoFrameCallback(this.frame);
        } catch (error) {
            this.failure = error instanceof Error ? error : unsupported();
            this.stopFromOwner();
        }
    };
    private decoded(report: RTCStatsReport): {frames: number; keys: number | null} | undefined {
        const expected = this.phase.binding, original = this.original!;
        const rows: RTCInboundRtpStreamStats[] = [];
        if (typeof report.forEach !== "function" || typeof report.get !== "function") throw unsupported();
        report.forEach((value) => {
            if (value.type !== "inbound-rtp" || value.isRemote === true || (value.kind ?? value.mediaType) !== "video") return;
            if ((value.trackIdentifier !== undefined && value.trackIdentifier !== original.track.id) || (value.mid !== undefined && value.mid !== original.mid) || !nonnegative(value.ssrc) || value.ssrc > 0xffffffff) throw changed();
            if (typeof value.codecId !== "string" || !value.codecId) throw unsupported();
            const codec = report.get(value.codecId);
            if (!codec || codec.type !== "codec" || typeof codec.mimeType !== "string") throw unsupported();
            if (codec.id !== undefined && codec.id !== value.codecId) throw changed();
            if (value.ssrc !== expected.primary_ssrc) {
                if (codec.mimeType.toLowerCase() !== "video/rtx") throw changed();
                return;
            }
            if (rows.length || codec.mimeType.toLowerCase() !== "video/vp8" || codec.clockRate !== expected.clock_rate_hz || codec.payloadType !== expected.payload_type) throw changed();
            rows.push(value);
        });
        if (!rows.length || !this.frameCount) return undefined;
        const row = rows[0];
        if (row.framesDecoded !== undefined && !nonnegative(row.framesDecoded)) throw changed();
        if (row.keyFramesDecoded !== undefined && !nonnegative(row.keyFramesDecoded)) throw changed();
        const frames = typeof row.framesDecoded === "number" && row.framesDecoded > 0 ? row.framesDecoded : this.frameCount;
        const keys = row.keyFramesDecoded === undefined ? null : row.keyFramesDecoded;
        if (keys !== null && (keys < 1 || keys > frames)) return undefined;
        return {frames, keys};
    }
    async start(): Promise<WebRtcPreparedPublicVideo> {
        try {
            this.check();
            if (typeof document === "undefined" || typeof MediaStream !== "function") throw unsupported();
            this.detachOwner = this.owner!.onRetire(this.stopFromOwner);
            this.signal!.addEventListener("abort", this.cancel, {once: true});
            this.picture = document.createElement("video");
            if (typeof this.picture.requestVideoFrameCallback !== "function" || typeof this.picture.cancelVideoFrameCallback !== "function") throw unsupported();
            this.picture.muted = true;
            this.picture.playsInline = true;
            this.picture.srcObject = new MediaStream([this.original!.track]);
            this.callback = this.picture.requestVideoFrameCallback(this.frame);
            await this.run(() => this.picture!.play());
            for (;;) {
                this.check();
                const report = await this.run(() => this.original!.receiver.getStats());
                this.check();
                const output = this.decoded(report);
                if (output) {
                    this.marker = Object.freeze({profile: "connect-replacement-public-video-ready-v1",
                        operation_record_id: this.phase.operation_record_id,
                        public_video_readiness: Object.freeze({reservation_id: this.phase.reservation_id,
                            source_cut_id: this.phase.source_cut_id!, nonce: this.phase.public_readiness!.nonce,
                            binding: this.phase.binding, decoded_frames: output.frames, decoded_keyframes: output.keys,
                            decoder_startup: "fresh-receiver-first-decoded-frame"}),
                    });
                    return this;
                }
                await this.run(() => new Promise<void>(resolve => setTimeout(resolve, Math.min(20, this.budget.remaining()))));
            }
        } catch (error) {
            const track = this.trackTask;
            const cleanup = this.release();
            if (track) void track(cleanup, true).catch(() => {}); else void cleanup.catch(() => {});
            throw error;
        }
    }
    take(activated: Readonly<WebRtcPublicVideoPhase>, track: MediaStreamTrack): WebRtcPreparedVideo | undefined {
        if (!this.marker || this.adopted || !this.owner?.activated(activated)) return undefined;
        this.check();
        if (this.original!.track !== track || !this.picture || this.presented < 1) return undefined;
        const element = this.picture;
        if (this.callback !== undefined) element.cancelVideoFrameCallback(this.callback);
        this.callback = undefined;
        this.adopted = true;
        this.picture = undefined;
        const output = Object.freeze({element, presentedFloor: this.presented});
        void this.release().catch(() => {});
        return output;
    }
    release(): Promise<void> {
        if (this.stopped) return Promise.resolve();
        this.stopped = true;
        this.detachOwner?.();this.detachOwner = undefined;
        this.signal?.removeEventListener("abort", this.cancel);
        const element = this.picture;this.picture = undefined;
        let failure: unknown;
        if (element && this.callback !== undefined) try { element.cancelVideoFrameCallback(this.callback); } catch (error) { failure ??= error; }
        this.callback = undefined;
        if (element) {
            try { element.pause(); } catch (error) { failure ??= error; }
            try { element.srcObject = null; } catch (error) { failure ??= error; }
        }
        this.peer = undefined;this.roles = undefined;this.original = undefined;
        this.owner = undefined;this.signal = undefined;this.trackTask = undefined;
        return failure === undefined ? Promise.resolve() : Promise.reject(failure);
    }
}
