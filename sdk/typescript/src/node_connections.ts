import type { SdkMoveInput, SdkMoveResult } from "./node_movements.js";
import { sdkRevision } from "./node_sdk.js";
import type {
    ConversationId,
    ParticipantId,
    SdkId,
    SdkRevision,
    SdkMutationOptions,
    SdkOperation,
    SdkOperationKind,
} from "./node_sdk.js";
import type { SessionId, SdkMedium, SdkNonEmpty } from "./node_sessions.js";
import { object, NodeSdkError } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
import type { Json, JsonObject } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";

export type ConnectionId = SdkId<"connection">;
export type StreamId = SdkId<"stream">;
export type SdkDirection = "sendonly" | "recvonly" | "sendrecv" | "inactive";
export interface SdkOfferedStream {
    readonly id: StreamId;
    readonly kind: SdkMedium;
    readonly direction: SdkDirection;
    readonly codecs: SdkNonEmpty<string>;
}
export interface SdkConnectionSnapshot {
    readonly id: ConnectionId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly generation: number;
    readonly streamRevision?: SdkRevision<"stream">;
    readonly state: string;
}
export interface SdkNegotiatedStream {
    readonly id: StreamId;
    readonly medium: SdkMedium;
    readonly direction: SdkDirection;
    readonly codec: Json;
    readonly streamLocalId?: number;
}
export interface SdkCodecObservation {
    readonly parameters: SdkNegotiatedStream;
    readonly streamGeneration: number;
    readonly connectionGeneration: number;
}
export interface SdkStreamGateObservation extends SdkCodecObservation {
    readonly state: "active" | "paused";
    readonly sourceMuted: boolean;
    readonly heldDirection?: SdkDirection;
}
export type SdkConnectionUpdateOutcome =
    | {
          readonly kind: "codecs";
          readonly resourcesChanged: boolean;
          readonly streams: readonly SdkCodecObservation[];
      }
    | {
          readonly kind: "gates";
          readonly action: "hold" | "resume" | "mute" | "unmute";
          readonly streams: readonly SdkStreamGateObservation[];
      }
    | { readonly kind: "quality"; readonly intervalMs: number };
export interface SdkConnectionUpdateReceipt {
    readonly connection: SdkConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly outcome: SdkConnectionUpdateOutcome;
}
export interface SdkStreamSelectionReceipt {
    readonly connection: SdkConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly resourcesChanged: boolean;
    readonly bindings: readonly SdkCodecObservation[] | null;
}
export type SdkStreamSelector =
    | { readonly stream: StreamId }
    | {
          readonly participant: ParticipantId;
          readonly kinds?: SdkNonEmpty<SdkMedium>;
      };
export type SdkConnectionUpdate =
    | {
          readonly action: "mute" | "unmute";
          readonly streams: SdkNonEmpty<StreamId>;
      }
    | {
          readonly action: "hold";
          readonly streams?: SdkNonEmpty<StreamId>;
          readonly direction: "sendonly" | "recvonly" | "inactive";
      }
    | { readonly action: "resume"; readonly streams?: SdkNonEmpty<StreamId> }
    | { readonly action: "quality"; readonly intervalMs: number }
    | {
          readonly action: "codec-renegotiate";
          readonly streams?: SdkNonEmpty<StreamId>;
          readonly codecPreferences: SdkNonEmpty<string>;
      };
export interface SdkStreamSnapshot {
    readonly id: StreamId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly connectionId: ConnectionId;
    readonly medium: SdkMedium;
    readonly state: string;
    readonly resourceRevision: SdkRevision<"stream-resource">;
    readonly direction?: SdkDirection;
    readonly format?: {
        readonly codec: string;
        readonly clockRate: number;
        readonly channels?: number;
        readonly width?: number;
        readonly height?: number;
    };
    readonly clock?: {
        readonly sourceId: string;
        readonly generation: number;
        readonly unitsPerSecond: bigint;
        readonly synchronized: boolean;
    };
    readonly ownership:
        "observation" | "local-source" | "local-sink" | "local-duplex";
}
export interface SdkConnections {
    byId(id: ConnectionId): SdkConnection;
    offer(
        input: {
            readonly connectionId: ConnectionId;
            readonly streams: SdkNonEmpty<SdkOfferedStream>;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkConnection>>;
}
interface Kinds {
    readonly connection_offer: SdkOperationKind<SdkConnection>;
    readonly connection_update: SdkOperationKind<SdkConnectionUpdateReceipt>;
    readonly connection_end: SdkOperationKind<SdkConnection>;
    readonly stream_subscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
    readonly stream_unsubscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
}
export interface ConnectionContext {
    move(
        id: ConnectionId,
        input: SdkMoveInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMoveResult>>;
    readonly cid: ConversationId;
    readonly sid: SessionId;
    readonly kinds: Kinds;
    mutate<T>(
        id: ConnectionId,
        action: JsonObject,
        kind: SdkOperationKind<T>,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<T>>;
    read<T>(
        id: ConnectionId,
        action: JsonObject,
        options: CanonicalRequestOptions,
        decode: (value: Json) => T,
    ): Promise<T>;
}
function invalid(): never {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol(): never {
    throw new Error("Invalid typed native control result");
}
const medium = (v: unknown): v is SdkMedium =>
    typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
const direction = (v: unknown): v is SdkDirection =>
    typeof v === "string" &&
    ["sendonly", "recvonly", "sendrecv", "inactive"].includes(v);
const natural = (v: unknown): v is number =>
    Number.isSafeInteger(v) && Number(v) >= 0;
const positive = (v: unknown): v is number => natural(v) && v > 0;
function nonempty<T>(
    v: SdkNonEmpty<T>,
    check: (value: unknown) => boolean,
): T[] {
    if (
        !v ||
        !Array.isArray(v.rest) ||
        !check(v.first) ||
        v.rest.some((x) => !check(x))
    )
        invalid();
    return [v.first, ...v.rest];
}
function frozen<T>(v: T): T {
    if (v && typeof v === "object") {
        for (const x of Object.values(v)) frozen(x);
        Object.freeze(v);
    }
    return v;
}
export function createConnections(ctx: ConnectionContext): SdkConnections {
    const owned = Object.freeze({ ...ctx });
    return Object.freeze({
        byId: (id: ConnectionId) => {
            if (!identifier(id)) invalid();
            return new SdkConnection(owned, id);
        },
        offer: (
            input: {
                readonly connectionId: ConnectionId;
                readonly streams: SdkNonEmpty<SdkOfferedStream>;
            },
            options: SdkMutationOptions,
        ) => {
            if (!input || !identifier(input.connectionId)) invalid();
            const streams = nonempty(
                input.streams,
                (v) => !!v && typeof v === "object",
            ).map((s) => {
                if (
                    !identifier(s.id) ||
                    !medium(s.kind) ||
                    !direction(s.direction)
                )
                    invalid();
                return {
                    id: s.id,
                    kind: s.kind,
                    direction: s.direction,
                    codecs: nonempty(
                        s.codecs,
                        (v) => typeof v === "string" && !!v,
                    ),
                };
            });
            return owned.mutate(
                input.connectionId,
                { type: "offer", streams },
                owned.kinds.connection_offer,
                options,
            );
        },
    });
}
/** Captured receipt revision is historical; byId construction supplies none. */
export class SdkConnection {
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly streams;
    constructor(
        private readonly ctx: ConnectionContext,
        readonly id: ConnectionId,
        readonly receiptStreamRevision?: SdkRevision<"stream">,
    ) {
        this.conversationId = ctx.cid;
        this.sessionId = ctx.sid;
        this.streams = Object.freeze({
            byId: (id: StreamId) => {
                if (!identifier(id)) invalid();
                return new SdkStream(ctx, this.id, id);
            },
            subscribe: (
                input: {
                    readonly selectors: SdkNonEmpty<SdkStreamSelector>;
                    readonly expectedStreamRevision: SdkRevision<"stream">;
                },
                options: SdkMutationOptions,
            ): Promise<SdkOperation<SdkStreamSelectionReceipt>> => {
                if (!input) invalid();
                const selectors = nonempty(
                    input.selectors,
                    (v) => !!v && typeof v === "object",
                ).map((v): JsonObject => {
                    if ("stream" in v) {
                        if (!identifier(v.stream) || "participant" in v)
                            invalid();
                        return { type: "stream", stream_id: v.stream };
                    }
                    if (!identifier(v.participant)) invalid();
                    return {
                        type: "participant",
                        participant_id: v.participant,
                        ...(v.kinds === undefined
                            ? {}
                            : { kinds: nonempty(v.kinds, medium) }),
                    };
                });
                return ctx.mutate(
                    this.id,
                    {
                        type: "subscribe",
                        selectors,
                        expected_revision: sdkRevision<"stream">(
                            input.expectedStreamRevision,
                        ),
                    },
                    ctx.kinds.stream_subscribe,
                    options,
                );
            },
            unsubscribe: (
                input: {
                    readonly streams: SdkNonEmpty<StreamId>;
                    readonly expectedStreamRevision: SdkRevision<"stream">;
                },
                options: SdkMutationOptions,
            ): Promise<SdkOperation<SdkStreamSelectionReceipt>> => {
                if (!input) invalid();
                return ctx.mutate(
                    this.id,
                    {
                        type: "unsubscribe",
                        streams: nonempty(input.streams, identifier),
                        expected_revision: sdkRevision<"stream">(
                            input.expectedStreamRevision,
                        ),
                    },
                    ctx.kinds.stream_unsubscribe,
                    options,
                );
            },
        });
        Object.freeze(this);
    }
    refresh(options: CanonicalRequestOptions): Promise<SdkConnectionSnapshot> {
        return this.ctx.read(this.id, { type: "refresh" }, options, (v) => {
            if (
                !object(v, [
                    "id",
                    "conversationId",
                    "sessionId",
                    "generation",
                    "streamRevision",
                    "state",
                ]) ||
                v.id !== this.id ||
                v.conversationId !== this.conversationId ||
                v.sessionId !== this.sessionId ||
                !positive(v.generation) ||
                typeof v.state !== "string" ||
                (v.streamRevision !== null && !positive(v.streamRevision))
            )
                return protocol();
            return frozen({
                id: this.id,
                conversationId: this.conversationId,
                sessionId: this.sessionId,
                generation: v.generation,
                state: v.state,
                ...(v.streamRevision === null
                    ? {}
                    : {
                          streamRevision:
                              v.streamRevision as SdkRevision<"stream">,
                      }),
            });
        });
    }
    replaceDevice(
        input: SdkMoveInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkMoveResult>> {
        return this.ctx.move(this.id, input, options);
    }
    update(
        input: SdkConnectionUpdate,
        expectedStreamRevision: SdkRevision<"stream">,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkConnectionUpdateReceipt>> {
        if (!input) invalid();
        let update: JsonObject;
        const streams = (v: SdkNonEmpty<StreamId> | undefined): JsonObject =>
            v === undefined ? {} : { streams: nonempty(v, identifier) };
        switch (input.action) {
            case "mute":
            case "unmute":
                update = {
                    action: input.action,
                    streams: nonempty(input.streams, identifier),
                };
                break;
            case "hold":
                if (
                    !["sendonly", "recvonly", "inactive"].includes(
                        input.direction,
                    )
                )
                    invalid();
                update = {
                    action: input.action,
                    direction: input.direction,
                    ...streams(input.streams),
                };
                break;
            case "resume":
                update = { action: input.action, ...streams(input.streams) };
                break;
            case "quality":
                if (!natural(input.intervalMs) || input.intervalMs > 4294967295)
                    invalid();
                update = {
                    action: input.action,
                    interval_ms: input.intervalMs,
                };
                break;
            case "codec-renegotiate":
                update = {
                    action: input.action,
                    ...streams(input.streams),
                    codec_preferences: nonempty(
                        input.codecPreferences,
                        (v) => typeof v === "string" && !!v,
                    ),
                };
                break;
            default:
                return invalid();
        }
        return this.ctx.mutate(
            this.id,
            {
                type: "update",
                update,
                expected_revision: sdkRevision<"stream">(
                    expectedStreamRevision,
                ),
            },
            this.ctx.kinds.connection_update,
            options,
        );
    }
    end(
        input: {
            readonly reasonCode: number;
            readonly reason: string;
            readonly expectedStreamRevision?: SdkRevision<"stream">;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkConnection>> {
        if (
            !input ||
            !natural(input.reasonCode) ||
            input.reasonCode > 65535 ||
            typeof input.reason !== "string"
        )
            invalid();
        return this.ctx.mutate(
            this.id,
            {
                type: "end",
                reason_code: input.reasonCode,
                reason: input.reason,
                ...(input.expectedStreamRevision === undefined
                    ? {}
                    : {
                          expected_revision: sdkRevision<"stream">(
                              input.expectedStreamRevision,
                          ),
                      }),
            },
            this.ctx.kinds.connection_end,
            options,
        );
    }
}
export class SdkStream {
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    constructor(
        private readonly ctx: ConnectionContext,
        readonly connectionId: ConnectionId,
        readonly id: StreamId,
    ) {
        this.conversationId = ctx.cid;
        this.sessionId = ctx.sid;
        Object.freeze(this);
    }
    refresh(options: CanonicalRequestOptions): Promise<SdkStreamSnapshot> {
        return this.ctx.read(
            this.connectionId,
            { type: "stream_refresh", stream_id: this.id },
            options,
            (v) => {
                if (
                    !object(v, [
                        "id",
                        "conversationId",
                        "sessionId",
                        "connectionId",
                        "medium",
                        "state",
                        "resourceRevision",
                        "direction",
                        "format",
                        "clock",
                        "ownership",
                    ]) ||
                    v.id !== this.id ||
                    v.conversationId !== this.conversationId ||
                    v.sessionId !== this.sessionId ||
                    v.connectionId !== this.connectionId ||
                    !medium(v.medium) ||
                    typeof v.state !== "string" ||
                    !natural(v.resourceRevision) ||
                    (v.direction !== null && !direction(v.direction)) ||
                    typeof v.ownership !== "string" ||
                    ![
                        "observation",
                        "local-source",
                        "local-sink",
                        "local-duplex",
                    ].includes(v.ownership)
                )
                    return protocol();
                let format: SdkStreamSnapshot["format"],
                    clock: SdkStreamSnapshot["clock"];
                if (v.format !== null) {
                    const f = v.format;
                    if (
                        !object(f, [
                            "codec",
                            "clockRate",
                            "channels",
                            "width",
                            "height",
                        ]) ||
                        typeof f.codec !== "string" ||
                        !positive(f.clockRate) ||
                        [f.channels, f.width, f.height].some(
                            (n) => n !== null && !positive(n),
                        )
                    )
                        return protocol();
                    format = {
                        codec: f.codec,
                        clockRate: f.clockRate,
                        ...(f.channels === null
                            ? {}
                            : { channels: f.channels as number }),
                        ...(f.width === null
                            ? {}
                            : { width: f.width as number }),
                        ...(f.height === null
                            ? {}
                            : { height: f.height as number }),
                    };
                }
                if (v.clock !== null) {
                    const c = v.clock;
                    if (
                        !object(c, [
                            "sourceId",
                            "generation",
                            "unitsPerSecond",
                            "synchronized",
                        ]) ||
                        typeof c.sourceId !== "string" ||
                        !c.sourceId ||
                        !positive(c.generation) ||
                        typeof c.unitsPerSecond !== "string" ||
                        !/^[1-9][0-9]{0,19}$/.test(c.unitsPerSecond) ||
                        BigInt(c.unitsPerSecond) > 18446744073709551615n ||
                        typeof c.synchronized !== "boolean"
                    )
                        return protocol();
                    clock = {
                        sourceId: c.sourceId,
                        generation: c.generation,
                        unitsPerSecond: BigInt(c.unitsPerSecond),
                        synchronized: c.synchronized,
                    };
                }
                return frozen({
                    id: this.id,
                    conversationId: this.conversationId,
                    sessionId: this.sessionId,
                    connectionId: this.connectionId,
                    medium: v.medium,
                    state: v.state,
                    resourceRevision:
                        v.resourceRevision as SdkRevision<"stream-resource">,
                    ownership: v.ownership as SdkStreamSnapshot["ownership"],
                    ...(v.direction === null
                        ? {}
                        : { direction: v.direction as SdkDirection }),
                    ...(format ? { format } : {}),
                    ...(clock ? { clock } : {}),
                });
            },
        );
    }
}
export function connectionHandle(
    v: Json,
    ctx: ConnectionContext,
    expected: ConnectionId,
): SdkConnection {
    if (
        !object(v, [
            "id",
            "conversationId",
            "sessionId",
            "receiptStreamRevision",
        ]) ||
        v.id !== expected ||
        v.conversationId !== ctx.cid ||
        v.sessionId !== ctx.sid ||
        (v.receiptStreamRevision !== null && !positive(v.receiptStreamRevision))
    )
        return protocol();
    return new SdkConnection(
        ctx,
        expected,
        v.receiptStreamRevision === null
            ? undefined
            : (v.receiptStreamRevision as SdkRevision<"stream">),
    );
}
function codec(
    v: Json,
    generation: number,
    gates = false,
): SdkCodecObservation | SdkStreamGateObservation {
    if (
        !object(v, [
            "parameters",
            "streamGeneration",
            "connectionGeneration",
            ...(gates ? ["state", "sourceMuted", "heldDirection"] : []),
        ]) ||
        !positive(v.streamGeneration) ||
        v.connectionGeneration !== generation ||
        !object(v.parameters, [
            "id",
            "medium",
            "direction",
            "codec",
            "streamLocalId",
        ])
    )
        return protocol();
    const p = v.parameters;
    if (
        !identifier(p.id) ||
        !medium(p.medium) ||
        !direction(p.direction) ||
        p.codec === undefined ||
        (p.streamLocalId !== null &&
            (!positive(p.streamLocalId) || p.streamLocalId > 65535))
    )
        return protocol();
    const parameters = {
        id: p.id as StreamId,
        medium: p.medium,
        direction: p.direction,
        codec: p.codec,
        ...(p.streamLocalId === null
            ? {}
            : { streamLocalId: p.streamLocalId as number }),
    };
    const base = {
        parameters,
        streamGeneration: v.streamGeneration,
        connectionGeneration: generation,
    };
    if (!gates) return frozen(base);
    if (
        typeof v.state !== "string" ||
        !["active", "paused"].includes(v.state) ||
        typeof v.sourceMuted !== "boolean" ||
        (v.heldDirection !== null && !direction(v.heldDirection))
    )
        return protocol();
    return frozen({
        ...base,
        state: v.state as "active" | "paused",
        sourceMuted: v.sourceMuted,
        ...(v.heldDirection === null
            ? {}
            : { heldDirection: v.heldDirection as SdkDirection }),
    });
}
export function connectionUpdate(
    v: Json,
    ctx: ConnectionContext,
    id: ConnectionId,
): SdkConnectionUpdateReceipt {
    if (
        !object(v, [
            "connection",
            "connectionGeneration",
            "streamRevision",
            "outcome",
        ]) ||
        !positive(v.connectionGeneration) ||
        !positive(v.streamRevision) ||
        !object(v.outcome, [
            "kind",
            "resourcesChanged",
            "streams",
            "action",
            "intervalMs",
        ])
    )
        return protocol();
    const connection = connectionHandle(v.connection, ctx, id),
        o = v.outcome;
    let outcome: SdkConnectionUpdateOutcome;
    if (o.kind === "quality") {
        if (
            !positive(o.intervalMs) ||
            Object.keys(o).some((k) => !["kind", "intervalMs"].includes(k))
        )
            return protocol();
        outcome = { kind: "quality", intervalMs: o.intervalMs };
    } else if (o.kind === "codecs" || o.kind === "gates") {
        if (
            !Array.isArray(o.streams) ||
            Object.keys(o).some(
                (k) =>
                    !(
                        o.kind === "codecs"
                            ? ["kind", "resourcesChanged", "streams"]
                            : ["kind", "action", "streams"]
                    ).includes(k),
            )
        )
            return protocol();
        const values = o.streams.map((s) =>
            codec(s, v.connectionGeneration as number, o.kind === "gates"),
        );
        if (o.kind === "codecs") {
            if (typeof o.resourcesChanged !== "boolean") return protocol();
            outcome = {
                kind: "codecs",
                resourcesChanged: o.resourcesChanged,
                streams: values,
            };
        } else {
            if (
                typeof o.action !== "string" ||
                !["hold", "resume", "mute", "unmute"].includes(o.action)
            )
                return protocol();
            outcome = {
                kind: "gates",
                action: o.action as "hold" | "resume" | "mute" | "unmute",
                streams: values as SdkStreamGateObservation[],
            };
        }
    } else return protocol();
    return Object.freeze({
        connection,
        connectionGeneration: v.connectionGeneration,
        streamRevision: v.streamRevision as SdkRevision<"stream">,
        outcome: frozen(outcome),
    });
}
export function streamSelection(
    v: Json,
    ctx: ConnectionContext,
    id: ConnectionId,
): SdkStreamSelectionReceipt {
    if (
        !object(v, [
            "connection",
            "connectionGeneration",
            "streamRevision",
            "resourcesChanged",
            "bindings",
        ]) ||
        !positive(v.connectionGeneration) ||
        !positive(v.streamRevision) ||
        typeof v.resourcesChanged !== "boolean" ||
        (v.bindings !== null && !Array.isArray(v.bindings))
    )
        return protocol();
    return Object.freeze({
        connection: connectionHandle(v.connection, ctx, id),
        connectionGeneration: v.connectionGeneration,
        streamRevision: v.streamRevision as SdkRevision<"stream">,
        resourcesChanged: v.resourcesChanged,
        bindings:
            v.bindings === null
                ? null
                : frozen(
                      (v.bindings as Json[]).map((s) =>
                          codec(s, v.connectionGeneration as number),
                      ),
                  ),
    });
}
