/** Historical original native packet membership. No permission or IO grant. */
import { ConnectError } from "./errors.js";
import { jsonObject, canonicalJson, type JsonObject } from "./uctp_json.js";
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
    readonly sourceClock?: Readonly<{ ntp: string; rtpTimestamp: number }>;
}
export interface NativeActivationBoundary {
    readonly profile: "connect/native-public-sequence-cut/1";
    readonly operation_record_id: string;
    readonly operation_id: string;
    readonly source_connection_id: string;
    readonly target_connection_id: string;
    readonly target_generation: number;
    readonly stream_revision: number;
    readonly audience:
        | Readonly<{ scope: "public"; streams: readonly NativeSequenceCut[] }>
        | Readonly<{
              scope: "private-consultation";
              original_operation: string;
          }>;
}
const uint = (v: unknown, max = Number.MAX_SAFE_INTEGER): v is number =>
    Number.isSafeInteger(v) && Number(v) >= 0 && Number(v) <= max;
const positive = (v: unknown): v is number => uint(v) && v > 0;
function bad(): never {
    throw new ConnectError(
        "protocol",
        "Invalid original native activation identity",
    );
}
function keys(v: unknown, k: readonly string[]): asserts v is JsonObject {
    if (
        v === null ||
        typeof v !== "object" ||
        Array.isArray(v) ||
        Object.keys(v).sort().join("\n") !== [...k].sort().join("\n")
    )
        bad();
}
const incarnation = (v: unknown): v is string =>
    typeof v === "string" && /^[0-9a-f]{32}$/.test(v) && !/^0+$/.test(v);
const id = (v: unknown): v is string =>
    typeof v === "string" &&
    v.length > 0 &&
    new TextEncoder().encode(v).byteLength <= 256 &&
    !/[\p{Cc}]/u.test(v);
export function nativeEgressIdentity(v: unknown): NativeEgressIdentity {
    keys(v, [
        "profile",
        "physical_incarnation",
        "stream_generation",
        "connection_generation",
        "initial_outer_sequence",
    ]);
    if (
        v.profile !== "native-sequence-cursor/1" ||
        !incarnation(v.physical_incarnation) ||
        !positive(v.stream_generation) ||
        !positive(v.connection_generation) ||
        v.initial_outer_sequence !== 0
    )
        bad();
    return Object.freeze({ ...v }) as unknown as NativeEgressIdentity;
}
export function nativeCut(v: unknown): NativeSequenceCut {
    keys(v, [
        "stream_id",
        "stream_local_id",
        "stream_generation",
        "physical_incarnation",
        "sequence_space",
        "public_sequence_floor",
        "cursor_ordinal",
        "submission_observed",
    ]);
    if (
        !id(v.stream_id) ||
        !uint(v.stream_local_id, 65535) ||
        v.stream_local_id === 0 ||
        !positive(v.stream_generation) ||
        !incarnation(v.physical_incarnation) ||
        v.sequence_space !== "native-outer-u32" ||
        !uint(v.cursor_ordinal, 0x7fffffff) ||
        v.public_sequence_floor !== v.cursor_ordinal ||
        v.submission_observed !== false
    )
        bad();
    return Object.freeze({ ...v }) as unknown as NativeSequenceCut;
}
export interface NegotiatedNativeReceiver {
    readonly streamId: string;
    readonly handle: number;
    readonly identity: NativeEgressIdentity;
}
/** Called only with this original client's verified completed operation record. */
export function nativeActivation(
    root: BrowserReplacementRoot,
    status: JsonObject,
    receivers: readonly NegotiatedNativeReceiver[],
): NativeActivationBoundary {
    const v = jsonObject(status.result)
        ? status.result.native_activation
        : undefined;
    keys(v, [
        "profile",
        "operation_record_id",
        "operation_id",
        "source_connection_id",
        "target_connection_id",
        "target_generation",
        "stream_revision",
        "audience",
    ]);
    if (
        ![
            v.operation_record_id,
            v.operation_id,
            v.source_connection_id,
            v.target_connection_id,
        ].every(id) ||
        v.source_connection_id === v.target_connection_id ||
        status.state !== "completed" ||
        status.record_id !== root.recordId ||
        status.operation_id !== root.operationId ||
        v.profile !== "connect/native-public-sequence-cut/1" ||
        v.operation_record_id !== root.recordId ||
        v.operation_id !== root.operationId ||
        v.source_connection_id !== root.sourceConnectionId ||
        v.target_connection_id !== root.targetConnectionId ||
        !positive(v.target_generation) ||
        !positive(v.stream_revision) ||
        v.stream_revision <= 2 ||
        !jsonObject(status.result) ||
        status.result.activation_state !== "ready" ||
        status.result.connection_id !== v.target_connection_id ||
        status.result.replaces !== v.source_connection_id ||
        status.result.generation !== v.target_generation ||
        !jsonObject(status.result.readiness) ||
        status.result.readiness.stream_revision !== v.stream_revision ||
        !jsonObject(v.audience)
    )
        bad();
    let audience: NativeActivationBoundary["audience"];
    if (v.audience.scope === "private-consultation") {
        keys(v.audience, ["scope", "original_operation"]);
        if (!id(v.audience.original_operation)) bad();
        audience = Object.freeze({
            scope: "private-consultation",
            original_operation: v.audience.original_operation,
        });
    } else {
        keys(v.audience, ["scope", "streams"]);
        if (
            v.audience.scope !== "public" ||
            !Array.isArray(v.audience.streams) ||
            v.audience.streams.length !== receivers.length ||
            receivers.length < 1 ||
            receivers.length > 32
        )
            bad();
        const streams = v.audience.streams.map(nativeCut),
            ids = new Set<string>(),
            handles = new Set<number>(),
            physical = new Set<string>();
        for (const cut of streams) {
            const r = receivers.find((r) => r.streamId === cut.stream_id);
            if (
                !r ||
                r.handle !== cut.stream_local_id ||
                r.identity.physical_incarnation !== cut.physical_incarnation ||
                r.identity.stream_generation !== cut.stream_generation ||
                r.identity.connection_generation !== v.target_generation ||
                ids.has(cut.stream_id) ||
                handles.has(cut.stream_local_id) ||
                physical.has(cut.physical_incarnation)
            )
                bad();
            ids.add(cut.stream_id);
            handles.add(cut.stream_local_id);
            physical.add(cut.physical_incarnation);
        }
        audience = Object.freeze({
            scope: "public",
            streams: Object.freeze(streams),
        });
    }
    return Object.freeze({
        ...v,
        audience,
    }) as unknown as NativeActivationBoundary;
}
/** Original fresh cursor begins at zero. A half-range or wrap is unclassifiable,
 * never silently rebased. Every fragment of one decoded frame must agree. */
export function nativeMembership(
    cut: NativeSequenceCut,
    observation: NativeDecodedObservation,
): "private" | "public" {
    nativeCut(cut);
    const i = nativeEgressIdentity(observation.identity);
    if (
        observation.profile !== "connect/native-decoded-sequence/1" ||
        i.physical_incarnation !== cut.physical_incarnation ||
        i.stream_generation !== cut.stream_generation ||
        !Array.isArray(observation.datagramSequences) ||
        observation.datagramSequences.length < 1 ||
        observation.datagramSequences.length > 64 ||
        new Set(observation.datagramSequences).size !==
            observation.datagramSequences.length
    )
        bad();
    const membership = observation.datagramSequences.map((s) => {
        if (!uint(s, 0x7fffffff)) bad();
        return s >= cut.public_sequence_floor;
    });
    if (membership.some((v) => v !== membership[0])) bad();
    return membership[0] ? "public" : "private";
}
export function sameNativeIdentity(
    a: NativeEgressIdentity | undefined,
    b: NativeEgressIdentity | undefined,
): boolean {
    return (
        canonicalJson((a ?? null) as unknown as JsonObject) ===
        canonicalJson((b ?? null) as unknown as JsonObject)
    );
}
