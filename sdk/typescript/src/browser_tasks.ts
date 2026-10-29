/** Internal ownership join. Task failure is observed by its original consumer;
 * retirement waits for settlement, including tasks admitted by closing tasks. */
export class BrowserTasks {
    private readonly pending = new Set<Promise<void>>();
    private failure?: { error: unknown };
    track<T>(work: Promise<T>, cleanup = false): Promise<T> {
        const settled = work.then(
            () => {},
            (error: unknown) => {
                if (cleanup) this.failure ??= { error };
            },
        );
        this.pending.add(settled);
        void settled.then(() => this.pending.delete(settled));
        return work;
    }
    async join(): Promise<void> {
        while (this.pending.size) await Promise.all([...this.pending]);
        if (this.failure) throw this.failure.error;
    }
}
