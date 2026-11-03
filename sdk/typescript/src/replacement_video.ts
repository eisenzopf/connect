import { identifier } from "./uctp_wire.js";
import type { JsonObject } from "./uctp_json.js";
export interface ReplacementVideoStream {
    readonly previous: string;
    readonly replacement: string;
}
export interface ReplacementVideo {
    readonly publications: readonly ReplacementVideoStream[];
    readonly receivers: readonly ReplacementVideoStream[];
}
export function replacementVideo(
    value: ReplacementVideo,
    bad: () => never,
): JsonObject {
    const closed = (v: unknown, keys: string[]): v is Record<string, unknown> =>
        !!v &&
        typeof v === "object" &&
        !Array.isArray(v) &&
        Object.keys(v).length === keys.length &&
        Object.keys(v).every((k) => keys.includes(k));
    if (
        !closed(value, ["publications", "receivers"]) ||
        !Array.isArray(value.publications) ||
        !Array.isArray(value.receivers) ||
        value.publications.length + value.receivers.length < 1 ||
        value.publications.length + value.receivers.length > 32
    )
        return bad();
    const targets = new Set<string>();
    const group = (items: readonly ReplacementVideoStream[]) => {
        const previous = new Set<string>();
        return items.map((v) => {
            if (
                !closed(v, ["previous", "replacement"]) ||
                !identifier(v.previous) ||
                !identifier(v.replacement) ||
                previous.has(v.previous) ||
                targets.has(v.replacement)
            )
                return bad();
            previous.add(v.previous);
            targets.add(v.replacement);
            return { previous: v.previous, replacement: v.replacement };
        });
    };
    return {
        publications: group(value.publications),
        receivers: group(value.receivers),
    };
}
