import type { Json, JsonObject } from "./uctp_json.js";
import { canonicalJson, jsonObject } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import type { ParticipantId, BrowserOfferedStream } from "./browser_sdk.js";
import type { BrowserNonEmpty, BrowserMedium } from "./browser_sdk_streams.js";

export type BrowserSessionMedium =
    "voice" | "video" | "voice+video" | "screen-share" | "text-chat" | "mixed";
export type BrowserSessionUpdate =
    | {
          readonly action: "participant-leave";
          readonly participantId: ParticipantId;
      }
    | {
          readonly action: "role-change";
          readonly participantId: ParticipantId;
          readonly role: string;
      }
    | {
          readonly action: "medium-upgrade";
          readonly media: BrowserNonEmpty<BrowserMedium>;
      }
    | {
          readonly action: "codec-renegotiate";
          readonly codecPreferences: BrowserNonEmpty<string>;
      };

export function fields(
    value: object,
    allowed: readonly string[],
    bad: () => never,
): void {
    if (
        !value ||
        typeof value !== "object" ||
        Object.keys(value).some((k) => !allowed.includes(k))
    )
        bad();
}
export function nonEmpty<T>(
    input: BrowserNonEmpty<T>,
    check: (v: unknown) => boolean,
    maximum: number,
    bad: () => never,
): T[] {
    if (
        !input ||
        typeof input !== "object" ||
        !Array.isArray(input.rest) ||
        input.rest.length >= maximum ||
        !check(input.first) ||
        input.rest.some((v) => !check(v))
    )
        return bad();
    fields(input, ["first", "rest"], bad);
    return [input.first, ...input.rest];
}
const text = (v: unknown, max: number): v is string =>
    typeof v === "string" &&
    !!v &&
    new TextEncoder().encode(v).length <= max &&
    !/[\x00-\x1f\x7f-\x9f]/u.test(v);
const codec = (v: unknown): v is string =>
    typeof v === "string" && /^[A-Za-z0-9._/-]{1,64}$/.test(v);
export const sessionMedium = (v: unknown): v is BrowserSessionMedium =>
    typeof v === "string" &&
    [
        "voice",
        "video",
        "voice+video",
        "screen-share",
        "text-chat",
        "mixed",
    ].includes(v);
export function sessionCapabilities(value: Json, bad: () => never): Json {
    if (
        !jsonObject(value) ||
        new TextEncoder().encode(canonicalJson(value)).length > 16384
    )
        return bad();
    for (const kind of ["audio", "video", "screen", "data"]) {
        const names = value[kind];
        if (
            names !== undefined &&
            (!Array.isArray(names) ||
                !names.length ||
                names.length > 16 ||
                names.some((v) => !codec(v)) ||
                new Set(names).size !== names.length)
        )
            return bad();
    }
    return value;
}
export function sessionInviteBody(
    input: {
        readonly to: readonly ParticipantId[];
        readonly medium: BrowserSessionMedium;
        readonly intent: string;
        readonly capabilities: Json;
    },
    bad: () => never,
): JsonObject {
    if (
        !input ||
        !Array.isArray(input.to) ||
        input.to.length > 256 ||
        input.to.some((v) => !identifier(v)) ||
        new Set(input.to).size !== input.to.length ||
        !sessionMedium(input.medium) ||
        !text(input.intent, 128)
    )
        return bad();
    return {
        to: [...input.to],
        medium: input.medium,
        intent: input.intent,
        capabilities_offer: sessionCapabilities(input.capabilities, bad),
    };
}
export function sessionUpdateBody(
    input: BrowserSessionUpdate,
    bad: () => never,
): JsonObject {
    if (!input || typeof input !== "object") return bad();
    switch (input.action) {
        case "participant-leave":
            fields(input, ["action", "participantId"], bad);
            if (!identifier(input.participantId)) return bad();
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
            const media = nonEmpty(
                input.media,
                (v) =>
                    typeof v === "string" &&
                    ["audio", "video", "screen", "data"].includes(v),
                4,
                bad,
            );
            if (new Set(media).size !== media.length) return bad();
            return { kind: input.action, details: { media } };
        }
        case "codec-renegotiate": {
            fields(input, ["action", "codecPreferences"], bad);
            const names = nonEmpty(input.codecPreferences, codec, 16, bad);
            if (new Set(names).size !== names.length) return bad();
            return {
                kind: input.action,
                details: { codec_preferences: names },
            };
        }
        default:
            return bad();
    }
}
export function offeredStreams(
    input: BrowserNonEmpty<BrowserOfferedStream>,
    bad: () => never,
) {
    const values = nonEmpty(input, (v) => !!v && typeof v === "object", 8, bad);
    if (new Set(values.map((v) => v.id)).size !== values.length) return bad();
    return values.map((v) => {
        fields(v, ["id", "kind", "direction", "codecs"], bad);
        if (
            !identifier(v.id) ||
            !["audio", "video", "screen", "data"].includes(v.kind) ||
            !["sendrecv", "sendonly", "recvonly", "inactive"].includes(
                v.direction,
            )
        )
            return bad();
        const codecs = nonEmpty(v.codecs, codec, 16, bad);
        if (new Set(codecs).size !== codecs.length) return bad();
        return { ...v, codecs };
    });
}
