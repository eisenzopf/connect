/** One monotonic budget across capture, ICE, signaling and sender configuration. */
export declare class BrowserDeadline {
    private readonly expires;
    constructor(timeoutMs: number);
    remaining(): number;
    run<T>(operation: () => Promise<T>): Promise<T>;
}
