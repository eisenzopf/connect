import { Operations } from "./operations.js";
import type { Operation, Outcome } from "./types.js";
/** Explicit loopback HTTP facade. No redirects or automatic mutation retries. */
export declare class HttpClient extends Operations {
    private readonly credential;
    private readonly url;
    private readonly timeoutMs;
    constructor(url: string, credential: string, timeoutMs?: number);
    request(operation: Operation): Promise<Outcome>;
}
