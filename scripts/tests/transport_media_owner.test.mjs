import assert from "node:assert/strict";
import test from "node:test";
import { ConnectionMediaOwner } from "../lib/transport_media_owner.mjs";

test("retiring one Connection joins its four tasks while the same participant's second Session remains live", async () => {
    const workload = new AbortController();
    const first = new ConnectionMediaOwner(workload.signal), second = new ConnectionMediaOwner(workload.signal);
    const joined = [0, 0], started = [0, 0];
    for (const [index, owner] of [first, second].entries()) for (let task = 0; task < 4; task++) {
        owner.run(signal => new Promise((resolve, reject) => {
            started[index]++;
            const stop = () => { joined[index]++; reject(Object.assign(Error("owned read/write cancelled"), { kind: "cancelled" })); };
            if (signal.aborted) stop(); else signal.addEventListener("abort", stop, { once: true });
        }));
    }
    await Promise.resolve();
    assert.deepEqual(started, [4, 4]);
    const a = first.stopAndJoin(), b = first.stopAndJoin();
    assert.equal(a, b);
    assert.deepEqual(await a, { original_packet_tasks_joined: 4, shared_sdk_closed: false });
    assert.deepEqual(joined, [4, 0]);
    assert.equal(workload.signal.aborted, false);
    assert.equal(second.signal.aborted, false);
    assert.throws(() => first.run(async () => {}));
    workload.abort();
    await second.stopAndJoin();
    assert.deepEqual(joined, [4, 4]);
});

test("Connection retirement never hides an unknown mutation or authority failure", async () => {
    const parent = new AbortController(), owner = new ConnectionMediaOwner(parent.signal);
    const unknown = Object.assign(Error("original effect uncertainty"), { kind: "authority", certainty: "uncertain" });
    const task = owner.run(async () => { throw unknown; });
    task.catch(() => {});
    await assert.rejects(owner.stopAndJoin(), error => error === unknown);
    assert.equal(parent.signal.aborted, false);
});
