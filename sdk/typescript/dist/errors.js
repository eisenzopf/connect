/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export class ConnectError extends Error {
    code;
    problem;
    constructor(code, message, problem) {
        super(message);
        this.code = code;
        this.problem = problem;
        this.name = "ConnectError";
    }
}
