/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export class ConnectError extends Error {
    constructor(
        public readonly code: string,
        message: string,
    ) {
        super(message);
        this.name = "ConnectError";
    }
}
