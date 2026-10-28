import type { CanonicalProblem } from "./canonical_error.js";
/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export declare class ConnectError extends Error {
    readonly code: string;
    readonly problem?: CanonicalProblem | undefined;
    constructor(code: string, message: string, problem?: CanonicalProblem | undefined);
}
