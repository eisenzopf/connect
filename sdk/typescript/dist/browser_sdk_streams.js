import { canonicalJson, jsonObject, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
const positive = (v) => Number.isSafeInteger(v) && Number(v) > 0;
const medium = (v) => typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
const direction = (v) => typeof v === "string" && ["sendonly", "recvonly", "sendrecv"].includes(v);
const codecName = (v) => typeof v === "string" && /^[A-Za-z0-9._/-]{1,64}$/.test(v);
const object = (v) => jsonObject(v);
function freeze(v) {
    if (v && typeof v === "object") {
        for (const child of Object.values(v))
            freeze(child);
        Object.freeze(v);
    }
    return v;
}
function only(v, allowed, bad) {
    if (Object.keys(v).some((k) => !allowed.includes(k)))
        bad();
}
function nonempty(v, check, bad, maximum = 128) {
    if (!v ||
        typeof v !== "object" ||
        !Array.isArray(v.rest) ||
        v.rest.length >= maximum ||
        !check(v.first) ||
        v.rest.some((x) => !check(x)))
        return bad();
    only(v, ["first", "rest"], bad);
    return [v.first, ...v.rest];
}
function ids(v, bad, maximum = 128) {
    const values = nonempty(v, identifier, bad, maximum);
    if (new Set(values).size !== values.length)
        bad();
    return values;
}
export function subscribeBody(selectors, bad) {
    const seen = new Set();
    const subscriptions = nonempty(selectors, object, bad).map((v) => {
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
        const kinds = v.kinds === undefined
            ? undefined
            : nonempty(v.kinds, medium, bad, 4);
        if (kinds && new Set(kinds).size !== kinds.length)
            return bad();
        return {
            from_participant: v.participant,
            ...(kinds ? { kinds } : {}),
        };
    });
    return { subscriptions };
}
export function unsubscribeBody(streams, bad) {
    return { strm_ids: ids(streams, bad) };
}
export function updateBody(input, bad) {
    if (!input || typeof input !== "object")
        return bad();
    const streams = (v) => v === undefined ? {} : { streams: ids(v, bad, 32) };
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
            if (!Number.isInteger(input.intervalMs) ||
                input.intervalMs < 100 ||
                input.intervalMs > 60000)
                return bad();
            return {
                action: input.action,
                quality_interval_ms: input.intervalMs,
            };
        case "codec-renegotiate": {
            only(input, ["action", "streams", "codecPreferences"], bad);
            const codecs = nonempty(input.codecPreferences, codecName, bad, 32);
            if (new Set(codecs).size !== codecs.length)
                return bad();
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
function codecs(value, generation, bad, empty = false) {
    if (!Array.isArray(value) ||
        value.length > 32 ||
        (!empty && !value.length) ||
        new TextEncoder().encode(canonicalJson(value)).length > 32768)
        return bad();
    const names = new Set(), handles = new Set();
    return freeze(value.map((v) => {
        if (!object(v) ||
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
            handles.has(v.stream_local_id))
            return bad();
        names.add(v.stream_id);
        handles.add(v.stream_local_id);
        return {
            parameters: {
                id: v.stream_id,
                medium: v.medium,
                direction: v.direction,
                codec: v.codec,
                streamLocalId: v.stream_local_id,
            },
            streamGeneration: v.stream_generation,
            connectionGeneration: generation,
        };
    }));
}
export function selectionResult(value, key, connection, bad) {
    if (!object(value) ||
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
        !Object.hasOwn(value, "stream_bindings"))
        return bad();
    return freeze({
        connection,
        connectionGeneration: value.connection_generation,
        streamRevision: value.stream_revision,
        resourcesChanged: value.resources_changed,
        bindings: value.stream_bindings === null
            ? null
            : codecs(value.stream_bindings, value.connection_generation, bad, true),
    });
}
export function updateResult(value, key, connection, bad) {
    if (!object(value) ||
        value.connection_id !== key.scope.connectionId ||
        !positive(value.stream_revision))
        return bad();
    let generation, outcome;
    if (Object.hasOwn(value, "quality_interval_ms")) {
        if (!positive(value.connection_generation) ||
            !Number.isInteger(value.quality_interval_ms) ||
            Number(value.quality_interval_ms) < 100 ||
            Number(value.quality_interval_ms) > 60000 ||
            value.resources_changed !== false ||
            value.codec_bindings !== undefined ||
            value.gate_streams !== undefined)
            return bad();
        generation = value.connection_generation;
        outcome = {
            kind: "quality",
            intervalMs: Number(value.quality_interval_ms),
        };
    }
    else if (Object.hasOwn(value, "gate_streams")) {
        if (!positive(value.connection_generation) ||
            value.resources_changed !== false ||
            !Array.isArray(value.gate_streams) ||
            !value.gate_streams.length ||
            value.codec_bindings !== undefined ||
            !["hold", "resume", "mute", "unmute"].includes(String(value.action)))
            return bad();
        generation = value.connection_generation;
        const bindings = codecs(value.gate_streams.map((v) => (object(v) ? v.binding : null)), generation, bad);
        const streams = value.gate_streams.map((v, i) => {
            if (!object(v) ||
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
                    v.held_direction === null))
                return bad();
            return {
                ...bindings[i],
                state: v.state,
                sourceMuted: v.source_muted,
                ...(v.held_direction === null
                    ? {}
                    : {
                        heldDirection: v.held_direction,
                    }),
            };
        });
        outcome = {
            kind: "gates",
            action: value.action,
            streams: freeze(streams),
        };
    }
    else {
        if (!Array.isArray(value.codec_bindings) ||
            !value.codec_bindings.length ||
            typeof value.resources_changed !== "boolean")
            return bad();
        const first = value.codec_bindings[0];
        if (!object(first) || !positive(first.connection_generation))
            return bad();
        generation = first.connection_generation;
        if (value.connection_generation !== undefined &&
            value.connection_generation !== generation)
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
        streamRevision: value.stream_revision,
        outcome,
    });
}
