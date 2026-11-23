import { bridgeCanonical, parseWebRtcBridgeOptIn, parseWebRtcBridgeSnapshot } from "./browser_webrtc_public_video_bridge.js";
import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
import { replacementRoot } from "./browser_replacement.js";
const base = ["profile", "phase", "reservation_id", "operation_record_id", "operation_id", "target_connection_id", "target_generation", "binding"];
const bindingFields = ["stream_id", "preview_stream_id", "mid", "stream_generation", "physical_writer_incarnation", "connection_generation", "primary_ssrc", "payload_type", "clock_rate_hz", "codec"];
const rank = { "negotiated-never-fed": 0, "cut-bound-post-f": 1, "decoded-ready": 2, "activated": 3, "discarded": 4 };
const discardReasons = ["cancelled", "expired", "refused", "owner-retired", "binding-changed", "cut-unavailable"];
const invalid = () => new ConnectError("protocol", "Original public video phase changed or is unavailable");
const uint = (v, max = Number.MAX_SAFE_INTEGER) => Number.isSafeInteger(v) && Number(v) >= 0 && Number(v) <= max;
const positive = (v) => uint(v) && v > 0;
const opaque = (v) => typeof v === "string" && /^[\x21-\x7e]{1,256}$/.test(v);
function closed(value, fields) {
    if (value === null || typeof value !== "object" || Array.isArray(value))
        throw invalid();
    const keys = Reflect.ownKeys(value), descriptors = Object.getOwnPropertyDescriptors(value);
    if (keys.length !== fields.length || keys.some(key => typeof key !== "string" || !fields.includes(key)) || fields.some(key => !descriptors[key] || !Object.hasOwn(descriptors[key], "value")))
        throw invalid();
    return Object.fromEntries(fields.map(key => [key, descriptors[key].value]));
}
const equalBinding = (a, b) => bindingFields.every(key => a[key] === b[key]);
const parsed = new WeakMap();
const originalOwners = new WeakSet();
/** Metadata validator only. observe() requires the SAME original signed operation
 * observation. Parsing never supplies a grant, source cut, readiness, or IO. */
export class WebRtcPublicVideoPhaseOwner {
    reservation;
    targetGeneration;
    privateReadinessNonce;
    bridgeOptIn;
    root;
    scope;
    selected;
    bridgeSnapshot;
    bridgeMarker;
    binding;
    cut;
    discardedCut;
    stopped = false;
    retired = new Set();
    constructor(root, scope, reservation, targetGeneration, privateReadinessNonce, bridgeOptIn) {
        this.reservation = reservation;
        this.targetGeneration = targetGeneration;
        this.privateReadinessNonce = privateReadinessNonce;
        this.bridgeOptIn = bridgeOptIn;
        this.root = replacementRoot(root);
        if (bridgeOptIn)
            parseWebRtcBridgeOptIn(bridgeOptIn);
        this.scope = Object.freeze({ ...scope });
        if (!positive(targetGeneration) || !identifier(privateReadinessNonce) ||
            !reservation.owner.owns(reservation.intent, scope) ||
            scope.recordId !== root.recordId || scope.operationId !== root.operationId || scope.targetConnectionId !== root.targetConnectionId)
            throw invalid();
        if (originalOwners.has(reservation.owner))
            throw new ConnectError("conflict", "Original public video phase owner already exists");
        // A dead/closed parser cannot be replaced to obtain another observation
        // budget or decoded lease for this same original physical reservation.
        originalOwners.add(reservation.owner);
    }
    observe(value, original) {
        this.check();
        if (original.recordId !== this.root.recordId || original.operationId !== this.root.operationId || original.targetConnectionId !== this.root.targetConnectionId || original.targetGeneration !== this.targetGeneration || typeof original.rootCompleted !== "boolean")
            throw invalid();
        if (value === null || typeof value !== "object")
            throw invalid();
        const phaseDescriptor = Object.getOwnPropertyDescriptor(value, "phase");
        if (!phaseDescriptor || !Object.hasOwn(phaseDescriptor, "value") || typeof phaseDescriptor.value !== "string" || !Object.hasOwn(rank, phaseDescriptor.value))
            throw invalid();
        const phase = phaseDescriptor.value;
        const withCut = phase !== "negotiated-never-fed" && phase !== "discarded";
        const v = closed(value, withCut ? [...base, "source_cut_id", "public_readiness"] : phase === "discarded" ? [...base, "source_cut_id", "reason"] : base);
        if (v.profile !== "connect/webrtc-public-video-phase/1" || v.reservation_id !== this.reservation.intent.reservation_id || v.operation_record_id !== this.root.recordId || v.operation_id !== this.root.operationId || v.target_connection_id !== this.root.targetConnectionId || v.target_generation !== this.targetGeneration || (phase === "activated" && !original.rootCompleted))
            throw invalid();
        const b = closed(v.binding, bindingFields);
        if (b.stream_id !== this.scope.publicStreamId || b.preview_stream_id !== this.scope.previewStreamId || typeof b.mid !== "string" || !/^[\x21-\x7e]{1,256}$/.test(b.mid) || !positive(b.stream_generation) || !opaque(b.physical_writer_incarnation) || !positive(b.connection_generation) || !uint(b.primary_ssrc, 0xffffffff) || !uint(b.payload_type, 127) || b.clock_rate_hz !== 90000 || b.codec !== "vp8")
            throw invalid();
        const binding = Object.freeze(b);
        if (this.binding && !equalBinding(this.binding, binding))
            throw invalid();
        let publicReadiness;
        if (withCut) {
            const challenge = closed(v.public_readiness, ["nonce", "decoder_startup"]);
            if (!opaque(v.source_cut_id) || !identifier(challenge.nonce) || challenge.nonce === this.privateReadinessNonce || challenge.nonce === this.reservation.intent.nonce || challenge.decoder_startup !== "fresh-receiver-first-decoded-frame")
                throw invalid();
            if (this.cut && (this.cut.id !== v.source_cut_id || this.cut.nonce !== challenge.nonce))
                throw invalid();
            if (this.bridgeSnapshot?.feeding?.bridge_readiness.nonce === challenge.nonce)
                throw invalid();
            publicReadiness = Object.freeze({ nonce: challenge.nonce, decoder_startup: "fresh-receiver-first-decoded-frame" });
        }
        if (phase === "discarded") {
            if ((v.source_cut_id !== null && !opaque(v.source_cut_id)) ||
                typeof v.reason !== "string" || !discardReasons.some(reason => reason === v.reason) ||
                (this.cut && v.source_cut_id !== this.cut.id) ||
                (this.discardedCut !== undefined && v.source_cut_id !== this.discardedCut) ||
                (this.selected?.phase === "discarded" && this.selected.reason !== v.reason))
                throw invalid();
        }
        if (this.selected && (rank[phase] < rank[this.selected.phase] || (this.selected.phase === "discarded" && phase !== "discarded")))
            throw invalid();
        const result = Object.freeze({ profile: "connect/webrtc-public-video-phase/1", phase,
            reservation_id: this.reservation.intent.reservation_id, operation_record_id: this.root.recordId,
            operation_id: this.root.operationId, target_connection_id: this.root.targetConnectionId,
            target_generation: this.targetGeneration, binding,
            ...(withCut ? { source_cut_id: v.source_cut_id, public_readiness: publicReadiness } : {}),
            ...(phase === "discarded" ? { source_cut_id: v.source_cut_id, reason: v.reason } : {}),
        });
        this.binding ??= binding;
        if (withCut)
            this.cut ??= { id: result.source_cut_id, nonce: result.public_readiness.nonce };
        this.selected = result;
        if (phase === "discarded")
            this.discardedCut = result.source_cut_id;
        parsed.set(result, this);
        if (phase === "discarded")
            this.retire();
        return result;
    }
    bridgeEnabled() { this.check(); return this.bridgeOptIn !== undefined; }
    negotiated() {
        this.check();
        if (!this.bridgeOptIn || this.selected?.phase !== "negotiated-never-fed")
            throw invalid();
        return this.selected;
    }
    /** Prearmed hidden output remains bound to the original physical B while
     * cut/ready progress. This never permits take or public presentation. */
    acceptsPreCutBinding(value) {
        return !!this.bridgeOptIn && !this.stopped && parsed.get(value) === this && !!this.selected && this.selected.phase !== "discarded" && equalBinding(value.binding, this.selected.binding) && this.reservation.owner.owns(this.reservation.intent, this.scope);
    }
    observeBridge(value) {
        this.check();
        if (!this.bridgeOptIn || !this.binding)
            throw invalid();
        const next = parseWebRtcBridgeSnapshot(value, this.root, this.reservation.intent.reservation_id, this.binding, this.privateReadinessNonce, this.reservation.intent.nonce, this.targetGeneration);
        if (bridgeCanonical(next.opt_in) !== bridgeCanonical(this.bridgeOptIn) || next.feeding?.target_generation !== undefined && next.feeding.target_generation !== this.targetGeneration)
            throw invalid();
        if (this.bridgeSnapshot?.feeding && (!next.feeding || bridgeCanonical(this.bridgeSnapshot.feeding) !== bridgeCanonical(next.feeding)))
            throw invalid();
        if (this.bridgeSnapshot?.decoded_ready && (!next.decoded_ready || bridgeCanonical(this.bridgeSnapshot.decoded_ready) !== bridgeCanonical(next.decoded_ready)))
            throw invalid();
        if (next.feeding && this.cut?.nonce === next.feeding.bridge_readiness.nonce)
            throw invalid();
        if (next.decoded_ready && (!this.bridgeMarker || bridgeCanonical(next.decoded_ready) !== bridgeCanonical(this.bridgeMarker)))
            throw invalid();
        this.bridgeSnapshot = Object.freeze({ ...next, feeding: this.bridgeSnapshot?.feeding ?? next.feeding, decoded_ready: this.bridgeSnapshot?.decoded_ready ?? next.decoded_ready });
        return this.bridgeSnapshot;
    }
    bridgeFeeding() {
        this.check();
        if (!this.bridgeOptIn || !this.bridgeSnapshot?.feeding)
            throw invalid();
        return this.bridgeSnapshot.feeding;
    }
    acceptsBridgeFeeding(value) {
        return !!this.bridgeSnapshot?.feeding && value === this.bridgeSnapshot.feeding && this.acceptsPreCutBinding(this.selected);
    }
    expectBridgeMarker(marker) {
        this.check();
        if (this.bridgeMarker && bridgeCanonical(this.bridgeMarker) !== bridgeCanonical(marker))
            throw invalid();
        this.bridgeMarker = marker;
    }
    check() {
        if (this.stopped || !this.reservation.owner.owns(this.reservation.intent, this.scope))
            throw new ConnectError("closed", "Original public video phase owner retired");
    }
    exactLocalPeer(peer) {
        return !this.stopped && this.reservation.owner.ownsPhysical(peer, this.reservation.intent, this.scope);
    }
    accepts(value) {
        return !this.stopped && this.reservation.owner.owns(this.reservation.intent, this.scope) && parsed.get(value) === this && !!this.selected &&
            this.selected.phase !== "discarded" && !!value.source_cut_id && !!value.public_readiness &&
            this.selected.source_cut_id === value.source_cut_id &&
            this.selected.public_readiness?.nonce === value.public_readiness.nonce &&
            equalBinding(this.selected.binding, value.binding);
    }
    cutBound() {
        this.check();
        if (!this.selected || this.selected.phase !== "cut-bound-post-f")
            throw invalid();
        return this.selected;
    }
    activated(value) {
        return this.accepts(value) && value.phase === "activated" && this.selected?.phase === "activated";
    }
    onRetire(fn) {
        this.check();
        if (this.retired.size >= 1)
            throw invalid();
        this.retired.add(fn);
        return () => this.retired.delete(fn);
    }
    retire() { for (const fn of [...this.retired])
        fn(); this.retired.clear(); }
    close() { if (this.stopped)
        return; this.stopped = true; this.retire(); this.selected = undefined; }
}
