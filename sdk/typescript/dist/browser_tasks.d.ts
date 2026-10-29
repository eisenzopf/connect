/** Internal ownership join. Task failure is observed by its original consumer;
 * retirement waits for settlement, including tasks admitted by closing tasks. */
export declare class BrowserTasks {
    private readonly pending;
    private failure?;
    track<T>(work: Promise<T>, cleanup?: boolean): Promise<T>;
    join(): Promise<void>;
}
