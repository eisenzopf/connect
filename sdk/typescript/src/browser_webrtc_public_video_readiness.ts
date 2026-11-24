import { postCutRtpEligible, type WebRtcBridgeFeeding, type WebRtcBridgeReadyMarker, type WebRtcReceiverPostCut } from "./browser_webrtc_public_video_bridge.js";
import { BrowserDeadline } from "./browser_deadline.js";
import type { ReceiverRtpOutput } from "./browser_receiver_video_release.js";
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
    /** SAME exact receiver callback. Absence cannot be replaced by stats or a clock. */
    lastDecodedRtpOutput(): Readonly<ReceiverRtpOutput> | undefined;
    take(activated: Readonly<WebRtcPublicVideoPhase>, track: MediaStreamTrack): WebRtcPreparedVideo | undefined;
    release(): Promise<void>;
}
export interface WebRtcPreCutPublicVideo extends WebRtcPreparedPublicVideo {
    assertPreCutCurrent():void;
    bridgeReady(feeding:Readonly<WebRtcBridgeFeeding>,readKeyframes:()=>Promise<readonly number[]|undefined>):Promise<Readonly<WebRtcBridgeReadyMarker>>;
    postCut(witness:Readonly<WebRtcReceiverPostCut>):Promise<WebRtcPreparedPublicVideo>;
}
interface PreCutAttempt {peer:WeakRef<RTCPeerConnection>;roles:WeakRef<WebRtcReceiveRoles>;signal:WeakRef<AbortSignal>;budget:BrowserDeadline;promise:Promise<WebRtcPreCutPublicVideo>;}
const preCutAttempts=new WeakMap<WebRtcPublicVideoPhaseOwner,PreCutAttempt>();
/** Attach the SAME hidden B before private A/audio Ready. No decode/public
 * evidence is fabricated by attachment and the element is never appended. */
export function prearmWebRtcPublicVideoBridge(owner:WebRtcPublicVideoPhaseOwner,peer:RTCPeerConnection,roles:WebRtcReceiveRoles,budget:BrowserDeadline,signal:AbortSignal,trackTask:TrackTask):Promise<WebRtcPreCutPublicVideo> {
    try {
        owner.check();if(!owner.bridgeEnabled()||!owner.exactLocalPeer(peer))throw changed();
        const previous=preCutAttempts.get(owner);
        if(previous) {if(previous.peer.deref()!==peer||previous.roles.deref()!==roles||previous.signal.deref()!==signal||previous.budget!==budget)throw changed();return previous.promise;}
        const phase=owner.negotiated();let resolve!:(v:WebRtcPreCutPublicVideo)=>void,reject!:(e:unknown)=>void;
        const promise=new Promise<WebRtcPreCutPublicVideo>((yes,no)=>{resolve=yes;reject=no;});preCutAttempts.set(owner,{peer:new WeakRef(peer),roles:new WeakRef(roles),signal:new WeakRef(signal),budget,promise});
        void new OriginalVideoObservation(owner,peer,roles,phase,budget,signal,trackTask,true).prearm().then(resolve,reject);return promise;
    }catch(error){return Promise.reject(error);}
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
        if(owner.bridgeEnabled())throw unsupported(); // never attach a second post-F decoder for prearmed B
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

class OriginalVideoObservation implements WebRtcPreCutPublicVideo {
    private picture?: HTMLVideoElement;
    private callback?: number;
    private original?: Readonly<WebRtcReceiveRoleBinding>;
    private marker?: WebRtcPublicVideoReadyMarker;
    // Numeric metadata only: retain the first callback and bounded recent B
    // callbacks. No pixels, decoder reset or public presentation is retained.
    private readonly preCutRtp:Readonly<ReceiverRtpOutput>[]=[];
    private bridgePromise?:Promise<Readonly<WebRtcBridgeReadyMarker>>;
    private postCutPromise?:Promise<WebRtcPreparedPublicVideo>;
    private witness?:Readonly<WebRtcReceiverPostCut>;
    private failure?: Error;
    private stopped = false;
    private adopted = false;
    private presented = 0;
    private rtpOutput?: Readonly<ReceiverRtpOutput>;
    private frameCount = 0;
    private detachOwner?: () => void;
    constructor(
        private owner: WebRtcPublicVideoPhaseOwner | undefined,
        private peer: RTCPeerConnection | undefined,
        private roles: WebRtcReceiveRoles | undefined,
        private phase: Readonly<WebRtcPublicVideoPhase>,
        private readonly budget: BrowserDeadline,
        private signal: AbortSignal | undefined,
        private trackTask: TrackTask | undefined,
        private preCut=false,
    ) {}
    get readiness(): WebRtcPublicVideoReadyMarker {
        if (!this.marker) throw new ConnectError("conflict", "Original B has not decoded a current frame");
        return this.marker;
    }
    lastDecodedRtpOutput(): Readonly<ReceiverRtpOutput> | undefined {
        this.assertCurrent();return this.rtpOutput;
    }
    assertCurrent(): void {
        if (!this.marker || this.adopted || !this.picture) throw changed();
        this.check();
        if(this.witness&&!postCutRtpEligible(this.witness,this.rtpOutput))throw changed();
    }
    private check(): void {
        if (this.failure) throw this.failure;
        if (this.signal?.aborted) throw new ConnectError("cancelled", "Original reserved B observation cancelled");
        if (this.stopped || !this.owner || !this.peer || !this.roles || !(this.preCut?this.owner.acceptsPreCutBinding(this.phase):this.owner.accepts(this.phase)) || !this.owner.exactLocalPeer(this.peer) || ["closed", "failed"].includes(this.peer.connectionState)) throw new ConnectError("closed", "Original reserved B owner retired");
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
                // VideoFrameCallbackMetadata.rtpTimestamp is optional across
                // engines. The release profile MUST fail closed when absent;
                // mediaTime/presentedFrames are not RTP timestamp substitutes.
                const rtpTimestamp = (metadata as VideoFrameCallbackMetadata & {rtpTimestamp?: number}).rtpTimestamp;
                this.rtpOutput = Number.isSafeInteger(rtpTimestamp) && Number(rtpTimestamp) >= 0 && Number(rtpTimestamp) <= 0xffffffff
                    ? Object.freeze({rtpTimestamp: rtpTimestamp!, presentedFrames: this.presented}) : undefined;
                if (this.preCut && this.rtpOutput) {
                    if(!this.preCutRtp.some(value=>value.rtpTimestamp===this.rtpOutput!.rtpTimestamp)) {
                        if(this.preCutRtp.length===64)this.preCutRtp.splice(1,1);
                        this.preCutRtp.push(this.rtpOutput);
                    }
                }
                if (!this.marker) this.frameCount++;
            } else if (this.presented > 0) throw new ConnectError("media", "Original B presentation counter became unavailable");
            this.callback = this.picture.requestVideoFrameCallback(this.frame);
        } catch (error) {
            this.failure = error instanceof Error ? error : unsupported();
            this.stopFromOwner();
        }
    };
    private decoded(report: RTCStatsReport): {frames: number; keys: number | null} | undefined {
        // A newly negotiated receiver can publish an inbound stats row before
        // it has decoded anything or attached the selected codec report.
        if (!this.frameCount) return undefined;
        const expected = this.phase.binding, original = this.original!;
        const rows: RTCInboundRtpStreamStats[] = [];
        let pendingCodec = false;
        if (typeof report.forEach !== "function" || typeof report.get !== "function") throw unsupported();
        report.forEach((value) => {
            if (value.type !== "inbound-rtp" || value.isRemote === true || (value.kind ?? value.mediaType) !== "video") return;
            if ((value.trackIdentifier !== undefined && value.trackIdentifier !== original.track.id) || (value.mid !== undefined && value.mid !== original.mid) || !nonnegative(value.ssrc) || value.ssrc > 0xffffffff) throw changed();
            if (value.codecId === undefined || value.codecId === "") {
                pendingCodec = true;
                return;
            }
            if (typeof value.codecId !== "string") throw changed();
            const codec = report.get(value.codecId);
            if (!codec) {
                pendingCodec = true;
                return;
            }
            if (codec.type !== "codec" || typeof codec.mimeType !== "string") throw changed();
            if (codec.id !== undefined && codec.id !== value.codecId) throw changed();
            if (value.ssrc !== expected.primary_ssrc) {
                if (codec.mimeType.toLowerCase() !== "video/rtx") throw changed();
                return;
            }
            if (rows.length || codec.mimeType.toLowerCase() !== "video/vp8" || codec.clockRate !== expected.clock_rate_hz || codec.payloadType !== expected.payload_type) throw changed();
            rows.push(value);
        });
        if (!rows.length || pendingCodec) return undefined;
        const row = rows[0];
        if (row.framesDecoded !== undefined && !nonnegative(row.framesDecoded)) throw changed();
        if (row.keyFramesDecoded !== undefined && !nonnegative(row.keyFramesDecoded)) throw changed();
        const frames = typeof row.framesDecoded === "number" && row.framesDecoded > 0 ? row.framesDecoded : this.frameCount;
        const keys = row.keyFramesDecoded === undefined ? null : row.keyFramesDecoded;
        if (keys !== null && (keys < 1 || keys > frames)) return undefined;
        return {frames, keys};
    }
    private async attach(waitForPlay=true):Promise<void> {
        if(this.picture)return;
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
            const playing=this.run(() => this.picture!.play());
            if(waitForPlay)await playing;
            else void playing.catch(error=>{
                if(this.stopped||this.adopted)return;
                this.failure=error instanceof Error?error:unsupported();this.stopFromOwner();
            });
    }
    async prearm():Promise<WebRtcPreCutPublicVideo> {
        try{await this.attach(false);return this;}catch(error){const task=this.trackTask,cleanup=this.release();if(task)void task(cleanup,true).catch(()=>{});else void cleanup.catch(()=>{});throw error;}
    }
    assertPreCutCurrent():void {if(!this.preCut||!this.picture)throw changed();this.check();}
    bridgeReady(feeding:Readonly<WebRtcBridgeFeeding>,readKeyframes:()=>Promise<readonly number[]|undefined>):Promise<Readonly<WebRtcBridgeReadyMarker>> {
        if(this.bridgePromise)return this.bridgePromise;
        let resolve!:(v:Readonly<WebRtcBridgeReadyMarker>)=>void,reject!:(e:unknown)=>void;
        const promise=new Promise<Readonly<WebRtcBridgeReadyMarker>>((yes,no)=>{resolve=yes;reject=no;});this.bridgePromise=promise;
        void(async()=>{
            if(!this.preCut||!this.owner?.acceptsBridgeFeeding(feeding))throw changed();
            for(;;){
                this.check();if(Date.now()>=feeding.bridge_readiness.expires_at*1000)throw new ConnectError("timeout","Original B bridge challenge expired");
                // Only the SAME verified original status reader supplies these
                // complete-keyframe writer receipts. No counters or first delta
                // callback may stand in for an exact authenticated coordinate.
                const retained=await this.run(readKeyframes);this.check();
                const decoded=retained&&this.preCutRtp.find(value=>retained.includes(value.rtpTimestamp));
                if (!decoded) {
                    await this.run(() => new Promise<void>(yes => setTimeout(yes, Math.min(20, this.budget.remaining()))));
                    continue;
                }
                const report=await this.run(()=>this.original!.receiver.getStats());this.check();const output=this.decoded(report);
                if(output) {
                    const marker:Readonly<WebRtcBridgeReadyMarker>=Object.freeze({profile:"connect-replacement-public-video-bridge-ready-v1",operation_record_id:feeding.operation_record_id,
                        bridge_readiness:Object.freeze({reservation_id:feeding.reservation_id,nonce:feeding.bridge_readiness.nonce,binding:feeding.binding,decoded_rtp_timestamp:decoded.rtpTimestamp,decoded_frames:output.frames,decoder_startup:"fresh-b-receiver-precut-keyframe"})});
                    this.owner!.expectBridgeMarker(marker);return marker;
                }
                await this.run(()=>new Promise<void>(yes=>setTimeout(yes,Math.min(20,this.budget.remaining()))));
            }
        })().then(resolve,error=>{this.failure=error instanceof Error?error:unsupported();this.stopFromOwner();reject(error);});return promise;
    }
    postCut(witness:Readonly<WebRtcReceiverPostCut>):Promise<WebRtcPreparedPublicVideo> {
        if(this.postCutPromise) {if(this.witness!==witness)throw changed();return this.postCutPromise;}
        if(!this.preCut||!this.bridgePromise||!this.owner)throw changed();
        this.phase=this.owner.cutBound();this.witness=witness;this.preCut=false;this.preCutRtp.length=0;
        this.postCutPromise=this.start();return this.postCutPromise;
    }
    async start(): Promise<WebRtcPreparedPublicVideo> {
        try {
            await this.attach();
            for (;;) {
                this.check();
                const report = await this.run(() => this.original!.receiver.getStats());
                this.check();
                const output = this.decoded(report);
                if (output&&(!this.witness||postCutRtpEligible(this.witness,this.rtpOutput))) {
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
        this.assertCurrent();
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
        this.peer = undefined;this.roles = undefined;this.original = undefined;this.rtpOutput = undefined;this.preCutRtp.length=0;
        this.owner = undefined;this.signal = undefined;this.trackTask = undefined;
        return failure === undefined ? Promise.resolve() : Promise.reject(failure);
    }
}
