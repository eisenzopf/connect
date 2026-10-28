import type { CanonicalRequestOptions } from "./canonical_validation.js";
export declare function readerOptions(options: CanonicalRequestOptions): void;
export declare function readerWait<T>(promise: Promise<T>, expires: number, signal: AbortSignal | undefined, closed: AbortSignal): Promise<T>;
