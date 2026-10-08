/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export class ConnectError extends Error {
    code;
    constructor(code, message) {
        super(message);
        this.code = code;
        this.name = "ConnectError";
    }
}
