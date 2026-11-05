import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { fileSha256, FILE_HASH_BUFFER_BYTES } from "../lib/transport_file_hash.mjs";

async function fixture(t) {
    const directory = await fs.mkdtemp(join(tmpdir(), "connect-streaming-digest-"));
    t.after(() => fs.rm(directory, { recursive: true, force: true }));
    return join(directory, "original-executable");
}

test("exact multi-chunk executable digest uses one bounded buffer and closes its original descriptor", async t => {
    const path = await fixture(t), output = await fs.open(path, "wx"), expected = createHash("sha256");
    try {
        for (let index = 0; index < 121; index++) {
            const chunk = Buffer.alloc(65539, index); expected.update(chunk); await output.writeFile(chunk);
        }
        const tail = Buffer.from([0, 255, 3, 127, 9]); expected.update(tail); await output.writeFile(tail);
    } finally { await output.close(); }
    const buffers = new Set(), actualOpen = fs.open;
    let original, closeCalls = 0, largestRead = 0, reads = 0;
    t.mock.method(fs, "open", async (...args) => {
        original = await actualOpen(...args);
        const actualRead = original.read.bind(original), actualClose = original.close.bind(original);
        t.mock.method(original, "read", async (...readArgs) => {
            buffers.add(readArgs[0]); largestRead = Math.max(largestRead, readArgs[2]); reads++;
            return actualRead(...readArgs);
        });
        t.mock.method(original, "close", async () => { closeCalls++; return actualClose(); });
        return original;
    });
    assert.equal(await fileSha256(path), expected.digest("hex"));
    assert.equal(buffers.size, 1); assert.ok(reads > 100); assert.equal(largestRead, FILE_HASH_BUFFER_BYTES);
    assert.equal(closeCalls, 1); assert.equal(original.fd, -1);
});

test("empty and partial-chunk files preserve their exact SHA-256", async t => {
    const path = await fixture(t);
    for (const bytes of [Buffer.alloc(0), Buffer.from("original image bytes\u0000\u00ff")]) {
        await fs.writeFile(path, bytes);
        assert.equal(await fileSha256(path), createHash("sha256").update(bytes).digest("hex"));
    }
});

test("an original read failure is preserved and its descriptor joins", async t => {
    const path = await fixture(t); await fs.writeFile(path, Buffer.alloc(100));
    const actualOpen = fs.open, failure = Object.assign(Error("owned read fault"), { code: "EIO" });
    let original;
    t.mock.method(fs, "open", async (...args) => {
        original = await actualOpen(...args);
        t.mock.method(original, "read", async () => { throw failure; });
        return original;
    });
    await assert.rejects(fileSha256(path), error => error === failure);
    assert.equal(original.fd, -1);
});

test("a growing original image is rejected instead of returning a partial digest", async t => {
    const path = await fixture(t); await fs.writeFile(path, Buffer.alloc(2 * FILE_HASH_BUFFER_BYTES));
    const actualOpen = fs.open; let original, changed = false;
    t.mock.method(fs, "open", async (...args) => {
        original = await actualOpen(...args); const actualRead = original.read.bind(original);
        t.mock.method(original, "read", async (...readArgs) => {
            const value = await actualRead(...readArgs);
            if (!changed) { changed = true; await fs.appendFile(path, Buffer.from([1])); }
            return value;
        });
        return original;
    });
    await assert.rejects(fileSha256(path), /grew during digest/);
    assert.equal(original.fd, -1);
});

test("missing original executable remains an IO refusal", async t => {
    const path = await fixture(t);
    await assert.rejects(fileSha256(path), error => error.code === "ENOENT");
});
