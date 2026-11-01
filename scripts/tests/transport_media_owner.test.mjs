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
    assert.deepEqual(await a, { original_packet_tasks_joined: 4, shared_sdk_closed: false, confirmed_retirement_refusals: 0 });
    assert.deepEqual(joined, [4, 0]);
    assert.equal(workload.signal.aborted, false);
    assert.equal(second.signal.aborted, false);
    assert.throws(() => first.run(async () => {}));
    workload.abort();
    await second.stopAndJoin();
    assert.deepEqual(joined, [4, 4]);
});

const retirement = () => ({ reference: { command: "connection.replace", operation_id: "original-move",
    scope: { connection_id: "old-connection" } }, original: { id: "old-connection", state: "retired", generation: 2 },
    movement: { retired: ["old-connection"] } });

test("read-only retirement refusal waits for the same completed original and newer retired incarnation", async () => {
    const parent = new AbortController(), owner = new ConnectionMediaOwner(parent.signal);
    const confirm = owner.expectRetirement({ operationId: "original-move", connectionId: "old-connection", generation: 1 });
    const failure = Object.assign(Error("retired binding"), { kind: "resync-required", certainty: "read-only" });
    let settled = false;
    const task = owner.run(async () => { throw failure; }).then(() => { settled = true; });
    await new Promise(setImmediate);
    assert.equal(settled, false);
    for (const change of [p => { p.reference.operation_id = "another-move"; },
        p => { p.reference.scope.connection_id = "another-connection"; },
        p => { p.original.state = "retiring"; }, p => { p.original.generation = 1; },
        p => { p.movement.retired = ["another-connection"]; }]) {
        const proof = retirement(); change(proof); assert.throws(() => confirm(proof));
        assert.equal(settled, false);
    }
    confirm(retirement()); await task;
    assert.equal(settled, true);
    assert.equal((await owner.stopAndJoin()).confirmed_retirement_refusals, 1);
    assert.equal(parent.signal.aborted, false);
});

test("parent stop or unconfirmed join preserves the original refusal without an abandoned proof wait", async () => {
    for (const parentStop of [false, true]) {
        const parent = new AbortController(), owner = new ConnectionMediaOwner(parent.signal);
        owner.expectRetirement({ operationId: "original-move", connectionId: "old-connection", generation: 1 });
        const failure = Object.assign(Error("original read refusal"), { kind: "resync-required", certainty: "read-only" });
        const task = owner.run(async () => { throw failure; }); task.catch(() => {});
        await new Promise(setImmediate);
        if (parentStop) parent.abort();
        await assert.rejects(owner.stopAndJoin(), error => error === failure);
    }
});

test("an expected retirement cannot hide uncertain IO or an unrelated authority failure", async () => {
    for (const [kind, certainty] of [["resync-required", "uncertain"], ["authority", "read-only"]]) {
        const parent = new AbortController(), owner = new ConnectionMediaOwner(parent.signal);
        const confirm = owner.expectRetirement({ operationId: "original-move", connectionId: "old-connection", generation: 1 });
        const failure = Object.assign(Error("original failure"), { kind, certainty });
        const task = owner.run(async () => { throw failure; }); task.catch(() => {});
        await assert.rejects(task, error => error === failure);
        confirm(retirement());
        await assert.rejects(owner.stopAndJoin(), error => error === failure);
    }
});

test("Connection retirement never hides an unknown mutation or authority failure", async () => {
    const parent = new AbortController(), owner = new ConnectionMediaOwner(parent.signal);
    const unknown = Object.assign(Error("original effect uncertainty"), { kind: "authority", certainty: "uncertain" });
    const task = owner.run(async () => { throw unknown; });
    task.catch(() => {});
    await assert.rejects(owner.stopAndJoin(), error => error === unknown);
    assert.equal(parent.signal.aborted, false);
});
