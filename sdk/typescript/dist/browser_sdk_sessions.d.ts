import type { Json, JsonObject } from "./uctp_json.js";
import type { ParticipantId, BrowserOfferedStream } from "./browser_sdk.js";
import type { BrowserNonEmpty, BrowserMedium } from "./browser_sdk_streams.js";
export type BrowserSessionMedium = "voice" | "video" | "voice+video" | "screen-share" | "text-chat" | "mixed";
export type BrowserSessionUpdate = {
    readonly action: "participant-leave";
    readonly participantId: ParticipantId;
} | {
    readonly action: "role-change";
    readonly participantId: ParticipantId;
    readonly role: string;
} | {
    readonly action: "medium-upgrade";
    readonly media: BrowserNonEmpty<BrowserMedium>;
} | {
    readonly action: "codec-renegotiate";
    readonly codecPreferences: BrowserNonEmpty<string>;
};
export declare function fields(value: object, allowed: readonly string[], bad: () => never): void;
export declare function nonEmpty<T>(input: BrowserNonEmpty<T>, check: (v: unknown) => boolean, maximum: number, bad: () => never): T[];
export declare const sessionMedium: (v: unknown) => v is BrowserSessionMedium;
export declare function sessionCapabilities(value: Json, bad: () => never): Json;
export declare function sessionInviteBody(input: {
    readonly to: readonly ParticipantId[];
    readonly medium: BrowserSessionMedium;
    readonly intent: string;
    readonly capabilities: Json;
}, bad: () => never): JsonObject;
export declare function sessionUpdateBody(input: BrowserSessionUpdate, bad: () => never): JsonObject;
export declare function offeredStreams(input: BrowserNonEmpty<BrowserOfferedStream>, bad: () => never): {
    codecs: string[];
    id: import("./sdk_identity.js").StreamId;
    kind: "audio" | "video" | "screen" | "data";
    direction: "sendonly" | "recvonly" | "sendrecv" | "inactive";
}[];
