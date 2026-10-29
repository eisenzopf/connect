import { canonicalJson, jsonObject } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export function fields(value, allowed, bad) {
    if (!value ||
        typeof value !== "object" ||
        Object.keys(value).some((k) => !allowed.includes(k)))
        bad();
}
export function nonEmpty(input, check, maximum, bad) {
    if (!input ||
        typeof input !== "object" ||
        !Array.isArray(input.rest) ||
        input.rest.length >= maximum ||
        !check(input.first) ||
        input.rest.some((v) => !check(v)))
        return bad();
    fields(input, ["first", "rest"], bad);
    return [input.first, ...input.rest];
}
const text = (v, max) => typeof v === "string" &&
    !!v &&
    new TextEncoder().encode(v).length <= max &&
    !/[\x00-\x1f\x7f-\x9f]/u.test(v);
const codec = (v) => typeof v === "string" && /^[A-Za-z0-9._/-]{1,64}$/.test(v);
export const sessionMedium = (v) => typeof v === "string" &&
    [
        "voice",
        "video",
        "voice+video",
        "screen-share",
        "text-chat",
        "mixed",
    ].includes(v);
export function sessionCapabilities(value, bad) {
    if (!jsonObject(value) ||
        new TextEncoder().encode(canonicalJson(value)).length > 16384)
        return bad();
    for (const kind of ["audio", "video", "screen", "data"]) {
        const names = value[kind];
        if (names !== undefined &&
            (!Array.isArray(names) ||
                !names.length ||
                names.length > 16 ||
                names.some((v) => !codec(v)) ||
                new Set(names).size !== names.length))
            return bad();
    }
    return value;
}
export function sessionInviteBody(input, bad) {
    if (!input ||
        !Array.isArray(input.to) ||
        input.to.length > 256 ||
        input.to.some((v) => !identifier(v)) ||
        new Set(input.to).size !== input.to.length ||
        !sessionMedium(input.medium) ||
        !text(input.intent, 128))
        return bad();
    return {
        to: [...input.to],
        medium: input.medium,
        intent: input.intent,
        capabilities_offer: sessionCapabilities(input.capabilities, bad),
    };
}
export function sessionUpdateBody(input, bad) {
    if (!input || typeof input !== "object")
        return bad();
    switch (input.action) {
        case "participant-leave":
            fields(input, ["action", "participantId"], bad);
            if (!identifier(input.participantId))
                return bad();
            return {
                kind: input.action,
                details: { participant_id: input.participantId },
            };
        case "role-change":
            fields(input, ["action", "participantId", "role"], bad);
            if (!identifier(input.participantId) || !text(input.role, 64))
                return bad();
            return {
                kind: input.action,
                details: {
                    participant_id: input.participantId,
                    role: input.role,
                },
            };
        case "medium-upgrade": {
            fields(input, ["action", "media"], bad);
            const media = nonEmpty(input.media, (v) => typeof v === "string" &&
                ["audio", "video", "screen", "data"].includes(v), 4, bad);
            if (new Set(media).size !== media.length)
                return bad();
            return { kind: input.action, details: { media } };
        }
        case "codec-renegotiate": {
            fields(input, ["action", "codecPreferences"], bad);
            const names = nonEmpty(input.codecPreferences, codec, 16, bad);
            if (new Set(names).size !== names.length)
                return bad();
            return {
                kind: input.action,
                details: { codec_preferences: names },
            };
        }
        default:
            return bad();
    }
}
export function offeredStreams(input, bad) {
    const values = nonEmpty(input, (v) => !!v && typeof v === "object", 8, bad);
    if (new Set(values.map((v) => v.id)).size !== values.length)
        return bad();
    return values.map((v) => {
        fields(v, ["id", "kind", "direction", "codecs"], bad);
        if (!identifier(v.id) ||
            !["audio", "video", "screen", "data"].includes(v.kind) ||
            !["sendrecv", "sendonly", "recvonly", "inactive"].includes(v.direction))
            return bad();
        const codecs = nonEmpty(v.codecs, codec, 16, bad);
        if (new Set(codecs).size !== codecs.length)
            return bad();
        return { ...v, codecs };
    });
}
