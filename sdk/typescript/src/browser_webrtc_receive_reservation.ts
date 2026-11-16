import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";

/** Setup metadata only. Authentication of the enclosing original operation is
 * the caller's prerequisite; parsing this object grants no media or readiness. */
export interface WebRtcReceiveReservationIntent {
    readonly profile: "connect/webrtc-public-video-reservation/1";
    readonly phase: "intent";
    readonly reservation_id: string;
    readonly nonce: string;
    readonly preview_stream_id: string;
    readonly preview_role: "receive";
    readonly public_stream_id: string;
}
export interface WebRtcReceiveReservationScope {
    readonly recordId: string;
    readonly operationId: string;
    readonly targetConnectionId: string;
    readonly audioStreamId: string;
    readonly previewStreamId: string;
    readonly publicStreamId: string;
}
/** Internal transfer from the original verified preparing snapshot. Not an
 * application media option or a public activation/readiness certificate. */
export interface OriginalWebRtcReceiveReservation {
    readonly owner: WebRtcReceiveReservationIntentOwner;
    readonly intent: WebRtcReceiveReservationIntent;
}
type Original = Pick<WebRtcReceiveReservationScope, "recordId" | "operationId" | "targetConnectionId">;
const fields = ["profile", "phase", "reservation_id", "nonce", "preview_stream_id", "preview_role", "public_stream_id"] as const;
const parsed = new WeakMap<object, Readonly<WebRtcReceiveReservationScope>>();
const invalid = () => new ConnectError("protocol", "Invalid original WebRTC receive reservation intent");
const sameScope = (a: WebRtcReceiveReservationScope, b: WebRtcReceiveReservationScope): boolean =>
    a.recordId === b.recordId && a.operationId === b.operationId &&
    a.targetConnectionId === b.targetConnectionId && a.audioStreamId === b.audioStreamId &&
    a.previewStreamId === b.previewStreamId && a.publicStreamId === b.publicStreamId;

/** Internal provenance check for accidental structural copies/foreign Roots.
 * It is not a signature verifier or a public-output permission check. */
export function originalWebRtcReceiveIntent(value: unknown, scope: WebRtcReceiveReservationScope): value is WebRtcReceiveReservationIntent {
    if (value === null || typeof value !== "object") return false;
    const original = parsed.get(value);
    return original !== undefined && sameScope(original, scope);
}

/** One original operation's immutable pre-offer intent. Future negotiated/cut
 * phases need different validators and cannot be smuggled through this one. */
export class WebRtcReceiveReservationIntentOwner {
    private readonly scope: Readonly<WebRtcReceiveReservationScope>;
    private selected?: WebRtcReceiveReservationIntent;
    private closed = false;
    private physical?: WeakRef<object>;
    constructor(scope: WebRtcReceiveReservationScope) {
        const captured = {recordId: scope.recordId, operationId: scope.operationId,
            targetConnectionId: scope.targetConnectionId, audioStreamId: scope.audioStreamId,
            previewStreamId: scope.previewStreamId, publicStreamId: scope.publicStreamId};
        if (!Object.values(captured).every(identifier) ||
            new Set([captured.audioStreamId, captured.previewStreamId, captured.publicStreamId]).size !== 3) throw invalid();
        this.scope = Object.freeze(captured);
    }
    observe(value: unknown, original: Original): WebRtcReceiveReservationIntent | undefined {
        if (this.closed) throw new ConnectError("closed", "Original WebRTC receive reservation retired");
        if (original.recordId !== this.scope.recordId || original.operationId !== this.scope.operationId ||
            original.targetConnectionId !== this.scope.targetConnectionId) throw invalid();
        if (value === undefined) {
            if (this.selected) throw invalid();
            return undefined;
        }
        if (value === null || typeof value !== "object" || Array.isArray(value)) throw invalid();
        const keys = Reflect.ownKeys(value);
        if (keys.length !== fields.length || keys.some(key => typeof key !== "string" || !fields.some(field => field === key))) throw invalid();
        const descriptors = Object.getOwnPropertyDescriptors(value);
        if (fields.some(field => !descriptors[field] || !Object.hasOwn(descriptors[field], "value"))) throw invalid();
        // Capture each scalar once; later getters or caller mutations cannot
        // substitute a different ID/phase after its validation.
        const read = (field: typeof fields[number]): unknown => descriptors[field].value;
        const profile = read("profile"), phase = read("phase"), reservation = read("reservation_id"), nonce = read("nonce"),
            preview = read("preview_stream_id"), role = read("preview_role"), publicId = read("public_stream_id");
        if (profile !== "connect/webrtc-public-video-reservation/1" || phase !== "intent" || role !== "receive" ||
            !identifier(reservation) || !identifier(nonce) || !identifier(preview) || !identifier(publicId) ||
            preview !== this.scope.previewStreamId || publicId !== this.scope.publicStreamId) throw invalid();
        if (this.selected) {
            if (fields.some(field => descriptors[field].value !== this.selected![field])) throw invalid();
            return this.selected;
        }
        const intent: WebRtcReceiveReservationIntent = Object.freeze({profile, phase,
            reservation_id: reservation, nonce, preview_stream_id: preview, preview_role: role, public_stream_id: publicId});
        parsed.set(intent, this.scope);
        this.selected = intent;
        return intent;
    }
    owns(intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): boolean {
        return !this.closed && this.selected === intent &&
            sameScope(this.scope, scope) && originalWebRtcReceiveIntent(intent, scope);
    }
    claimPhysical(peer: object, intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): void {
        if (!this.owns(intent, scope) || (this.physical && this.physical.deref() !== peer)) throw invalid();
        this.physical ??= new WeakRef(peer);
    }
    ownsPhysical(peer: object, intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): boolean {
        return this.owns(intent, scope) && this.physical?.deref() === peer;
    }
    close(): void {
        this.closed = true;
        if (this.selected) parsed.delete(this.selected);
        this.selected = undefined;
        this.physical = undefined;
    }
}
