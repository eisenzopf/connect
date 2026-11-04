// Same-client-host programme coordinates. Wall time labels RTP/NTP only.
import assert from "node:assert/strict";
import { readFileSync, readlinkSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

export const monotonicNow = () => Number(process.hrtime.bigint()) / 1e6;
export function programmeClockDomain() {
    if (process.platform === "linux") return {
        kind: "linux-monotonic-v1",
        domain: readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim().toLowerCase()
            + "/" + readlinkSync("/proc/self/ns/time"),
    };
    if (process.platform === "darwin") {
        const [major, minor] = process.versions.uv.split(".").map(Number);
        assert.ok(major > 1 || major === 1 && minor >= 49, "qualified libuv continuous clock required");
        return { kind: "darwin-continuous-v1", domain: execFileSync("/usr/sbin/sysctl",
            ["-n", "kern.bootsessionuuid"], { encoding: "utf8", timeout: 1000 }).trim().toLowerCase() };
    }
    throw Error("Programme clock requires the qualified Linux/macOS client host");
}
export function validateProgrammeClock(clock) {
    assert.deepEqual(Object.keys(clock).sort(), ["domain", "kind", "origin_ns"]);
    assert.ok(typeof clock.domain === "string" && clock.domain.length > 0 && clock.domain.length <= 160);
    assert.ok(["linux-monotonic-v1", "darwin-continuous-v1"].includes(clock.kind));
    assert.ok(typeof clock.origin_ns === "string" && /^[1-9][0-9]{0,19}$/.test(clock.origin_ns));
    assert.ok(BigInt(clock.origin_ns) <= 18446744073709551615n);
    return clock;
}
export function startProgrammeClock() {
    const domain = programmeClockDomain();
    return Object.freeze({ ...domain, origin_ns: process.hrtime.bigint().toString() });
}
export function programmeElapsed(clock, seconds) {
    validateProgrammeClock(clock);
    const domain = programmeClockDomain();
    assert.equal(clock.kind, domain.kind); assert.equal(clock.domain, domain.domain);
    const elapsed = process.hrtime.bigint() - BigInt(clock.origin_ns);
    assert.ok(elapsed >= 0n && elapsed < BigInt(seconds) * 1000000000n,
        "original monotonic programme must be current");
    return Number(elapsed) / 1e6;
}

// Use the source programme coordinate at every boundary, including warmup.
export async function waitForProgrammeOffset(workload, offsetMs,
    wait = (ms, signal) => sleep(ms, undefined, { signal })) {
    assert.equal(typeof workload.now, "function");
    assert.ok(Number.isFinite(workload.startedAt));
    assert.ok(Number.isFinite(offsetMs) && offsetMs >= 0 && offsetMs <= 62000);
    await wait(Math.max(0, workload.startedAt + offsetMs - workload.now()), workload.signal);
}
