// One Connection's packet tasks; the shared SDK/helper remains the caller's.
import assert from "node:assert/strict";

export class ConnectionMediaOwner {
    #controller = new AbortController();
    #parent;
    #abort;
    #tasks = [];
    #join;
    #retirement;
    #retiredRefusals = 0;

    constructor(parent) {
        this.#parent = parent;
        this.#abort = () => {
            this.#controller.abort();
            this.#retirement?.settle(false);
        };
        parent.addEventListener("abort", this.#abort, { once: true });
        if (parent.aborted) this.#abort();
    }

    get signal() { return this.#controller.signal; }

    expectRetirement({ operationId, connectionId, generation }) {
        assert.equal(this.signal.aborted, false);
        assert.equal(this.#retirement, undefined, "one original retirement per physical owner");
        assert.ok([operationId, connectionId].every(v => typeof v === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(v)));
        assert.ok(Number.isSafeInteger(generation) && generation > 0);
        let settle;
        const observed = new Promise(done => { settle = done; });
        this.#retirement = { operationId, connectionId, generation, observed, settle, confirmed: false };
        return proof => {
            const expected = this.#retirement;
            assert.equal(this.signal.aborted, false);
            assert.equal(proof.reference.command, "connection.replace");
            assert.equal(proof.reference.operation_id, expected.operationId);
            assert.equal(proof.reference.scope.connection_id, expected.connectionId);
            assert.equal(proof.original.id, expected.connectionId);
            assert.equal(proof.original.state, "retired");
            assert.ok(proof.original.generation > expected.generation);
            assert.deepEqual(proof.movement.retired, [expected.connectionId]);
            expected.confirmed = true;
            expected.settle(true);
        };
    }

    run(work) {
        assert.equal(this.signal.aborted, false, "retired Connection cannot acquire new packet tasks");
        const task = Promise.resolve().then(() => work(this.signal)).catch(failure => {
            if (this.signal.aborted && failure.kind === "cancelled") return;
            // Only an explicit read-only refusal can wait for the exact root's
            // completed retirement proof. Uncertain writes always fail intact.
            const retirement = this.#retirement;
            if (retirement && failure.certainty === "read-only" &&
                ["resync-required", "not-found", "cancelled"].includes(failure.kind)) {
                return retirement.observed.then(confirmed => {
                    if (!confirmed) throw failure;
                    this.#retiredRefusals++;
                });
            }
            throw failure;
        });
        this.#tasks.push(task);
        return task;
    }

    stopAndJoin() {
        return this.#join ??= (async () => {
            this.#controller.abort();
            if (!this.#retirement?.confirmed) this.#retirement?.settle(false);
            try {
                const results = await Promise.allSettled(this.#tasks);
                const failed = results.find(result => result.status === "rejected");
                if (failed) throw failed.reason;
                return Object.freeze({ original_packet_tasks_joined: this.#tasks.length,
                    shared_sdk_closed: false, confirmed_retirement_refusals: this.#retiredRefusals });
            } finally { this.#parent.removeEventListener("abort", this.#abort); }
        })();
    }
}
