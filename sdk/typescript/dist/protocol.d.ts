import type { Operation, Outcome } from "./types.js";
export declare const MAX_REQUEST = 65537;
export declare const MAX_RESPONSE = 600000;
export declare function expectedKind(operation: Operation): Outcome["kind"];
export declare function timeoutValue(ms: number): number;
export declare function decodeResponse(value: unknown, expected: Outcome["kind"]): Outcome;
export declare function parseJson(text: string): unknown;
export declare function helperResponse(text: string, id: string, expected: Outcome["kind"]): Outcome;
