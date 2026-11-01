// One Connection's packet tasks; the shared SDK/helper remains the caller's.
import assert from "node:assert/strict";

export class ConnectionMediaOwner {
    #controller = new AbortController();
    #parent;
    #abort;
    #tasks = [];
    #join;

    constructor(parent) {
        this.#parent = parent;
        this.#abort = () => this.#controller.abort();
        parent.addEventListener("abort", this.#abort, { once: true });
        if (parent.aborted) this.#abort();
    }

    get signal() { return this.#controller.signal; }

    run(work) {
        assert.equal(this.signal.aborted, false, "retired Connection cannot acquire new packet tasks");
        const task = Promise.resolve().then(() => work(this.signal)).catch(failure => {
            if (this.signal.aborted && failure.kind === "cancelled") return;
            throw failure;
        });
        this.#tasks.push(task);
        return task;
    }

    stopAndJoin() {
        return this.#join ??= (async () => {
            this.#controller.abort();
            try {
                const results = await Promise.allSettled(this.#tasks);
                const failed = results.find(result => result.status === "rejected");
                if (failed) throw failed.reason;
                return Object.freeze({ original_packet_tasks_joined: this.#tasks.length,
                    shared_sdk_closed: false });
            } finally { this.#parent.removeEventListener("abort", this.#abort); }
        })();
    }
}
