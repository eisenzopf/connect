import test from "node:test";
import assert from "node:assert/strict";
import { VideoPlayout } from "./ui/media-playout.mjs";

function fixture(onPaint) {
    let now = 0, next = 0; const timers = new Map(), painted = [], dropped = [], clones = [];
    const queue = new VideoPlayout({ now: () => now / 1000,
        paint: frame => { painted.push({ id: frame.id, at: now }); onPaint?.(queue); }, drop: reason => dropped.push(reason),
        setTimer: (fn, ms) => { const id = ++next; timers.set(id, { at: now + ms, fn }); return id; },
        clearTimer: id => timers.delete(id) });
    const frame = id => ({ clone() {
        const clone = { id, closed: 0, close() { assert.equal(this.closed++, 0); } }; clones.push(clone); return clone;
    } });
    function advance(at) {
        while (true) {
            const [id, timer] = [...timers].sort((a, b) => a[1].at - b[1].at)[0] ?? [];
            if (!timer || timer.at > at) break;
            timers.delete(id); now = timer.at; timer.fn();
        }
        now = at;
    }
    return { queue, frame, advance, painted, dropped, clones, timers,
        jump: at => { now = at; }, fire: () => { const [id, timer] = [...timers][0]; timers.delete(id); timer.fn(); } };
}

test("steady30Hz arrivals present every original frame80ms later without timer starvation", () => {
    const f = fixture();
    for (let i = 0; i < 90; i++) {
        f.advance(i * 1000 / 30); f.queue.enqueue(f.frame(i), i / 30 + .08);
        assert.ok(f.queue.size <= 3); assert.equal(f.timers.size, 1);
    }
    f.advance(3100);
    assert.equal(f.painted.length, 90); assert.deepEqual(f.dropped, []);
    for (const [i, frame] of f.painted.entries()) {
        assert.equal(frame.id, i); assert.ok(frame.at >= i * 1000 / 30 + 80);
        assert.ok(frame.at < i * 1000 / 30 + 82);
    }
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
    assert.ok(f.clones.every(frame => frame.closed === 1));
});

test("capacity, horizon and late timer bounds discard frames without refreshing their source time", () => {
    const f = fixture();
    for (let i = 0; i < 8; i++) f.queue.enqueue(f.frame(i), .1 + i * .001);
    assert.equal(f.queue.size, 6); assert.equal(f.clones.length, 6);
    assert.deepEqual(f.dropped, ["capacity", "capacity"]);
    f.queue.enqueue(f.frame(8), .201); f.queue.enqueue(f.frame(9), -.021);
    assert.deepEqual(f.dropped.slice(2), ["timing", "timing"]);
    f.jump(170); f.fire(); f.advance(180);
    assert.equal(f.painted.length, 0); assert.equal(f.queue.size, 0);
    assert.equal(f.dropped.filter(v => v === "late").length, 6);
    assert.ok(f.clones.every(frame => frame.closed === 1));
});

test("clock correction and stop cancel every owned frame and timer exactly once", () => {
    const f = fixture();
    f.queue.enqueue(f.frame(0), .08); f.queue.enqueue(f.frame(1), .1);
    f.queue.clear(); f.advance(200); assert.equal(f.painted.length, 0);
    f.queue.enqueue(f.frame(2), .28); f.queue.close(); f.queue.close();
    f.queue.enqueue(f.frame(3), .3); f.advance(400);
    assert.equal(f.clones.length, 3); assert.ok(f.clones.every(frame => frame.closed === 1));
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
});

test("one overdue callback releases every expired original frame before another observer runs", () => {
    const f = fixture();
    for (let i = 0; i < 3; i++) f.queue.enqueue(f.frame(i), .08 + i / 30);
    f.jump(1686); f.fire();
    assert.deepEqual(f.painted, []);
    assert.deepEqual(f.dropped, ["late", "late", "late"]);
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
    assert.ok(f.clones.every(frame => frame.closed === 1));
});

test("late wake drains due frames using the unchanged original allowance and retains future frames", () => {
    const f = fixture();
    for (let i = 0; i < 3; i++) f.queue.enqueue(f.frame(i), .08 + i / 30);
    f.jump(140); f.fire();
    assert.deepEqual(f.dropped, ["late"]);
    assert.deepEqual(f.painted, [{ id: 1, at: 140 }]);
    assert.equal(f.queue.size, 1); assert.equal(f.timers.size, 1);
    assert.equal(f.clones[0].closed, 1); assert.equal(f.clones[1].closed, 1);
    assert.equal(f.clones[2].closed, 0);
    f.advance(148);
    assert.equal(f.painted[1].id, 2); assert.ok(f.painted[1].at >= 146);
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
    assert.ok(f.clones.every(frame => frame.closed === 1));
});

test("early native wake keeps the original target without painting or losing its timer", () => {
    const f = fixture(); f.queue.enqueue(f.frame(0), .08);
    f.jump(40); f.fire();
    assert.deepEqual(f.painted, []); assert.deepEqual(f.dropped, []);
    assert.equal(f.queue.size, 1); assert.equal(f.timers.size, 1);
    f.advance(80);
    assert.deepEqual(f.painted, [{ id: 0, at: 80 }]);
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
    assert.equal(f.clones[0].closed, 1);
});

test("a paint callback may close the queue without double closing owned frames", () => {
    const f = fixture(queue => queue.close());
    for (let i = 0; i < 3; i++) f.queue.enqueue(f.frame(i), .08 + i / 30);
    f.jump(120); f.fire();
    assert.deepEqual(f.painted, [{ id: 0, at: 120 }]);
    assert.equal(f.queue.size, 0); assert.equal(f.timers.size, 0);
    assert.ok(f.clones.every(frame => frame.closed === 1));
});
