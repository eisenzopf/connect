import type { CanonicalProblem } from "./canonical_error.js";

/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export class ConnectError extends Error {
    constructor(
        public readonly code: string,
        message: string,
        public readonly problem?: CanonicalProblem,
        public readonly httpStatus?: number,
    ) {
        super(message);
        this.name = "ConnectError";
    }
}
