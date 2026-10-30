import type {
    BrowserConnection,
    SdkRevision,
    StreamId,
    ParticipantId,
} from "./browser_sdk.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import {
    canonicalJson,
    jsonObject,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";

export type BrowserMedium = "audio" | "video" | "screen" | "data";
export type BrowserDirection =
    "sendonly" | "recvonly" | "sendrecv" | "inactive";
export interface BrowserNonEmpty<T> {
    readonly first: T;
    readonly rest: readonly T[];
}
export type BrowserStreamSelector =
    | { readonly stream: StreamId }
    | {
          readonly participant: ParticipantId;
          readonly kinds?: BrowserNonEmpty<BrowserMedium>;
      };
export type BrowserConnectionUpdate =
    | {
          readonly action: "mute" | "unmute";
          readonly streams: BrowserNonEmpty<StreamId>;
      }
    | {
          readonly action: "hold";
          readonly streams?: BrowserNonEmpty<StreamId>;
          readonly direction: "sendonly" | "recvonly" | "inactive";
      }
    | {
          readonly action: "resume";
          readonly streams?: BrowserNonEmpty<StreamId>;
      }
    | { readonly action: "quality"; readonly intervalMs: number }
    | {
          readonly action: "codec-renegotiate";
          readonly streams?: BrowserNonEmpty<StreamId>;
          readonly codecPreferences: BrowserNonEmpty<string>;
      };
export interface BrowserCodecObservation {
    readonly parameters: {
        readonly id: StreamId;
        readonly medium: BrowserMedium;
        readonly direction: BrowserDirection;
        readonly codec: Json;
        readonly streamLocalId: number;
    };
    readonly streamGeneration: number;
    readonly connectionGeneration: number;
}
export interface BrowserStreamGateObservation extends BrowserCodecObservation {
    readonly state: "active" | "paused";
    readonly sourceMuted: boolean;
    readonly heldDirection?: BrowserDirection;
}
export interface BrowserConnectionUpdateReceipt {
    readonly connection: BrowserConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly outcome:
        | {
              readonly kind: "codecs";
              readonly resourcesChanged: boolean;
              readonly streams: readonly BrowserCodecObservation[];
          }
        | {
              readonly kind: "gates";
              readonly action: "hold" | "resume" | "mute" | "unmute";
              readonly streams: readonly BrowserStreamGateObservation[];
          }
        | { readonly kind: "quality"; readonly intervalMs: number }
        | { readonly kind: "ice-restart"; readonly iceGeneration: number };
}
export interface BrowserStreamSelectionReceipt {
    readonly connection: BrowserConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly resourcesChanged: boolean;
    readonly bindings: readonly BrowserCodecObservation[] | null;
}
const positive = (v: unknown): v is number =>
    Number.isSafeInteger(v) && Number(v) > 0;
const medium = (v: unknown): v is BrowserMedium =>
    typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
const direction = (v: unknown): v is BrowserDirection =>
    typeof v === "string" && ["sendonly", "recvonly", "sendrecv"].includes(v);
const codecName = (v: unknown): v is string =>
    typeof v === "string" && /^[A-Za-z0-9._/-]{1,64}$/.test(v);
const object = (v: unknown): v is JsonObject => jsonObject(v as Json);
function freeze<T>(v: T): T {
    if (v && typeof v === "object") {
        for (const child of Object.values(v)) freeze(child);
        Object.freeze(v);
    }
    return v;
}
function only(v: object, allowed: readonly string[], bad: () => never) {
    if (Object.keys(v).some((k) => !allowed.includes(k))) bad();
}
function nonempty<T>(
    v: BrowserNonEmpty<T>,
    check: (value: unknown) => boolean,
    bad: () => never,
    maximum = 128,
): T[] {
    if (
        !v ||
        typeof v !== "object" ||
        !Array.isArray(v.rest) ||
        v.rest.length >= maximum ||
        !check(v.first) ||
        v.rest.some((x) => !check(x))
    )
        return bad();
    only(v, ["first", "rest"], bad);
    return [v.first, ...v.rest];
}
function ids(
    v: BrowserNonEmpty<StreamId>,
    bad: () => never,
    maximum = 128,
): StreamId[] {
    const values = nonempty(v, identifier, bad, maximum);
    if (new Set(values).size !== values.length) bad();
    return values;
}
export function subscribeBody(
    selectors: BrowserNonEmpty<BrowserStreamSelector>,
    bad: () => never,
): JsonObject {
    const seen = new Set<string>();
    const subscriptions = nonempty(selectors, object, bad).map(
        (v): JsonObject => {
            if ("stream" in v) {
                only(v, ["stream"], bad);
                if (!identifier(v.stream) || seen.has("s:" + v.stream))
                    return bad();
                seen.add("s:" + v.stream);
                return { strm_id: v.stream };
            }
            only(v, ["participant", "kinds"], bad);
            if (!identifier(v.participant) || seen.has("p:" + v.participant))
                return bad();
            seen.add("p:" + v.participant);
            const kinds =
                v.kinds === undefined
                    ? undefined
                    : nonempty(v.kinds, medium, bad, 4);
            if (kinds && new Set(kinds).size !== kinds.length) return bad();
            return {
                from_participant: v.participant,
                ...(kinds ? { kinds } : {}),
            };
        },
    );
    return { subscriptions };
}
export function unsubscribeBody(
    streams: BrowserNonEmpty<StreamId>,
    bad: () => never,
): JsonObject {
    return { strm_ids: ids(streams, bad) };
}
export function updateBody(
    input: BrowserConnectionUpdate,
    bad: () => never,
): JsonObject {
    if (!input || typeof input !== "object") return bad();
    const streams = (v: BrowserNonEmpty<StreamId> | undefined): JsonObject =>
        v === undefined ? {} : { streams: ids(v, bad, 32) };
    switch (input.action) {
        case "mute":
        case "unmute":
            only(input, ["action", "streams"], bad);
            return {
                action: input.action,
                streams: ids(input.streams, bad, 32),
            };
        case "hold":
            only(input, ["action", "streams", "direction"], bad);
            if (!["sendonly", "recvonly", "inactive"].includes(input.direction))
                return bad();
            return {
                action: input.action,
                ...streams(input.streams),
                details: { direction: input.direction },
            };
        case "resume":
            only(input, ["action", "streams"], bad);
            return { action: input.action, ...streams(input.streams) };
        case "quality":
            only(input, ["action", "intervalMs"], bad);
            if (
                !Number.isInteger(input.intervalMs) ||
                input.intervalMs < 100 ||
                input.intervalMs > 60000
            )
                return bad();
            return {
                action: input.action,
                quality_interval_ms: input.intervalMs,
            };
        case "codec-renegotiate": {
            only(input, ["action", "streams", "codecPreferences"], bad);
            const codecs = nonempty(input.codecPreferences, codecName, bad, 32);
            if (new Set(codecs).size !== codecs.length) return bad();
            return {
                action: input.action,
                ...streams(input.streams),
                codec_preferences: codecs,
            };
        }
        default:
            return bad();
    }
}
function codecs(
    value: Json,
    generation: number,
    bad: () => never,
    empty = false,
): readonly BrowserCodecObservation[] {
    if (
        !Array.isArray(value) ||
        value.length > 32 ||
        (!empty && !value.length) ||
        new TextEncoder().encode(canonicalJson(value)).length > 32768
    )
        return bad();
    const names = new Set(),
        handles = new Set();
    return freeze(
        value.map((v) => {
            if (
                !object(v) ||
                !identifier(v.stream_id) ||
                !medium(v.medium) ||
                !direction(v.direction) ||
                !positive(v.stream_local_id) ||
                v.stream_local_id > 65535 ||
                !positive(v.stream_generation) ||
                v.connection_generation !== generation ||
                !object(v.codec) ||
                !codecName(v.codec.name) ||
                !object(v.codec.params) ||
                names.has(v.stream_id) ||
                handles.has(v.stream_local_id)
            )
                return bad();
            names.add(v.stream_id);
            handles.add(v.stream_local_id);
            return {
                parameters: {
                    id: v.stream_id as StreamId,
                    medium: v.medium,
                    direction: v.direction,
                    codec: v.codec,
                    streamLocalId: v.stream_local_id,
                },
                streamGeneration: v.stream_generation,
                connectionGeneration: generation,
            };
        }),
    );
}
export function selectionResult(
    value: Json,
    key: BrowserOperationReference,
    connection: BrowserConnection,
    bad: () => never,
): BrowserStreamSelectionReceipt {
    if (
        !object(value) ||
        value.connection_id !== key.scope.connectionId ||
        value.command_type !== key.command ||
        !positive(value.connection_generation) ||
        !positive(value.expected_revision) ||
        !positive(value.stream_revision) ||
        value.stream_revision < value.expected_revision ||
        value.stream_revision > value.expected_revision + 1 ||
        typeof value.resources_changed !== "boolean" ||
        (value.resources_changed &&
            value.stream_revision !== value.expected_revision + 1) ||
        !Object.hasOwn(value, "stream_bindings")
    )
        return bad();
    return freeze({
        connection,
        connectionGeneration: value.connection_generation,
        streamRevision: value.stream_revision as SdkRevision<"stream">,
        resourcesChanged: value.resources_changed,
        bindings:
            value.stream_bindings === null
                ? null
                : codecs(
                      value.stream_bindings,
                      value.connection_generation,
                      bad,
                      true,
                  ),
    });
}
export function updateResult(
    value: Json,
    key: BrowserOperationReference,
    connection: BrowserConnection,
    bad: () => never,
): BrowserConnectionUpdateReceipt {
    if (
        !object(value) ||
        value.connection_id !== key.scope.connectionId ||
        !positive(value.stream_revision)
    )
        return bad();
    let generation: number, outcome: BrowserConnectionUpdateReceipt["outcome"];
    if (value.action === "ice-restart") {
        if (
            !positive(value.connection_generation) ||
            !positive(value.ice_generation) ||
            value.resources_changed !== false ||
            [
                "codec_bindings",
                "gate_streams",
                "quality_interval_ms",
                "setup",
                "sdp",
            ].some((k) => Object.hasOwn(value, k))
        )
            return bad();
        generation = value.connection_generation;
        outcome = { kind: "ice-restart", iceGeneration: value.ice_generation };
    } else if (Object.hasOwn(value, "ice_generation")) {
        return bad();
    } else if (Object.hasOwn(value, "quality_interval_ms")) {
        if (
            !positive(value.connection_generation) ||
            !Number.isInteger(value.quality_interval_ms) ||
            Number(value.quality_interval_ms) < 100 ||
            Number(value.quality_interval_ms) > 60000 ||
            value.resources_changed !== false ||
            value.codec_bindings !== undefined ||
            value.gate_streams !== undefined
        )
            return bad();
        generation = value.connection_generation;
        outcome = {
            kind: "quality",
            intervalMs: Number(value.quality_interval_ms),
        };
    } else if (Object.hasOwn(value, "gate_streams")) {
        if (
            !positive(value.connection_generation) ||
            value.resources_changed !== false ||
            !Array.isArray(value.gate_streams) ||
            !value.gate_streams.length ||
            value.codec_bindings !== undefined ||
            !["hold", "resume", "mute", "unmute"].includes(String(value.action))
        )
            return bad();
        generation = value.connection_generation;
        const bindings = codecs(
            value.gate_streams.map((v) => (object(v) ? v.binding : null)),
            generation,
            bad,
        );
        const streams = value.gate_streams.map(
            (v, i): BrowserStreamGateObservation => {
                if (
                    !object(v) ||
                    !["active", "paused"].includes(String(v.state)) ||
                    typeof v.source_muted !== "boolean" ||
                    (v.held_direction !== null &&
                        !direction(v.held_direction)) ||
                    (value.action === "mute" && !v.source_muted) ||
                    (value.action === "unmute" && v.source_muted) ||
                    (value.action === "resume" &&
                        (v.state !== "active" || v.held_direction !== null)) ||
                    (value.action === "hold" &&
                        v.state !== "paused" &&
                        v.held_direction === null)
                )
                    return bad();
                return {
                    ...bindings[i],
                    state: v.state as "active" | "paused",
                    sourceMuted: v.source_muted,
                    ...(v.held_direction === null
                        ? {}
                        : {
                              heldDirection:
                                  v.held_direction as BrowserDirection,
                          }),
                };
            },
        );
        outcome = {
            kind: "gates",
            action: value.action as "hold" | "resume" | "mute" | "unmute",
            streams: freeze(streams),
        };
    } else {
        if (
            !Array.isArray(value.codec_bindings) ||
            !value.codec_bindings.length ||
            typeof value.resources_changed !== "boolean"
        )
            return bad();
        const first = value.codec_bindings[0];
        if (!object(first) || !positive(first.connection_generation))
            return bad();
        generation = first.connection_generation;
        if (
            value.connection_generation !== undefined &&
            value.connection_generation !== generation
        )
            return bad();
        outcome = {
            kind: "codecs",
            resourcesChanged: value.resources_changed,
            streams: codecs(value.codec_bindings, generation, bad),
        };
    }
    return freeze({
        connection,
        connectionGeneration: generation,
        streamRevision: value.stream_revision as SdkRevision<"stream">,
        outcome,
    });
}
