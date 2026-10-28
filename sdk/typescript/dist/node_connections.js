import { sdkRevision } from "./node_sdk.js";
import { object, NodeSdkError } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
function invalid() {
    throw new NodeSdkError("validation", "not-submitted");
}
function protocol() {
    throw new Error("Invalid typed native control result");
}
const medium = (v) => typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
const direction = (v) => typeof v === "string" &&
    ["sendonly", "recvonly", "sendrecv", "inactive"].includes(v);
const natural = (v) => Number.isSafeInteger(v) && Number(v) >= 0;
const positive = (v) => natural(v) && v > 0;
function nonempty(v, check) {
    if (!v ||
        !Array.isArray(v.rest) ||
        !check(v.first) ||
        v.rest.some((x) => !check(x)))
        invalid();
    return [v.first, ...v.rest];
}
function frozen(v) {
    if (v && typeof v === "object") {
        for (const x of Object.values(v))
            frozen(x);
        Object.freeze(v);
    }
    return v;
}
export function createConnections(ctx) {
    const owned = Object.freeze({ ...ctx });
    return Object.freeze({
        byId: (id) => {
            if (!identifier(id))
                invalid();
            return new SdkConnection(owned, id);
        },
        offer: (input, options) => {
            if (!input || !identifier(input.connectionId))
                invalid();
            const streams = nonempty(input.streams, (v) => !!v && typeof v === "object").map((s) => {
                if (!identifier(s.id) ||
                    !medium(s.kind) ||
                    !direction(s.direction))
                    invalid();
                return {
                    id: s.id,
                    kind: s.kind,
                    direction: s.direction,
                    codecs: nonempty(s.codecs, (v) => typeof v === "string" && !!v),
                };
            });
            return owned.mutate(input.connectionId, { type: "offer", streams }, owned.kinds.connection_offer, options);
        },
    });
}
/** Captured receipt revision is historical; byId construction supplies none. */
export class SdkConnection {
    ctx;
    id;
    receiptStreamRevision;
    conversationId;
    sessionId;
    streams;
    constructor(ctx, id, receiptStreamRevision) {
        this.ctx = ctx;
        this.id = id;
        this.receiptStreamRevision = receiptStreamRevision;
        this.conversationId = ctx.cid;
        this.sessionId = ctx.sid;
        this.streams = Object.freeze({
            byId: (id) => {
                if (!identifier(id))
                    invalid();
                return new SdkStream(ctx, this.id, id);
            },
            subscribe: (input, options) => {
                if (!input)
                    invalid();
                const selectors = nonempty(input.selectors, (v) => !!v && typeof v === "object").map((v) => {
                    if ("stream" in v) {
                        if (!identifier(v.stream) || "participant" in v)
                            invalid();
                        return { type: "stream", stream_id: v.stream };
                    }
                    if (!identifier(v.participant))
                        invalid();
                    return {
                        type: "participant",
                        participant_id: v.participant,
                        ...(v.kinds === undefined
                            ? {}
                            : { kinds: nonempty(v.kinds, medium) }),
                    };
                });
                return ctx.mutate(this.id, {
                    type: "subscribe",
                    selectors,
                    expected_revision: sdkRevision(input.expectedStreamRevision),
                }, ctx.kinds.stream_subscribe, options);
            },
            unsubscribe: (input, options) => {
                if (!input)
                    invalid();
                return ctx.mutate(this.id, {
                    type: "unsubscribe",
                    streams: nonempty(input.streams, identifier),
                    expected_revision: sdkRevision(input.expectedStreamRevision),
                }, ctx.kinds.stream_unsubscribe, options);
            },
        });
        Object.freeze(this);
    }
    refresh(options) {
        return this.ctx.read(this.id, { type: "refresh" }, options, (v) => {
            if (!object(v, [
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
                (v.streamRevision !== null && !positive(v.streamRevision)))
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
                        streamRevision: v.streamRevision,
                    }),
            });
        });
    }
    move(input, options) {
        return this.ctx.move(this.id, input, options);
    }
    update(input, expectedStreamRevision, options) {
        if (!input)
            invalid();
        let update;
        const streams = (v) => v === undefined ? {} : { streams: nonempty(v, identifier) };
        switch (input.action) {
            case "mute":
            case "unmute":
                update = {
                    action: input.action,
                    streams: nonempty(input.streams, identifier),
                };
                break;
            case "hold":
                if (!["sendonly", "recvonly", "inactive"].includes(input.direction))
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
                    codec_preferences: nonempty(input.codecPreferences, (v) => typeof v === "string" && !!v),
                };
                break;
            default:
                return invalid();
        }
        return this.ctx.mutate(this.id, {
            type: "update",
            update,
            expected_revision: sdkRevision(expectedStreamRevision),
        }, this.ctx.kinds.connection_update, options);
    }
    end(input, options) {
        if (!input ||
            !natural(input.reasonCode) ||
            input.reasonCode > 65535 ||
            typeof input.reason !== "string")
            invalid();
        return this.ctx.mutate(this.id, {
            type: "end",
            reason_code: input.reasonCode,
            reason: input.reason,
            ...(input.expectedStreamRevision === undefined
                ? {}
                : {
                    expected_revision: sdkRevision(input.expectedStreamRevision),
                }),
        }, this.ctx.kinds.connection_end, options);
    }
}
export class SdkStream {
    ctx;
    connectionId;
    id;
    conversationId;
    sessionId;
    constructor(ctx, connectionId, id) {
        this.ctx = ctx;
        this.connectionId = connectionId;
        this.id = id;
        this.conversationId = ctx.cid;
        this.sessionId = ctx.sid;
        Object.freeze(this);
    }
    refresh(options) {
        return this.ctx.read(this.connectionId, { type: "stream_refresh", stream_id: this.id }, options, (v) => {
            if (!object(v, [
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
                ].includes(v.ownership))
                return protocol();
            let format, clock;
            if (v.format !== null) {
                const f = v.format;
                if (!object(f, [
                    "codec",
                    "clockRate",
                    "channels",
                    "width",
                    "height",
                ]) ||
                    typeof f.codec !== "string" ||
                    !positive(f.clockRate) ||
                    [f.channels, f.width, f.height].some((n) => n !== null && !positive(n)))
                    return protocol();
                format = {
                    codec: f.codec,
                    clockRate: f.clockRate,
                    ...(f.channels === null
                        ? {}
                        : { channels: f.channels }),
                    ...(f.width === null
                        ? {}
                        : { width: f.width }),
                    ...(f.height === null
                        ? {}
                        : { height: f.height }),
                };
            }
            if (v.clock !== null) {
                const c = v.clock;
                if (!object(c, [
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
                    typeof c.synchronized !== "boolean")
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
                resourceRevision: v.resourceRevision,
                ownership: v.ownership,
                ...(v.direction === null
                    ? {}
                    : { direction: v.direction }),
                ...(format ? { format } : {}),
                ...(clock ? { clock } : {}),
            });
        });
    }
}
export function connectionHandle(v, ctx, expected) {
    if (!object(v, [
        "id",
        "conversationId",
        "sessionId",
        "receiptStreamRevision",
    ]) ||
        v.id !== expected ||
        v.conversationId !== ctx.cid ||
        v.sessionId !== ctx.sid ||
        (v.receiptStreamRevision !== null && !positive(v.receiptStreamRevision)))
        return protocol();
    return new SdkConnection(ctx, expected, v.receiptStreamRevision === null
        ? undefined
        : v.receiptStreamRevision);
}
function codec(v, generation, gates = false) {
    if (!object(v, [
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
        ]))
        return protocol();
    const p = v.parameters;
    if (!identifier(p.id) ||
        !medium(p.medium) ||
        !direction(p.direction) ||
        p.codec === undefined ||
        (p.streamLocalId !== null &&
            (!positive(p.streamLocalId) || p.streamLocalId > 65535)))
        return protocol();
    const parameters = {
        id: p.id,
        medium: p.medium,
        direction: p.direction,
        codec: p.codec,
        ...(p.streamLocalId === null
            ? {}
            : { streamLocalId: p.streamLocalId }),
    };
    const base = {
        parameters,
        streamGeneration: v.streamGeneration,
        connectionGeneration: generation,
    };
    if (!gates)
        return frozen(base);
    if (!["active", "paused"].includes(String(v.state)) ||
        typeof v.sourceMuted !== "boolean" ||
        (v.heldDirection !== null && !direction(v.heldDirection)))
        return protocol();
    return frozen({
        ...base,
        state: v.state,
        sourceMuted: v.sourceMuted,
        ...(v.heldDirection === null
            ? {}
            : { heldDirection: v.heldDirection }),
    });
}
export function connectionUpdate(v, ctx, id) {
    if (!object(v, [
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
        ]))
        return protocol();
    const connection = connectionHandle(v.connection, ctx, id), o = v.outcome;
    let outcome;
    if (o.kind === "quality") {
        if (!positive(o.intervalMs) ||
            Object.keys(o).some((k) => !["kind", "intervalMs"].includes(k)))
            return protocol();
        outcome = { kind: "quality", intervalMs: o.intervalMs };
    }
    else if (o.kind === "codecs" || o.kind === "gates") {
        if (!Array.isArray(o.streams) ||
            Object.keys(o).some((k) => !(o.kind === "codecs"
                ? ["kind", "resourcesChanged", "streams"]
                : ["kind", "action", "streams"]).includes(k)))
            return protocol();
        const values = o.streams.map((s) => codec(s, v.connectionGeneration, o.kind === "gates"));
        if (o.kind === "codecs") {
            if (typeof o.resourcesChanged !== "boolean")
                return protocol();
            outcome = {
                kind: "codecs",
                resourcesChanged: o.resourcesChanged,
                streams: values,
            };
        }
        else {
            if (typeof o.action !== "string" ||
                !["hold", "resume", "mute", "unmute"].includes(o.action))
                return protocol();
            outcome = {
                kind: "gates",
                action: o.action,
                streams: values,
            };
        }
    }
    else
        return protocol();
    return Object.freeze({
        connection,
        connectionGeneration: v.connectionGeneration,
        streamRevision: v.streamRevision,
        outcome: frozen(outcome),
    });
}
export function streamSelection(v, ctx, id) {
    if (!object(v, [
        "connection",
        "connectionGeneration",
        "streamRevision",
        "resourcesChanged",
        "bindings",
    ]) ||
        !positive(v.connectionGeneration) ||
        !positive(v.streamRevision) ||
        typeof v.resourcesChanged !== "boolean" ||
        (v.bindings !== null && !Array.isArray(v.bindings)))
        return protocol();
    return Object.freeze({
        connection: connectionHandle(v.connection, ctx, id),
        connectionGeneration: v.connectionGeneration,
        streamRevision: v.streamRevision,
        resourcesChanged: v.resourcesChanged,
        bindings: v.bindings === null
            ? null
            : frozen(v.bindings.map((s) => codec(s, v.connectionGeneration))),
    });
}
