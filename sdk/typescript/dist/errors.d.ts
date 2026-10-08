/** Stable error codes. Uncertainty never authorizes a new mutation ID. */
export declare class ConnectError extends Error {
    readonly code: string;
    constructor(code: string, message: string);
}
