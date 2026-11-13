// Package-private provenance only. This module is not part of ./browser exports.
// It confers no IO permission; the original route/registry remains authoritative.
export interface VideoPublication {
    active: boolean;
    epoch: 0 | 1;
    needed: boolean;
    boundaryMs: number;
}
export interface CaptureRegistration {
    readonly publication?: VideoPublication;
    readonly epoch?: 0 | 1;
    readonly registeredMs: number;
}
interface OwnedInput {
    registration: CaptureRegistration;
    captureTimeMs?: number;
    deliveredMs: number;
    presentationTimeMs?: number;
}
export interface VideoPublicationInput {
    readonly publication: VideoPublication;
    readonly epoch: 0 | 1;
    readonly timestampUs: number;
    readonly basis:
        | "explicit-sdk-submission"
        | "owned-capture-clock"
        | "owned-new-callback-presentation-only";
    readonly captureTimeMs?: number;
    accepted: boolean;
    output: boolean;
}
const lookups = new WeakMap<
    object,
    (id: string, stream: string) => VideoPublication | undefined
>();
const connections = new WeakMap<object, { peer: object; id: string }>();
const owned = new WeakMap<object, OwnedInput>();
const inputs = new WeakMap<object, VideoPublicationInput>();
const encoded = new WeakMap<Uint8Array, VideoPublicationInput>();
interface Group {
    input: VideoPublicationInput;
    key: boolean;
    remaining: number;
    failed: boolean;
    deadlineMs: number;
}
interface Packet {
    group: Group;
    settled: boolean;
}
const packets = new WeakMap<Uint8Array, Packet>();
export function videoPublication(): VideoPublication {
    return { active: true, epoch: 0, needed: false, boundaryMs: Infinity };
}
export function completeVideoPublication(
    value: VideoPublication,
    at = performance.now(),
): void {
    if (!value.active || value.epoch !== 0 || !Number.isFinite(at))
        throw new Error("Original replacement publication boundary changed");
    value.boundaryMs = at;
    value.epoch = 1;
    value.needed = true;
}
export function closeVideoPublication(
    value: VideoPublication | undefined,
): void {
    if (value) value.active = false;
}
export function publicationLookup(
    peer: object,
    lookup: (id: string, stream: string) => VideoPublication | undefined,
): void {
    lookups.set(peer, lookup);
}
export function publicationConnection(
    connection: object,
    peer: object,
    id: string,
): void {
    connections.set(connection, { peer, id });
}
export function capturePublicationRegistration(
    connection: object,
    stream: string,
): CaptureRegistration {
    const source = connections.get(connection);
    const publication = source
        ? lookups.get(source.peer)?.(source.id, stream)
        : undefined;
    return Object.freeze({
        publication,
        epoch: publication?.epoch,
        registeredMs: performance.now(),
    });
}
export function ownPublicationFrame(
    frame: object,
    registration: CaptureRegistration,
    captureTimeMs?: number,
    presentationTimeMs?: number,
): void {
    owned.set(frame, {
        registration,
        captureTimeMs,
        presentationTimeMs,
        deliveredMs: performance.now(),
    });
}
// false refuses the SAME owned input. undefined preserves ordinary publishers.
export function publicationInput(
    publication: VideoPublication | undefined,
    frame: VideoFrame,
): VideoPublicationInput | false | undefined {
    const original = owned.get(frame),
        stamp = original?.registration;
    if (
        stamp?.publication &&
        (!stamp.publication.active || stamp.epoch !== stamp.publication.epoch)
    )
        return false;
    if (stamp?.publication && stamp.publication !== publication) return false;
    if (!publication) return undefined;
    if (!publication.active) return false;
    if (
        original &&
        (!Number.isFinite(stamp!.registeredMs) ||
            !Number.isFinite(original.deliveredMs) ||
            (original.captureTimeMs !== undefined &&
                !Number.isFinite(original.captureTimeMs)) ||
            (original.presentationTimeMs !== undefined &&
                !Number.isFinite(original.presentationTimeMs)))
    )
        return false;
    if (
        original &&
        (stamp?.publication !== publication ||
            stamp!.epoch !== publication.epoch)
    )
        return false;
    if (
        publication.epoch === 1 &&
        original &&
        (stamp!.registeredMs < publication.boundaryMs ||
            original.deliveredMs < publication.boundaryMs ||
            (original.captureTimeMs !== undefined &&
                original.captureTimeMs < publication.boundaryMs) ||
            (original.captureTimeMs === undefined &&
                (original.presentationTimeMs === undefined ||
                    original.presentationTimeMs < publication.boundaryMs)))
    )
        return false;
    const value: VideoPublicationInput = {
        publication,
        epoch: publication.epoch,
        timestampUs: frame.timestamp,
        basis: !original
            ? "explicit-sdk-submission"
            : original.captureTimeMs !== undefined
              ? "owned-capture-clock"
              : "owned-new-callback-presentation-only",
        captureTimeMs: original?.captureTimeMs,
        accepted: false,
        output: false,
    };
    inputs.set(frame, value);
    return value;
}
export function originalPublicationInput(
    frame: object,
): VideoPublicationInput | undefined {
    return inputs.get(frame);
}
export function acceptPublicationInput(
    input: VideoPublicationInput | undefined,
): void {
    if (input) input.accepted = true;
}
export function encodedPublication(
    bytes: Uint8Array,
    input: VideoPublicationInput | undefined,
): void {
    if (input?.accepted && !input.output) {
        input.output = true;
        encoded.set(bytes, input);
    }
}
export function takeEncodedPublication(
    bytes: Uint8Array,
): VideoPublicationInput | undefined {
    const value = encoded.get(bytes);
    encoded.delete(bytes);
    return value;
}
export function currentPublicationOutput(
    publication: VideoPublication,
    input: VideoPublicationInput | undefined,
    timestampUs: number,
    key: boolean,
): boolean {
    return (
        !!input &&
        input.accepted &&
        input.publication === publication &&
        publication.active &&
        input.epoch === publication.epoch &&
        input.timestampUs === timestampUs &&
        !(publication.epoch === 1 && publication.needed && !key)
    );
}
export function publicationPackets(
    input: VideoPublicationInput,
    key: boolean,
    bytes: Uint8Array[],
    createdMs: number,
): void {
    const group: Group = {
        input,
        key,
        remaining: bytes.length,
        failed: false,
        deadlineMs: createdMs + 200,
    };
    for (const value of bytes) packets.set(value, { group, settled: false });
}
// Move metadata through the EXISTING clone; no payload copy or public API added.
export function clonePublicationPacket(
    original: Uint8Array,
    clone: Uint8Array,
): void {
    const value = packets.get(original);
    if (value) {
        packets.delete(original);
        packets.set(clone, value);
    }
}
export function currentPublicationPacket(
    bytes: Uint8Array,
    expected?: VideoPublication,
): boolean {
    const value = packets.get(bytes);
    if (!value) return !expected;
    return (
        !!expected &&
        value.group.input.publication === expected &&
        expected.active &&
        value.group.input.epoch === expected.epoch
    );
}
export function settlePublicationPacket(
    bytes: Uint8Array,
    sent: boolean,
    at = performance.now(),
): void {
    const value = packets.get(bytes);
    packets.delete(bytes);
    if (!value || value.settled) return;
    value.settled = true;
    const group = value.group;
    group.failed ||= !sent;
    if (--group.remaining !== 0) return;
    const input = group.input,
        publication = input.publication;
    if (
        !group.failed &&
        group.key &&
        at < group.deadlineMs &&
        publication.active &&
        input.accepted &&
        input.epoch === 1 &&
        publication.epoch === 1
    )
        publication.needed = false;
}
