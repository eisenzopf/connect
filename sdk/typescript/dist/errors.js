/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export class ConnectError extends Error {
    code;
    problem;
    httpStatus;
    constructor(code, message, problem, httpStatus) {
        super(message);
        this.code = code;
        this.problem = problem;
        this.httpStatus = httpStatus;
        this.name = "ConnectError";
    }
}
/** Exact signed unavailable refusals; private text never selects recovery. */
export function recoverableReadRefusal(error) {
    if (!(error instanceof ConnectError) || error.code !== "refused")
        return false;
    const problem = error.problem;
    return problem?.code === 503 &&
        ((problem.category === "authorization" && problem.reason === "authority_unavailable") ||
            (problem.category === "storage" && problem.reason === "storage_unavailable"));
}
