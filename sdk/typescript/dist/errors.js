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
