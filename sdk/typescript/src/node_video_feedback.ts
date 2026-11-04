/** Original helper-owned feedback lifetime; no media or route authority. */
import { canonicalJson, type Json } from "./uctp_json.js";
import type { MediaFrame } from "./node_media_channel.js";
import type { SdkMediaBinding } from "./node_media_types.js";
import type { StreamId } from "./node_connections.js";

export interface SdkVideoReceiveEpoch {
    readonly epoch: string;
    readonly incarnation: string;
    readonly revision: number;
    readonly ssrc: number;
    readonly firstSequence: number;
    readonly firstTimestamp: number;
}
export interface SdkVideoFeedback {
    readonly datagramSequence: number;
    readonly ssrc: number;
    readonly epoch: string;
    readonly epochRevision: number;
}
export interface SdkVideoKeyframePermit {
    readonly feedback: SdkVideoFeedback;
    /** Local original-age bound only; current server authority still gates media. */
    isFresh(): boolean;
    readonly expiresInMs: number;
}
function exact(value: Json, keys: string[]): value is { [key: string]: Json } {
    return (
        value !== null &&
        typeof value === "object" &&
        !Array.isArray(value) &&
        Object.keys(value).length === keys.length &&
        keys.every((k) => Object.hasOwn(value, k))
    );
}
function uint(value: unknown, positive = false): number {
    if (
        !Number.isSafeInteger(value) ||
        Number(value) < Number(positive) ||
        Number(value) > 0xffffffff
    )
        throw new Error();
    return Number(value);
}
function revision(value: unknown): number {
    if (!Number.isSafeInteger(value) || Number(value) < 1) throw new Error();
    return Number(value);
}
function hex(value: unknown): string {
    if (
        typeof value !== "string" ||
        !/^[0-9a-f]{32}$/.test(value) ||
        /^0+$/.test(value)
    )
        throw new Error();
    return value;
}
export function captureVideoEpoch(value: Json): SdkVideoReceiveEpoch {
    if (
        !exact(value, [
            "epoch",
            "incarnation",
            "revision",
            "ssrc",
            "first_sequence",
            "first_timestamp",
        ])
    )
        throw new Error();
    return Object.freeze({
        epoch: hex(value.epoch),
        incarnation: hex(value.incarnation),
        revision: revision(value.revision),
        ssrc: uint(value.ssrc, true),
        firstSequence: uint(value.first_sequence),
        firstTimestamp: uint(value.first_timestamp),
    });
}
function bindingKey(binding: SdkMediaBinding, id: StreamId): string {
    return JSON.stringify([
        binding.conversationId,
        binding.sessionId,
        binding.connectionId,
        id,
    ]);
}
function root(binding: SdkMediaBinding, id: StreamId, frame: MediaFrame): void {
    const v = frame.metadata;
    if (
        frame.kind !== 0x81 ||
        frame.payload.length ||
        v.stream_id !== id ||
        canonicalJson(v.binding) !==
            canonicalJson({
                conversation_id: binding.conversationId,
                session_id: binding.sessionId,
                connection_id: binding.connectionId,
            })
    )
        throw new Error();
}
const references = new WeakMap<
    SdkVideoFeedback,
    { owner: object; key: string; token: string }
>();
export function captureVideoFeedback(
    owner: object,
    binding: SdkMediaBinding,
    id: StreamId,
    frame: MediaFrame,
): SdkVideoFeedback {
    root(binding, id, frame);
    const v = frame.metadata;
    if (
        !exact(v, [
            "type",
            "binding",
            "stream_id",
            "token",
            "datagram_sequence",
            "ssrc",
            "epoch",
            "epoch_revision",
        ]) ||
        v.type !== "video_feedback"
    )
        throw new Error();
    const token = hex(v.token),
        result = Object.freeze({
            datagramSequence: uint(v.datagram_sequence),
            ssrc: uint(v.ssrc, true),
            epoch: hex(v.epoch),
            epochRevision: revision(v.epoch_revision),
        });
    references.set(result, { owner, key: bindingKey(binding, id), token });
    return result;
}
export function takeVideoFeedback(
    owner: object,
    binding: SdkMediaBinding,
    id: StreamId,
    value: SdkVideoFeedback,
): string {
    const ref = references.get(value);
    if (!ref || ref.owner !== owner || ref.key !== bindingKey(binding, id))
        throw new Error();
    references.delete(value);
    return ref.token;
}
export function consumedVideoFeedback(
    binding: SdkMediaBinding,
    id: StreamId,
    feedback: SdkVideoFeedback,
    token: string,
    started: number,
    frame: MediaFrame,
): SdkVideoKeyframePermit | undefined {
    root(binding, id, frame);
    const v = frame.metadata;
    if (
        !exact(v, ["type", "binding", "stream_id", "token", "remaining_ms"]) ||
        v.type !== "video_feedback_consumed" ||
        v.token !== token ||
        !Number.isSafeInteger(v.remaining_ms) ||
        Number(v.remaining_ms) < 0 ||
        Number(v.remaining_ms) > 200 ||
        !Number.isFinite(started) ||
        started > performance.now()
    )
        throw new Error();
    // Charging the entire RPC round trip, rather than starting a new timer at
    // response receipt, makes this deadline no later than the Rust deadline.
    const deadline = started + Number(v.remaining_ms);
    if (performance.now() >= deadline) return undefined;
    return Object.freeze({
        feedback,
        isFresh: () => performance.now() < deadline,
        get expiresInMs() {
            return Math.max(0, deadline - performance.now());
        },
    });
}
