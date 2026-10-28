import { CanonicalError, type CanonicalErrorKind, type OperationKey } from "./canonical_error.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { type CanonicalRequestOptions } from "./canonical_validation.js";
export declare function object(value: unknown, fields: readonly string[]): value is JsonObject;
export declare class NodeSdkError extends CanonicalError {
    readonly observation?: Json | undefined;
    constructor(kind: CanonicalErrorKind, certainty: CanonicalError["certainty"], operation?: OperationKey, problem?: CanonicalError["problem"], observation?: Json | undefined);
}
/** Internal framing only. The Rust SDK owns trust, journals and typed behavior. */
export declare class SdkHelper {
    private readonly child;
    private readonly exited;
    private joined;
    private closed;
    private readonly queue;
    private active?;
    private chunks;
    private bytes;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    });
    request<T>(action: JsonObject, options: CanonicalRequestOptions, decode: (value: Json) => T, key?: OperationKey, mutation?: boolean): Promise<T>;
    private error;
    private release;
    private cancel;
    private next;
    fence(kind: CanonicalErrorKind): void;
    private receive;
    close(options: CanonicalRequestOptions): Promise<void>;
}
