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

/** Exact signed unavailable refusals; private text never selects recovery. */
export function recoverableReadRefusal(error: unknown): boolean {
    if (!(error instanceof ConnectError) || error.code !== "refused") return false;
    const problem = error.problem;
    return problem?.code === 503 &&
        ((problem.category === "authorization" && problem.reason === "authority_unavailable") ||
         (problem.category === "storage" && problem.reason === "storage_unavailable"));
}
