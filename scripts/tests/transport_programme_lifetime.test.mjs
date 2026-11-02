import assert from "node:assert/strict";
import test from "node:test";
import { joinMediaProgramme } from "../lib/transport_programme_lifetime.mjs";

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
function fixture({ control = deferred(), stop = () => {} } = {}) {
    let time = 0, next = 0, stops = 0;
    const controller = new AbortController(), timers = new Map(), packet = deferred();
    const run = joinMediaProgramme({ startedAt: 0, signal: controller.signal, abort: reason => controller.abort(reason),
        owners: [{ async stopAndJoin() { stops++; await stop(); packet.resolve(); } }], packetTasks: [packet.promise], controlTask: control.promise,
        now: () => time, setTimer: (fn, ms) => { const id = ++next; timers.set(id, { at: time + ms, fn }); return id; },
        clearTimer: id => timers.delete(id) });
    let complete = false;
    run.then(() => { complete = true; }, () => { complete = true; });
    return { run, controller, control, packet, timers, get stops() { return stops; }, get complete() { return complete; },
        async advance(at) {
            time = at;
            for (;;) {
                const [id, timer] = [...timers].sort((a, b) => a[1].at - b[1].at)[0] ?? [];
                if (!timer || timer.at > time) break;
                timers.delete(id); timer.fn(); await flush();
            }
            await flush();
        } };
}

test("normal62s packet window joins final captures without cancelling their control owner", async () => {
    const f = fixture();
    await f.advance(62199); assert.equal(f.stops, 0);
    await f.advance(62200); assert.equal(f.stops, 1); assert.equal(f.complete, false);
    assert.equal(f.controller.signal.aborted, false);
    await f.advance(62500); f.control.resolve(); await f.run;
    assert.equal(f.controller.signal.aborted, false); assert.equal(f.timers.size, 0);
});

test("early control completion cannot shorten the original packet observation", async () => {
    const f = fixture(); f.control.resolve(); await flush();
    assert.equal(f.complete, false); assert.equal(f.stops, 0);
    await f.advance(62200); await f.run; assert.equal(f.stops, 1); assert.equal(f.timers.size, 0);
});

test("a parent cancellation remains a control failure after completed roots", async () => {
    const f = fixture(), original = Object.assign(Error("parent cancelled after20roots"), { kind: "cancelled" });
    f.controller.abort(original); f.control.reject(original);
    await assert.rejects(f.run, error => error === original);
    assert.equal(f.stops, 1); assert.equal(f.controller.signal.reason, original); assert.equal(f.timers.size, 0);
});

test("a failed original packet stops and joins siblings before returning its exact error", async () => {
    const joined = deferred(), f = fixture({ stop: () => joined.promise });
    const original = Error("original packet refused"); f.packet.reject(original); await flush();
    assert.equal(f.controller.signal.reason, original); assert.equal(f.complete, false); assert.equal(f.stops, 1);
    f.control.resolve(); joined.resolve(); await assert.rejects(f.run, error => error === original);
    assert.equal(f.timers.size, 0);
});

test("one absolute5s join bound cancels overdue control but retains ownership until it joins", async () => {
    const f = fixture(); await f.advance(62200); await f.advance(66999);
    assert.equal(f.controller.signal.aborted, false); assert.equal(f.complete, false);
    await f.advance(67000); assert.equal(f.controller.signal.reason.code, "media-programme-join-deadline");
    assert.equal(f.complete, false, "a timeout cannot abandon the outstanding original control owner");
    f.control.resolve(); await assert.rejects(f.run, error => error.code === "media-programme-join-deadline");
    assert.equal(f.stops, 1); assert.equal(f.timers.size, 0);
});

test("control failure joins every original packet owner and never substitutes deadline success", async () => {
    const f = fixture(), original = Object.assign(Error("original move uncertain"), { certainty: "uncertain" });
    f.control.reject(original); await assert.rejects(f.run, error => error === original);
    assert.equal(f.stops, 1); assert.equal(f.controller.signal.reason, original); assert.equal(f.timers.size, 0);
});

test("a parent cancellation cannot become success when control already completed", async () => {
    const f = fixture(), original = Error("cancelled completed-root workload");
    f.control.resolve(); f.controller.abort(original);
    await assert.rejects(f.run, error => error === original);
    assert.equal(f.stops, 1); assert.equal(f.timers.size, 0);
});

test("failed packet-owner joining cancels control immediately and retains both owners until joined", async () => {
    const original = Error("original packet owner join refused"), f = fixture({ stop: () => { throw original; } });
    await f.advance(62200); assert.equal(f.controller.signal.reason, original); assert.equal(f.complete, false);
    f.packet.resolve(); f.control.resolve(); await assert.rejects(f.run, error => error === original);
    assert.equal(f.stops, 1); assert.equal(f.timers.size, 0);
});

test("early cancellation narrows cleanup to5s and a late one cannot renew the original absolute bound", async () => {
    for (const [at, until] of [[1000, 6000], [66000, 67000]]) {
        const f = fixture(), original = Error("original cancellation");
        await f.advance(at); f.controller.abort(original); await flush();
        assert.ok([...f.timers.values()].some(t => t.at === until));
        await f.advance(until); assert.equal(f.complete, false);
        assert.equal(f.controller.signal.reason, original);
        f.control.resolve(); await assert.rejects(f.run, error => error === original);
        assert.equal(f.timers.size, 0);
    }
});
