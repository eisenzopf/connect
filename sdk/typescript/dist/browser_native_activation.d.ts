import { type JsonObject } from "./uctp_json.js";
import type { BrowserReplacementRoot } from "./browser_replacement.js";
export interface NativeEgressIdentity {
    readonly profile: "native-sequence-cursor/1";
    readonly physical_incarnation: string;
    readonly stream_generation: number;
    readonly connection_generation: number;
    readonly initial_outer_sequence: 0;
}
export interface NativeSequenceCut {
    readonly stream_id: string;
    readonly stream_local_id: number;
    readonly stream_generation: number;
    readonly physical_incarnation: string;
    readonly sequence_space: "native-outer-u32";
    readonly public_sequence_floor: number;
    readonly cursor_ordinal: number;
    readonly submission_observed: false;
}
export interface NativeDecodedObservation {
    readonly profile: "connect/native-decoded-sequence/1";
    readonly identity: NativeEgressIdentity;
    readonly datagramSequences: readonly number[];
    readonly ssrc: number;
    readonly rtpTimestamp: number;
    readonly videoEpoch?: string;
    readonly sourceNtp?: string;
    readonly sourceClock?: Readonly<{
        ntp: string;
        rtpTimestamp: number;
    }>;
}
export interface NativeActivationBoundary {
    readonly profile: "connect/native-public-sequence-cut/1";
    readonly operation_record_id: string;
    readonly operation_id: string;
    readonly source_connection_id: string;
    readonly target_connection_id: string;
    readonly target_generation: number;
    readonly stream_revision: number;
    readonly audience: Readonly<{
        scope: "public";
        streams: readonly NativeSequenceCut[];
    }> | Readonly<{
        scope: "private-consultation";
        original_operation: string;
    }>;
}
export declare function nativeEgressIdentity(v: unknown): NativeEgressIdentity;
export declare function nativeCut(v: unknown): NativeSequenceCut;
export interface NegotiatedNativeReceiver {
    readonly streamId: string;
    readonly handle: number;
    readonly identity: NativeEgressIdentity;
}
/** Called only with this original client's verified completed operation record. */
export declare function nativeActivation(root: BrowserReplacementRoot, status: JsonObject, receivers: readonly NegotiatedNativeReceiver[]): NativeActivationBoundary;
/** Original fresh cursor begins at zero. A half-range or wrap is unclassifiable,
 * never silently rebased. Every fragment of one decoded frame must agree. */
export declare function nativeMembership(cut: NativeSequenceCut, observation: NativeDecodedObservation): "private" | "public";
export declare function sameNativeIdentity(a: NativeEgressIdentity | undefined, b: NativeEgressIdentity | undefined): boolean;
