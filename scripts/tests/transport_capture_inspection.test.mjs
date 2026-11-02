import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { inspectReplacementCapture } from "../lib/transport_capture_inspection.mjs";

test("joined capture inspection preserves exact byte custody and every original-root boundary without transferring packet bodies", async () => {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true });
    const directory = await mkdtemp(join(parent, "capture-inspection-"));
    try {
        const path = join(directory, "original.json");
        const bytes = Buffer.from(JSON.stringify({ program_start_unix_us: 1793600000000000,
            received: [false, true].map(boundary => ({
                original_root_completed_observed: boundary, payload: Array(100000).fill(42),
            })) }));
        await writeFile(path, bytes, { mode: 0o600 });
        const result = await inspectReplacementCapture(path, 1024 * 1024, 2);
        assert.equal(result.worker_joined, true);
        assert.equal(result.byteLength, bytes.length);
        assert.equal(result.sha256, createHash("sha256").update(bytes).digest("hex"));
        assert.deepEqual(result.observation, { program_start_unix_us: 1793600000000000,
            received: [{ original_root_completed_observed: false }, { original_root_completed_observed: true }] });
        assert.deepEqual(await readFile(path), bytes);
        await assert.rejects(inspectReplacementCapture(path, bytes.length - 1, 2));
        await assert.rejects(inspectReplacementCapture(path, 1024 * 1024, 1));
    } finally { await rm(directory, { recursive: true, force: true }); }
});

test("malformed capture boundaries stay invalid and oversized metadata or invalid JSON cannot produce a joined success", async () => {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true });
    const directory = await mkdtemp(join(parent, "capture-inspection-invalid-"));
    try {
        const path = join(directory, "original.json");
        await writeFile(path, JSON.stringify({ received: [{ original_root_completed_observed: "true" }, null] }), { mode: 0o600 });
        const result = await inspectReplacementCapture(path, 1024 * 1024, 2);
        assert.deepEqual(result.observation.received, [
            { original_root_completed_observed: null }, { original_root_completed_observed: null }]);
        await writeFile(path, JSON.stringify({ received: [], unexpected: "x".repeat(65536) }));
        await assert.rejects(inspectReplacementCapture(path, 1024 * 1024, 2));
        await writeFile(path, "{broken");
        await assert.rejects(inspectReplacementCapture(path, 1024 * 1024, 2));
    } finally { await rm(directory, { recursive: true, force: true }); }
});
