import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
const fields = ["profile", "phase", "reservation_id", "nonce", "preview_stream_id", "preview_role", "public_stream_id"];
const parsed = new WeakMap();
const invalid = () => new ConnectError("protocol", "Invalid original WebRTC receive reservation intent");
const sameScope = (a, b) => a.recordId === b.recordId && a.operationId === b.operationId &&
    a.targetConnectionId === b.targetConnectionId && a.audioStreamId === b.audioStreamId &&
    a.previewStreamId === b.previewStreamId && a.publicStreamId === b.publicStreamId;
/** Internal provenance check for accidental structural copies/foreign Roots.
 * It is not a signature verifier or a public-output permission check. */
export function originalWebRtcReceiveIntent(value, scope) {
    if (value === null || typeof value !== "object")
        return false;
    const original = parsed.get(value);
    return original !== undefined && sameScope(original, scope);
}
/** One original operation's immutable pre-offer intent. Future negotiated/cut
 * phases need different validators and cannot be smuggled through this one. */
export class WebRtcReceiveReservationIntentOwner {
    scope;
    selected;
    closed = false;
    physical;
    constructor(scope) {
        const captured = { recordId: scope.recordId, operationId: scope.operationId,
            targetConnectionId: scope.targetConnectionId, audioStreamId: scope.audioStreamId,
            previewStreamId: scope.previewStreamId, publicStreamId: scope.publicStreamId };
        if (!Object.values(captured).every(identifier) ||
            new Set([captured.audioStreamId, captured.previewStreamId, captured.publicStreamId]).size !== 3)
            throw invalid();
        this.scope = Object.freeze(captured);
    }
    observe(value, original) {
        if (this.closed)
            throw new ConnectError("closed", "Original WebRTC receive reservation retired");
        if (original.recordId !== this.scope.recordId || original.operationId !== this.scope.operationId ||
            original.targetConnectionId !== this.scope.targetConnectionId)
            throw invalid();
        if (value === undefined) {
            if (this.selected)
                throw invalid();
            return undefined;
        }
        if (value === null || typeof value !== "object" || Array.isArray(value))
            throw invalid();
        const keys = Reflect.ownKeys(value);
        if (keys.length !== fields.length || keys.some(key => typeof key !== "string" || !fields.some(field => field === key)))
            throw invalid();
        const descriptors = Object.getOwnPropertyDescriptors(value);
        if (fields.some(field => !descriptors[field] || !Object.hasOwn(descriptors[field], "value")))
            throw invalid();
        // Capture each scalar once; later getters or caller mutations cannot
        // substitute a different ID/phase after its validation.
        const read = (field) => descriptors[field].value;
        const profile = read("profile"), phase = read("phase"), reservation = read("reservation_id"), nonce = read("nonce"), preview = read("preview_stream_id"), role = read("preview_role"), publicId = read("public_stream_id");
        if (profile !== "connect/webrtc-public-video-reservation/1" || phase !== "intent" || role !== "receive" ||
            !identifier(reservation) || !identifier(nonce) || !identifier(preview) || !identifier(publicId) ||
            preview !== this.scope.previewStreamId || publicId !== this.scope.publicStreamId)
            throw invalid();
        if (this.selected) {
            if (fields.some(field => descriptors[field].value !== this.selected[field]))
                throw invalid();
            return this.selected;
        }
        const intent = Object.freeze({ profile, phase,
            reservation_id: reservation, nonce, preview_stream_id: preview, preview_role: role, public_stream_id: publicId });
        parsed.set(intent, this.scope);
        this.selected = intent;
        return intent;
    }
    owns(intent, scope) {
        return !this.closed && this.selected === intent &&
            sameScope(this.scope, scope) && originalWebRtcReceiveIntent(intent, scope);
    }
    claimPhysical(peer, intent, scope) {
        if (!this.owns(intent, scope) || (this.physical && this.physical.deref() !== peer))
            throw invalid();
        this.physical ??= new WeakRef(peer);
    }
    ownsPhysical(peer, intent, scope) {
        return this.owns(intent, scope) && this.physical?.deref() === peer;
    }
    close() {
        this.closed = true;
        if (this.selected)
            parsed.delete(this.selected);
        this.selected = undefined;
        this.physical = undefined;
    }
}
