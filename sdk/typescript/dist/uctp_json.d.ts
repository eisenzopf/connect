export type Json = null | boolean | number | string | Json[] | {
    [key: string]: Json;
};
export type JsonObject = {
    [key: string]: Json;
};
export declare const MAX_ENVELOPE_BYTES: number;
/** RFC 8785: UTF-16 key ordering, finite ECMAScript numbers, scalar Unicode. */
export declare function canonicalJson(value: Json): string;
/** Reject duplicate decoded keys before JSON.parse can erase signature input. */
export declare function parseCanonicalJson(bytes: Uint8Array, maximumBytes?: number): Json;
export declare function jsonObject(value: Json | undefined): value is JsonObject;
