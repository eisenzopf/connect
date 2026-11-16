import { type WebRtcReceiveReservationIntent, type WebRtcReceiveReservationScope } from "./browser_webrtc_receive_reservation.js";
type Role = "audio" | "preview" | "public";
export interface WebRtcReceiveRoleBinding {
    readonly role: Role;
    readonly streamId: string;
    readonly mid: string;
    readonly transceiver: RTCRtpTransceiver;
    readonly receiver: RTCRtpReceiver;
    readonly track: MediaStreamTrack;
}
/** Optional internal metadata registry. No native operations/listeners/tasks are
 * created here. In particular a binding is NOT decoded-ready or public credit. */
export declare class WebRtcReceiveRoles {
    private peer?;
    private intent?;
    private scope?;
    private readonly slots;
    constructor(peer: RTCPeerConnection, scope: WebRtcReceiveReservationScope, intent: WebRtcReceiveReservationIntent, transceivers: {
        audio: RTCRtpTransceiver;
        preview: RTCRtpTransceiver;
        public: RTCRtpTransceiver;
    });
    private check;
    accept(event: Pick<RTCTrackEvent, "transceiver" | "receiver" | "track">): Role;
    binding(role: Role): Readonly<WebRtcReceiveRoleBinding> | undefined;
    /** Exact original three-m-section answer and native pointer/MID association.
     * This is negotiation only: no packet, decode, source cut or activation. */
    validateAnswer(sdp: string): void;
    close(): void;
}
/** Absence preserves the default without inspecting or changing a native peer.
 * The future constructor must itself create B before its initial offer only
 * after the original authenticated preparing snapshot supplied this intent. */
export declare function receiveRolesForOriginalIntent(peer: RTCPeerConnection, scope: WebRtcReceiveReservationScope, intent: WebRtcReceiveReservationIntent | undefined, transceivers: {
    audio: RTCRtpTransceiver;
    preview: RTCRtpTransceiver;
    public: RTCRtpTransceiver;
}): WebRtcReceiveRoles | undefined;
export {};
