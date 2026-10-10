import { Operations } from "./operations.js";
import type { Operation, Outcome } from "./types.js";
/** Rust owns TLS/auth/UCTP/QUIC; helper failure closes this client. */
export declare class NativeClient extends Operations {
    private readonly child;
    private readonly exited;
    private pending?;
    private buffer;
    private closed;
    private chain;
    private readonly timeoutMs;
    constructor(config: string, executable?: string, timeoutMs?: number);
    private fail;
    private receive;
    request(operation: Operation, taskBinding?: string): Promise<Outcome>;
    close(): Promise<void>;
}
