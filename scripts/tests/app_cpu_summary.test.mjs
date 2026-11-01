import test from "node:test";
import assert from "node:assert/strict";
import { summarizeCpuProfile } from "../lib/app_cpu_summary.mjs";
const bytes = value => Buffer.from(JSON.stringify(value));
const frame = (id, functionName, url, children = []) => ({ id, callFrame: { functionName, url, lineNumber: 12 }, children });
test("CPU attribution distinguishes self/inclusive and excludes private paths and unknown function names", () => {
    const profile = { nodes: [frame(1, "(root)", "", [2, 4]),
        frame(2, "serve", "file:///private/credential/consumer/app/server.mjs", [3]),
        frame(3, "parse", "file:///private/credential/consumer/node_modules/@rudeless/connect/dist/sdk_helper.js"),
        frame(4, "private-secret-name", "file:///private/secret.js")], samples: [3, 3, 2, 4], timeDeltas: [1000, 1000, 2000, 1000] };
    const result = summarizeCpuProfile(bytes(profile));
    assert.equal(result.total_sampled_ms, 5);
    const app = result.top_self.find(row => row.module === "app/server.mjs");
    assert.equal(app.self_sampled_ms, 2); assert.equal(app.inclusive_sampled_ms, 4);
    assert.equal(result.top_self.find(row => row.module === "sdk/sdk_helper.js").self_sampled_ms, 2);
    assert.equal(JSON.stringify(result).includes("private"), false); assert.equal(JSON.stringify(result).includes("secret"), false);
    assert.equal(result.raw_profile_sha256.length, 64);
});
test("malformed CPU node cycles, unknown samples and oversize input refuse", () => {
    for (const profile of [
        { nodes: [frame(1, "(root)", "", [1])], samples: [1], timeDeltas: [1000] },
        { nodes: [frame(1, "(root)", "")], samples: [2], timeDeltas: [1000] },
        { nodes: [frame(1, "(root)", "")], samples: [1], timeDeltas: [-1] }])
        assert.throws(() => summarizeCpuProfile(bytes(profile)));
    assert.throws(() => summarizeCpuProfile(Buffer.alloc(8 * 1024 * 1024 + 1)));
});
