import assert from "node:assert/strict";
import { test } from "node:test";
import { startProgrammeClock, programmeElapsed, monotonicNow, validateProgrammeClock } from "../lib/transport_programme_clock.mjs";

test("one original monotonic programme survives wall steps and rejects foreign or expired anchors", () => {
    const originalDateNow = Date.now;
    const wall = Date.now(), clock = startProgrammeClock();
    try {
        for (const step of [-100, 100, -86400000, 86400000]) {
            Date.now = () => wall + step;
            const before = monotonicNow() - Number(BigInt(clock.origin_ns)) / 1e6;
            const elapsed = programmeElapsed(clock, 62);
            const after = monotonicNow() - Number(BigInt(clock.origin_ns)) / 1e6;
            assert.ok(elapsed >= before - 0.001 && elapsed <= after + 0.001);
            assert.equal(clock.origin_ns, validateProgrammeClock(clock).origin_ns);
        }
        assert.throws(() => programmeElapsed({ ...clock, domain: "foreign-boot" }, 62));
        assert.throws(() => programmeElapsed({ ...clock, origin_ns: (process.hrtime.bigint() + 1000000000n).toString() }, 62));
        assert.throws(() => programmeElapsed({ ...clock, origin_ns: (process.hrtime.bigint() - 62000000001n).toString() }, 62));
        for (const value of ["0", "01", "-1", "1.5", "18446744073709551616"])
            assert.throws(() => validateProgrammeClock({ ...clock, origin_ns: value }));
    } finally { Date.now = originalDateNow; }
});
