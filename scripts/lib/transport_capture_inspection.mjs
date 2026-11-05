// Large private packet captures must not stop the live six-route media loop.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { open } from "node:fs/promises";
import { isMainThread, parentPort, Worker, workerData } from "node:worker_threads";

// The existing Data receipt is flat JSON. Hash every field in stable key order;
// never transfer its duplicate64KiB payload into bounded control metadata.
function dataReceiptDigest(receipt) {
    assert.ok(receipt && typeof receipt === "object" && !Array.isArray(receipt));
    assert.ok(Object.values(receipt).every(value => value === null || ["string", "boolean"].includes(typeof value) || Number.isSafeInteger(value)));
    const bytes = Buffer.from(JSON.stringify(Object.fromEntries(Object.keys(receipt).sort().map(key => [key, receipt[key]]))));
    assert.ok(bytes.length <= 256 * 1024);
    return createHash("sha256").update(bytes).digest("hex");
}
export function verifyInspectedDataReceipt(observation, receipt) {
    assert.equal(observation.data_exchange_receipt, undefined);
    assert.match(observation.data_exchange_receipt_sha256, /^[0-9a-f]{64}$/);
    assert.equal(observation.data_exchange_receipt_sha256, dataReceiptDigest(receipt));
}

if (!isMainThread && workerData?.profile === "connect/replacement-capture-inspection/1") {
    const { path, maximumBytes, maximumPackets } = workerData;
    const file = await open(path, "r");
    let bytes;
    try {
        const before = await file.stat();
        assert.ok(before.isFile() && before.size > 0 && before.size <= maximumBytes);
        assert.equal(before.mode & 0o077, 0);
        bytes = await file.readFile();
        const after = await file.stat();
        assert.equal(bytes.length, before.size);
        assert.equal(after.size, before.size);
        assert.equal(after.mtimeMs, before.mtimeMs);
        assert.equal(after.ctimeMs, before.ctimeMs);
    } finally { await file.close(); }
    const observation = JSON.parse(bytes.toString("utf8"));
    assert.ok(Array.isArray(observation.received) && observation.received.length <= maximumPackets);
    const received = observation.received.map(row => ({
        original_root_completed_observed: typeof row?.original_root_completed_observed === "boolean"
            ? row.original_root_completed_observed : null,
    }));
    delete observation.received;
    assert.ok(!Object.hasOwn(observation, "data_exchange_receipt_sha256"));
    if (observation.data_exchange_receipt !== undefined && observation.data_exchange_receipt !== null) {
        observation.data_exchange_receipt_sha256 = dataReceiptDigest(observation.data_exchange_receipt);
        delete observation.data_exchange_receipt;
    }
    // Transfer bounded metadata only; payload bytes stay in their original file.
    assert.ok(Buffer.byteLength(JSON.stringify(observation)) <= 65536);
    parentPort.postMessage({ observation: { ...observation, received },
        sha256: createHash("sha256").update(bytes).digest("hex"), byteLength: bytes.length });
}

// Keep the result private until the exact worker exits successfully. Even
// timeout/error paths join their worker before rejecting the original call.
export async function inspectReplacementCapture(path, maximumBytes, maximumPackets) {
    assert.ok(typeof path === "string");
    assert.ok(Number.isSafeInteger(maximumBytes) && maximumBytes > 0 && maximumBytes <= 80 * 1024 * 1024);
    assert.ok(Number.isSafeInteger(maximumPackets) && maximumPackets >= 0 && maximumPackets <= 16000);
    const worker = new Worker(new URL(import.meta.url), { workerData: { profile: "connect/replacement-capture-inspection/1", path, maximumBytes, maximumPackets } });
    let value, failure;
    worker.on("message", message => { value = message; });
    worker.on("error", error => { failure = error; });
    const timer = setTimeout(() => {
        failure = Error("replacement capture inspection deadline");
        void worker.terminate();
    }, 5000);
    const code = await new Promise(done => worker.once("exit", done));
    clearTimeout(timer);
    if (failure) throw failure;
    assert.equal(code, 0); assert.ok(value);
    return { ...value, worker_joined: true };
}
