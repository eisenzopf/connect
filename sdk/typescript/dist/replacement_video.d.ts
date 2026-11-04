import type { JsonObject } from "./uctp_json.js";
export interface ReplacementVideoStream {
    readonly previous: string;
    readonly replacement: string;
}
export interface ReplacementVideo {
    readonly publications: readonly ReplacementVideoStream[];
    readonly receivers: readonly ReplacementVideoStream[];
}
export declare function replacementVideo(value: ReplacementVideo, bad: () => never): JsonObject;
