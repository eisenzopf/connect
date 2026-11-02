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

test("a full64KiB Data receipt stays in the original file while joined control metadata retains its digest", async () => {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true });
    const directory = await mkdtemp(join(parent, "capture-inspection-data-"));
    try {
        const path = join(directory, "original.json");
        const receipt = { format: "connect/physical-data-exchange/1", incoming_exact_body_base64: Buffer.alloc(65536, 7).toString("base64"),
            incoming_received: true, outgoing_core_queue_admission: true, outgoing_mutation_submissions: 1,
            root_operation_id: "original-root", connection_id: "original-connection" };
        const observation = { passed: true, received: [], data_exchange_receipt: receipt };
        const bytes = Buffer.from(JSON.stringify(observation));
        assert.ok(bytes.length > 65536);
        await writeFile(path, bytes, { mode: 0o600 });
        const result = await inspectReplacementCapture(path, 1024 * 1024, 2);
        assert.equal(result.worker_joined, true);
        assert.equal(result.observation.data_exchange_receipt, undefined);
        const canonical = Object.fromEntries(Object.keys(receipt).sort().map(key => [key, receipt[key]]));
        assert.equal(result.observation.data_exchange_receipt_sha256, createHash("sha256").update(JSON.stringify(canonical)).digest("hex"));
        const { verifyInspectedDataReceipt } = await import("../lib/transport_capture_inspection.mjs");
        verifyInspectedDataReceipt(result.observation, receipt);
        verifyInspectedDataReceipt(result.observation, Object.fromEntries(Object.entries(receipt).reverse()));
        assert.throws(() => verifyInspectedDataReceipt(result.observation, { ...receipt, root_operation_id: "foreign" }));
        assert.throws(() => verifyInspectedDataReceipt(result.observation, { ...receipt, incoming_exact_body_base64: Buffer.alloc(65536, 8).toString("base64") }));
        assert.ok(Buffer.byteLength(JSON.stringify(result.observation)) <= 65536);
        assert.equal(result.sha256, createHash("sha256").update(bytes).digest("hex"));
        assert.deepEqual(await readFile(path), bytes);
        await writeFile(path, JSON.stringify({ ...observation, unexpected: "x".repeat(65536) }));
        await assert.rejects(inspectReplacementCapture(path, 1024 * 1024, 2));
        await writeFile(path, JSON.stringify({ ...observation, data_exchange_receipt_sha256: result.observation.data_exchange_receipt_sha256 }));
        await assert.rejects(inspectReplacementCapture(path, 1024 * 1024, 2));
    } finally { await rm(directory, { recursive: true, force: true }); }
});
