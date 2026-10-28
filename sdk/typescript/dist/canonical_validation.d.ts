import { type Envelope, type Scope } from "./uctp_wire.js";
import { CanonicalError, type CanonicalProblem, type OperationKey } from "./canonical_error.js";
export interface CanonicalRequestOptions {
    readonly timeoutMs: number;
    readonly signal?: AbortSignal;
}
export type CanonicalCommandScope = Omit<Scope, "in_reply_to">;
export declare const maximumRequest = 65536, maximumResponse: number;
export declare const reads: Set<string>;
export declare const commands: Set<string>;
export declare const topFields: Set<string>;
export declare function duration(options: CanonicalRequestOptions): void;
export declare function problem(value: unknown): CanonicalProblem;
export declare function refused(p: CanonicalProblem, key?: OperationKey): CanonicalError;
export declare function checkScope(frame: Envelope, scope: CanonicalCommandScope, assigned: boolean): void;
