/** Internal ownership join. Task failure is observed by its original consumer;
 * retirement waits for settlement, including tasks admitted by closing tasks. */
export class BrowserTasks {
    pending = new Set();
    failure;
    track(work, cleanup = false) {
        const settled = work.then(() => { }, (error) => {
            if (cleanup)
                this.failure ??= { error };
        });
        this.pending.add(settled);
        void settled.then(() => this.pending.delete(settled));
        return work;
    }
    async join() {
        while (this.pending.size)
            await Promise.all([...this.pending]);
        if (this.failure)
            throw this.failure.error;
    }
}
