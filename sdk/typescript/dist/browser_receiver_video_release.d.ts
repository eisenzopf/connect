import { type NativeSequenceCut, type NativeDecodedObservation } from "./browser_native_activation.js";
export interface ReceiverVideoReleaseScope {
    readonly operation_record_id: string;
    readonly operation_id: string;
    readonly reservation_id: string;
    readonly source_connection_id: string;
    readonly target_connection_id: string;
    readonly stream_id: string;
    readonly stream_generation: number;
    readonly connection_generation: number;
    /** Public Root generation, independent of the physical writer generation. */
    readonly target_generation: number;
    readonly source_cut_id: string;
}
export interface ReceiverOriginalFrame {
    readonly realm: string;
    readonly ordinal: string;
    readonly last_old_ordinal: string;
}
export interface ReceiverRtpReleaseFloor {
    readonly carrier: "webrtc-rtp";
    readonly physical_writer_incarnation: string;
    readonly primary_ssrc: number;
    readonly first_public_rtp_timestamp: number;
}
export interface ReceiverNativeReleaseFloor {
    readonly carrier: "native-datagram";
    /** Reuse the EXISTING exact native cut parser and membership convention. */
    readonly cut: NativeSequenceCut;
}
export interface ReceiverVideoRelease extends ReceiverVideoReleaseScope {
    readonly profile: "connect-receiver-video-release-v1";
    readonly original_frame: ReceiverOriginalFrame;
    readonly floor: ReceiverRtpReleaseFloor | ReceiverNativeReleaseFloor;
}
export interface OriginalReceiverReleaseObservation {
    readonly operationRecordId: string;
    readonly operationId: string;
    readonly targetConnectionId: string;
    readonly releaseAuthorized: true;
}
export type ExactReceiverIdentity = Readonly<{
    carrier: "webrtc-rtp";
    physical_writer_incarnation: string;
    primary_ssrc: number;
}> | Readonly<{
    carrier: "native-datagram";
    physical_incarnation: string;
    stream_local_id: number;
}>;
export interface ReceiverRtpOutput {
    readonly rtpTimestamp: number;
    readonly presentedFrames: number;
}
/** One owner for the actual warmed receiver/decoder and original operation.
 * ownsPhysical MUST perform only existing local identity/state checks, no SQL,
 * registry admission, network IO, or locks. State-change retirement closes owner.
 * This local capability cannot substitute for cross-device old-output detach ACK.
 */
export declare class ReceiverVideoReleaseOwner {
    private readonly ownsPhysical;
    private readonly scope;
    private release?;
    private rawIdentity?;
    private token?;
    private lastRtcPresented;
    private lastRtcTimestamp?;
    private lastNativeSequence;
    private stopped;
    private readonly identity;
    constructor(scopeValue: ReceiverVideoReleaseScope, identityValue: ExactReceiverIdentity, ownsPhysical: () => boolean);
    private check;
    /** Call ONLY after verified original release authorization and actual old
     * output-detach acknowledgment; never from a caller supplied media option.
     * No raw PCM/image is retagged; only future matching original outputs pass.
     */
    observe(value: unknown, original: OriginalReceiverReleaseObservation): Readonly<ReceiverVideoRelease>;
    borrow(): ReceiverVideoReleaseLoan;
    close(): void;
}
/** One renderer may borrow a warmed decoder's public output at a time. Test
 * eligibility both on bounded enqueue AND immediately before actual drawing.
 * Never replace the renderer's existing duplicate/order/queue/privacy checks.
 */
export interface ReceiverVideoReleaseLoan {
    eligibleRtp(output: ReceiverRtpOutput): boolean;
    eligibleNative(output: NativeDecodedObservation): boolean;
    /** Record only after the synchronous actual draw succeeds. High water is
     * owner-scoped and survives renderer reborrowing; no historical replay. */
    presentedRtp(output: ReceiverRtpOutput): void;
    presentedNative(output: NativeDecodedObservation): void;
    release(): void;
}
