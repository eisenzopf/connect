import { bridgeClosed, bridgeCanonical, parseWebRtcBridgeOptIn, type WebRtcBridgeOptIn, type WebRtcBridgeReadyMarker } from "./browser_webrtc_public_video_bridge.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserDeadline } from "./browser_deadline.js";
import {
    replacementFrame,
    replacementReceipt,
    replacementRoot,
    type BrowserReplacementReceipt,
    type BrowserReplacementRoot,
} from "./browser_replacement.js";
import { ConnectError, recoverableReadRefusal } from "./errors.js";
import {
    canonicalJson,
    jsonObject,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier, type Scope } from "./uctp_wire.js";
import type { OriginalWebRtcReceiveReservation } from "./browser_webrtc_receive_reservation.js";
import type { WebRtcPublicVideoReadyMarker } from "./browser_webrtc_public_video_readiness.js";

export interface WebRtcReplacementChallenge {
    readonly nonce: string;
    readonly source_generation: number;
    readonly target_generation: number;
    readonly stream_revision: number;
}
export interface WebRtcDecodedReadiness {
    readonly audio: {
        readonly stream_id: string;
        readonly decoded_samples: number;
    };
    readonly video: null | {
        readonly stream_id: string;
        readonly decoded_frames: number;
        readonly decoded_keyframes: number | null;
        readonly decoder_startup: "fresh-peer-first-decoded-frame";
    };
}
const protocol = (message: string) => new ConnectError("protocol", message);
const copy = (value: JsonObject): JsonObject =>
    parseCanonicalJson(
        new TextEncoder().encode(canonicalJson(value)),
    ) as JsonObject;
const originalStatusSnapshot = (value: JsonObject): JsonObject => {
    // The brand belongs to these immutable original bytes, never to a mutable
    // payload retained by a caller or the transport response owner.
    const snapshot = copy(value);
    const freeze = (item: Json): void => {
        if (item !== null && typeof item === "object") {
            for (const child of Object.values(item)) freeze(child);
            Object.freeze(item);
        }
    };
    freeze(snapshot);
    return snapshot;
};
const positive = (value: unknown): value is number =>
    typeof value === "number" && Number.isSafeInteger(value) && value > 0;

export const recoverableReadinessFailure = (error: unknown): boolean => {
    if (!(error instanceof ConnectError)) return false;
    if (error.code === "timeout" || error.code === "transport") return true;
    return recoverableReadRefusal(error);
};

/** One target's original replacement phases. A readiness assertion is a signed
 * observation of this fresh decoder, never a new replacement mutation. */
export class BrowserWebRtcReplacement {
    readonly root: BrowserReplacementRoot;
    private allocated = false;
    private readyAttempted = false;
    private refused = false;
    private original?: JsonObject;
    private originalScope?: Scope;
    private observedRevision = -1;
    private selected?: WebRtcReplacementChallenge;
    private readyOffer?: JsonObject;
    private publicReadyOffer?: JsonObject;
    private bridgeReadyOffer?:JsonObject;
    private bridgeOptIn?:Readonly<WebRtcBridgeOptIn>;
    private readonly originalObservations = new WeakSet<JsonObject>();
    constructor(
        private readonly client: BrowserClient,
        root: BrowserReplacementRoot,
        private readonly receiveReservation?: OriginalWebRtcReceiveReservation,
    ) {
        this.root = replacementRoot(root);
        if (
            client.controlTransport !== "websocket" ||
            client.deviceId !== root.deviceId
        )
            throw new ConnectError(
                "invalid-request",
                "Replacement belongs to the current signed target Device",
            );
    }
    get mayHaveCommitted(): boolean {
        return (this.receiveReservation ? !!this.publicReadyOffer : this.readyAttempted) && !this.refused;
    }
    get challenge(): WebRtcReplacementChallenge {
        if (!this.selected)
            throw new ConnectError(
                "conflict",
                "Replacement has no current decoder challenge",
            );
        return this.selected;
    }
    get savedReadiness(): JsonObject | undefined {
        return this.readyOffer ? copy(this.readyOffer) : undefined;
    }
    private observe(value: JsonObject): JsonObject {
        if (Number(value.revision) < this.observedRevision)
            throw protocol("Original replacement revision regressed");
        this.observedRevision = Number(value.revision);
        return value;
    }
    private async status(options: RequestOptions): Promise<JsonObject> {
        const root = this.root;
        const frames = await this.client.requestReplacementContinuation(
            "operation.get",
            { operation_id: root.operationId },
            {
                cid: root.conversationId,
                sid: root.sessionId,
                connid: root.sourceConnectionId,
            },
            options,
        );
        const value=this.observe(
            replacementFrame(root, frames, root.sourceConnectionId),
        );
        const observed=this.receiveReservation ? originalStatusSnapshot(value) : value;
        this.originalObservations.add(observed);
        return observed;
    }
    async allocate(
        payload: JsonObject,
        scope: Scope,
        options: RequestOptions,
        attempted?: () => void,
        originalBudget?: BrowserDeadline,
    ): Promise<void> {
        const root = this.root;
        if (
            this.allocated ||
            this.original ||
            scope.cid !== root.conversationId ||
            scope.sid !== root.sessionId ||
            scope.connid !== root.targetConnectionId ||
            scope.operation_id !== root.operationId ||
            scope.expected_revision !== 0
        )
            throw new ConnectError(
                "conflict",
                "Use the original WebRTC replacement allocation once",
            );
        const budget = originalBudget ?? new BrowserDeadline(options.timeoutMs ?? 10000);
        const observed = await this.status({
            ...options,
            timeoutMs: budget.remaining(),
        });
        if (
            observed.state !== "preparing" ||
            observed.native_replacement_prepared !== true
        )
            throw new ConnectError(
                "conflict",
                "Original replacement preparation is not current",
            );
        if(payload.webrtc_public_video_bridge_opt_in!==undefined)throw protocol("Bridge intent must come from the original signed operation");
        if(observed.webrtc_public_video_bridge!==undefined) {
            if(!this.receiveReservation)throw protocol("Bridge requires the original explicit reserved B layout");
            const snapshot=bridgeClosed(observed.webrtc_public_video_bridge,["opt_in","feeding","decoded_ready"]);
            this.bridgeOptIn=parseWebRtcBridgeOptIn(snapshot.opt_in);
        }
        this.original = {...copy(payload),...(this.bridgeOptIn?{webrtc_public_video_bridge_opt_in:{...this.bridgeOptIn}}:{})};
        this.originalScope = { ...scope };
        attempted?.();
        const frames = await this.client.requestReplacementContinuation(
            "connection.offer",
            {
                ...copy(this.original!),
                replacement_preparation: {
                    profile: "connect-replacement-preparation-v1",
                    operation_record_id: root.recordId,
                },
            },
            { ...scope },
            { ...options, timeoutMs: budget.remaining() },
        );
        const allocated = this.observe(
            replacementFrame(root, frames, root.targetConnectionId),
        );
        if (allocated.state !== "preparing")
            throw new ConnectError(
                "conflict",
                "Original WebRTC replacement did not remain preparing",
            );
        this.allocated = true;
    }
    mediaOffer(payload: JsonObject): JsonObject {
        if(payload.webrtc_public_video_bridge_opt_in!==undefined&&(!this.bridgeOptIn||bridgeCanonical(payload.webrtc_public_video_bridge_opt_in)!==bridgeCanonical(this.bridgeOptIn)))throw protocol("Original signed bridge echo changed");
        if (
            !this.allocated ||
            !this.original ||
            canonicalJson({...payload,...(this.bridgeOptIn?{webrtc_public_video_bridge_opt_in:{...this.bridgeOptIn}}:{})}) !== canonicalJson(this.original)
        )
            throw protocol(
                "WebRTC replacement media changed the original immutable offer",
            );
        return {
            ...copy(this.original!),
            replacement_preparation: {
                profile: "connect-replacement-media-v1",
                operation_record_id: this.root.recordId,
            },
        };
    }
    answer(payload: JsonObject): void {
        const marker = payload.replacement_preparation;
        const challenge = jsonObject(marker) ? marker.readiness : undefined;
        if (
            !this.allocated ||
            this.selected ||
            !jsonObject(marker) ||
            marker.profile !== "connect-replacement-media-v1" ||
            marker.operation_record_id !== this.root.recordId ||
            !jsonObject(challenge) ||
            typeof challenge.nonce !== "string" ||
            !identifier(challenge.nonce) ||
            !positive(challenge.source_generation) ||
            !positive(challenge.target_generation) ||
            challenge.target_generation !== challenge.source_generation + 1 ||
            !positive(challenge.stream_revision)
        )
            throw protocol(
                "Replacement answer changed its current decoder challenge",
            );
        this.selected = Object.freeze({
            nonce: challenge.nonce,
            source_generation: challenge.source_generation,
            target_generation: challenge.target_generation,
            stream_revision: challenge.stream_revision,
        });
    }
    private completedReceipt(value: JsonObject): BrowserReplacementReceipt {
        const root = this.root, challenge = this.challenge;
        const streams = this.original?.streams_offered;
        if (!Array.isArray(streams)) throw protocol("Replacement has no frozen stream offer");
        const receipt = replacementReceipt(root, value);
        const result = value.result as JsonObject;
        const actual = (result.readiness as JsonObject)
            .streams as string[];
        const offered = new Set(
            streams.filter(jsonObject).map((s) => String(s.id)),
        );
        if (
            receipt.generation !== challenge.target_generation ||
            receipt.streamRevision <= challenge.stream_revision ||
            actual.length !== offered.size ||
            new Set(actual).size !== actual.length ||
            actual.some((id) => !offered.has(id))
        )
            throw protocol(
                "Replacement receipt changed the prepared generation or streams",
            );
        return receipt;
    }
    /** Internal reserved-profile context. The caller still owns the exact native
     * peer/roles; no plain option or caller boolean enables the workflow. */
    reservedContext() {
        if(this.client.closed)throw new ConnectError("closed","Original reserved control client retired");
        if (!this.receiveReservation || !this.allocated || !this.original || !this.originalScope || !this.selected)
            throw new ConnectError("unsupported-media-profile", "Original reserved B challenge is unavailable");
        return Object.freeze({root:this.root,reservation:this.receiveReservation,challenge:this.challenge,bridgeOptIn:this.bridgeOptIn});
    }
    private checkReservedStatus(value: JsonObject, phase: string): void {
        this.reservedContext();
        const observed=value.webrtc_public_video_phase;
        if (!this.originalObservations.has(value) || value.state!=="preparing" || value.native_replacement_prepared!==true ||
            !jsonObject(observed) || observed.phase!==phase)
            throw protocol("Original public video continuation is not current");
    }
    async reservedStatus(options: RequestOptions): Promise<JsonObject> {
        const context=this.reservedContext(),value=await this.status(options);
        const intent=value.webrtc_video_receive_reservation;
        if (intent!==undefined && canonicalJson(intent)!==canonicalJson(context.reservation.intent as unknown as JsonObject))
            throw protocol("Original public video reservation changed");
        if (value.state==="preparing" && intent===undefined)
            throw protocol("Original public video reservation disappeared");
        if(this.bridgeOptIn) {
            const b=bridgeClosed(value.webrtc_public_video_bridge,["opt_in","feeding","decoded_ready"]);
            if(bridgeCanonical(parseWebRtcBridgeOptIn(b.opt_in))!==bridgeCanonical(this.bridgeOptIn))throw protocol("Original signed bridge opt-in changed");
        } else if(value.webrtc_public_video_bridge!==undefined)throw protocol("Bridge cannot be enabled after original allocation");
        if (["failed","cancelled"].includes(String(value.state))) this.refused=true;
        return value;
    }
    reservedCompleted(value: JsonObject): BrowserReplacementReceipt {
        this.reservedContext();
        if (!this.originalObservations.has(value) || value.state!=="completed" || !this.publicReadyOffer)
            throw protocol("Original public video completion is unproved");
        return this.completedReceipt(value);
    }
    /** Same frozen private A/audio observation. Only an original signed
     * pre-cut read can permit replay; no decoder is observed again. */
    async reservedPrivateReady(evidence: WebRtcDecodedReadiness, options: RequestOptions, original?: JsonObject): Promise<void> {
        const context=this.reservedContext(),challenge=this.challenge;
        if (this.readyAttempted) {
            if (!original || !this.readyOffer) throw protocol("Original private readiness already attempted");
            this.checkReservedStatus(original,"negotiated-never-fed");
        } else {
            const streams=this.original!.streams_offered;
            if (!Array.isArray(streams)) throw protocol("Replacement has no frozen stream offer");
            const audio=streams.filter(s=>jsonObject(s)&&s.kind==="audio"&&s.direction==="sendrecv");
            const preview=streams.filter(s=>jsonObject(s)&&s.id===context.reservation.intent.preview_stream_id&&s.kind==="video"&&s.direction==="sendrecv");
            if (audio.length!==1 || !jsonObject(audio[0]) || evidence.audio.stream_id!==audio[0].id || !positive(evidence.audio.decoded_samples) ||
                preview.length!==1 || !evidence.video || evidence.video.stream_id!==context.reservation.intent.preview_stream_id ||
                !positive(evidence.video.decoded_frames) || evidence.video.decoder_startup!=="fresh-peer-first-decoded-frame" ||
                (evidence.video.decoded_keyframes!==null && (!positive(evidence.video.decoded_keyframes)||evidence.video.decoded_keyframes>evidence.video.decoded_frames)))
                throw protocol("Private readiness does not name original audio and A preview");
            this.readyOffer={...copy(this.original!),replacement_preparation:{profile:"connect-replacement-ready-v1",operation_record_id:this.root.recordId,
                readiness:{...challenge,audio:{...evidence.audio},video:{...evidence.video}}}};
            this.readyAttempted=true;
        }
        const frames=await this.client.requestReplacementContinuation("connection.offer",copy(this.readyOffer!),{...this.originalScope!},options);
        this.observe(replacementFrame(this.root,frames,this.root.targetConnectionId));
    }
    async reservedBridgeReady(marker:WebRtcBridgeReadyMarker,options:RequestOptions,original?:JsonObject):Promise<void> {
        const context=this.reservedContext();if(!this.bridgeOptIn||!this.readyAttempted||marker.profile!=="connect-replacement-public-video-bridge-ready-v1"||marker.operation_record_id!==this.root.recordId||marker.bridge_readiness.reservation_id!==context.reservation.intent.reservation_id)throw protocol("Original signed B bridge differs");
        const value=copy(marker as unknown as JsonObject);
        if(this.bridgeReadyOffer) {
            if(!original||!this.originalObservations.has(original)||original.state!=="preparing"||original.native_replacement_prepared!==true||bridgeCanonical(this.bridgeReadyOffer.replacement_preparation)!==bridgeCanonical(value))throw protocol("Original bridge readiness already attempted");
            const snapshot=bridgeClosed(original.webrtc_public_video_bridge,["opt_in","feeding","decoded_ready"]),feed=bridgeClosed(snapshot.feeding,["profile","phase","reservation_id","operation_record_id","operation_id","target_connection_id","target_generation","binding","bridge_readiness"]);
            if(feed.phase!=="feeding"||feed.operation_record_id!==this.root.recordId||feed.reservation_id!==marker.bridge_readiness.reservation_id)throw protocol("Bridge replay lacks original feeding");
        } else this.bridgeReadyOffer={...copy(this.original!),replacement_preparation:value};
        const frames=await this.client.requestReplacementContinuation("connection.offer",copy(this.bridgeReadyOffer),{...this.originalScope!},options);this.observe(replacementFrame(this.root,frames,this.root.targetConnectionId));
    }
    reservedReceiverWitness(value:JsonObject,final:boolean):unknown {
        this.reservedContext();if(!this.bridgeOptIn||!this.originalObservations.has(value))throw protocol("Receiver witness is not an original signed observation");
        const phase=value.webrtc_public_video_phase;
        if(!jsonObject(phase)||(final?(value.state!=="completed"||phase.phase!=="activated"):(!["preparing","ready","committing","unknown","interrupted"].includes(String(value.state))||!["cut-bound-post-f","decoded-ready"].includes(String(phase.phase)))))throw protocol("Receiver witness phase differs");
        const raw=final?value.receiver_video_release:value.receiver_video_post_cut;return raw;
    }
    /** Frozen B marker uses the original signed offer continuation, never a new
     * business operation, challenge, private readiness or media observation. */
    async reservedPublicReady(marker: WebRtcPublicVideoReadyMarker, options: RequestOptions, original?: JsonObject): Promise<void> {
        const context=this.reservedContext();
        const value:JsonObject={profile:marker.profile,operation_record_id:marker.operation_record_id,
            public_video_readiness:{reservation_id:marker.public_video_readiness.reservation_id,
                source_cut_id:marker.public_video_readiness.source_cut_id,nonce:marker.public_video_readiness.nonce,
                binding:{...marker.public_video_readiness.binding},decoded_frames:marker.public_video_readiness.decoded_frames,
                decoded_keyframes:marker.public_video_readiness.decoded_keyframes,decoder_startup:marker.public_video_readiness.decoder_startup}};
        if (!this.readyAttempted || marker.profile!=="connect-replacement-public-video-ready-v1" || marker.operation_record_id!==this.root.recordId ||
            marker.public_video_readiness.reservation_id!==context.reservation.intent.reservation_id)
            throw protocol("Original public video marker differs");
        if (this.publicReadyOffer) {
            if (!original || canonicalJson(this.publicReadyOffer.replacement_preparation)!==canonicalJson(value))
                throw protocol("Original public video readiness already attempted");
            this.checkReservedStatus(original,"cut-bound-post-f");
        } else this.publicReadyOffer={...copy(this.original!),replacement_preparation:value};
        const frames=await this.client.requestReplacementContinuation("connection.offer",copy(this.publicReadyOffer),{...this.originalScope!},options);
        this.observe(replacementFrame(this.root,frames,this.root.targetConnectionId));
    }
    /** Internal read-only recovery: never repeat decoded readiness or mutation. */
    async recoverCompletion(error: unknown, options: RequestOptions): Promise<BrowserReplacementReceipt | undefined> {
        if (this.receiveReservation)
            throw new ConnectError("unsupported-media-profile", "Reserved B recovery requires its original activated decoded lease");
        if (
            !this.mayHaveCommitted || !this.original || !this.selected ||
            !(recoverableReadinessFailure(error) ||
                (error instanceof ConnectError && error.code === "cancelled"))
        ) throw error;
        if (options.signal?.aborted) throw new ConnectError("cancelled", "Original replacement read cancelled");
        const value = await this.status(options);
        if (["failed", "cancelled"].includes(String(value.state))) {
            this.refused = true;
            throw new ConnectError("refused", "Original WebRTC replacement did not complete");
        }
        // Pending/unknown is NOT permission to render. The original target
        // keeps its private decoder for another read; no mutation is renewed.
        if (["accepted", "preparing", "ready", "committing", "unknown", "interrupted"].includes(String(value.state))) return undefined;
        if (value.state !== "completed") throw protocol("Invalid original WebRTC replacement state");
        return this.completedReceipt(value);
    }
    async complete(
        evidence: WebRtcDecodedReadiness,
        options: RequestOptions,
    ): Promise<BrowserReplacementReceipt> {
        if (this.receiveReservation)
            throw new ConnectError("unsupported-media-profile", "Reserved B cannot complete through private A readiness alone");
        const root = this.root,
            challenge = this.challenge;
        if (this.readyAttempted || !this.original || !this.originalScope)
            throw new ConnectError(
                "conflict",
                "Use the original decoder readiness continuation once",
            );
        const streams = this.original.streams_offered;
        if (!Array.isArray(streams))
            throw protocol("Replacement has no frozen stream offer");
        const audio = streams.filter(
            (s) =>
                jsonObject(s) &&
                s.kind === "audio" &&
                s.direction === "sendrecv",
        );
        const video = streams.filter(
            (s) =>
                jsonObject(s) &&
                s.kind === "video" &&
                s.direction === "recvonly",
        );
        if (
            audio.length !== 1 ||
            !jsonObject(audio[0]) ||
            evidence.audio.stream_id !== audio[0].id ||
            !positive(evidence.audio.decoded_samples) ||
            video.length > 1 ||
            (video.length === 0
                ? evidence.video !== null
                : !jsonObject(video[0]) ||
                  !evidence.video ||
                  evidence.video.stream_id !== video[0].id ||
                  !positive(evidence.video.decoded_frames) ||
                  evidence.video.decoder_startup !==
                      "fresh-peer-first-decoded-frame" ||
                  (evidence.video.decoded_keyframes !== null &&
                      (!positive(evidence.video.decoded_keyframes) ||
                          evidence.video.decoded_keyframes >
                              evidence.video.decoded_frames)))
        )
            throw protocol(
                "Decoded readiness does not name the frozen target receive streams",
            );
        const budget = new BrowserDeadline(options.timeoutMs ?? 10000);
        this.readyOffer = {
            ...copy(this.original),
            replacement_preparation: {
                profile: "connect-replacement-ready-v1",
                operation_record_id: root.recordId,
                readiness: {
                    ...challenge,
                    audio: { ...evidence.audio },
                    video: evidence.video ? { ...evidence.video } : null,
                },
            },
        };
        // The server captures this exact observation once. Recovery can replay
        // these frozen bytes only after a fresh original-root read still says
        // preparing; no new readiness observation or logical effect is created.
        this.readyAttempted = true;
        let attempts = 0,
            replayNeeded = false,
            nextReplayAt = 0;
        const ioOptions = () => ({
            ...options,
            // Leave time in the original budget for authoritative recovery.
            timeoutMs: Math.min(
                1000,
                Math.max(1, Math.floor(budget.remaining() / 2)),
            ),
        });
        const delay = () =>
            budget.run(
                () =>
                    new Promise<void>((resolve) =>
                        setTimeout(resolve, Math.min(20, budget.remaining())),
                    ),
                options.signal,
            );
        const sendReady = async () => {
            if (options.signal?.aborted)
                throw new ConnectError(
                    "cancelled",
                    "Original replacement wait cancelled",
                );
            attempts++;
            try {
                const frames = await this.client.requestReplacementContinuation(
                    "connection.offer",
                    copy(this.readyOffer!),
                    { ...this.originalScope! },
                    ioOptions(),
                );
                // Ready acknowledgements deliberately omit the final result.
                // Even a completed acknowledgement requires operation.get.
                this.observe(
                    replacementFrame(root, frames, root.targetConnectionId),
                );
                replayNeeded = false;
            } catch (error) {
                if (
                    options.signal?.aborted ||
                    this.client.closed ||
                    !recoverableReadinessFailure(error)
                )
                    throw error;
                replayNeeded = true;
                nextReplayAt =
                    performance.now() +
                    Math.min(1000, 50 * 2 ** (attempts - 1));
            }
        };
        await sendReady();
        for (;;) {
            let value: JsonObject;
            try {
                value = await this.status(ioOptions());
            } catch (error) {
                if (
                    options.signal?.aborted ||
                    this.client.closed ||
                    !recoverableReadinessFailure(error)
                )
                    throw error;
                await delay();
                continue;
            }
            if (value.state === "completed") {
                return this.completedReceipt(value);
            }
            if (["failed", "cancelled"].includes(String(value.state))) {
                this.refused = true;
                throw new ConnectError(
                    "refused",
                    "Original WebRTC replacement did not complete",
                );
            }
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
                throw protocol("Invalid original WebRTC replacement state");
            if (
                replayNeeded &&
                attempts < 8 &&
                performance.now() >= nextReplayAt &&
                value.state === "preparing" &&
                value.native_replacement_prepared === true
            ) {
                await sendReady();
            } else {
                await delay();
            }
        }
    }
}
