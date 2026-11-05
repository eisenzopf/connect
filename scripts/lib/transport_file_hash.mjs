// Exact file custody without retaining an executable-sized Buffer in the live
// fixture owner. The one buffer and file descriptor belong to this invocation.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";

export const FILE_HASH_BUFFER_BYTES = 64 * 1024;

export async function fileSha256(path) {
    const file = await fs.open(path, "r");
    try {
        const before = await file.stat({ bigint: true });
        assert.ok(before.isFile(), "Original ordinary executable/file required");
        const bytes = Buffer.allocUnsafe(FILE_HASH_BUFFER_BYTES), hash = createHash("sha256");
        let consumed = 0n;
        while (consumed < before.size) {
            const remaining = before.size - consumed;
            const length = remaining > BigInt(bytes.length) ? bytes.length : Number(remaining);
            const { bytesRead } = await file.read(bytes, 0, length, null);
            assert.ok(bytesRead > 0, "Original file truncated during digest");
            hash.update(bytes.subarray(0, bytesRead));
            consumed += BigInt(bytesRead);
        }
        const { bytesRead } = await file.read(bytes, 0, 1, null);
        assert.equal(bytesRead, 0, "Original file grew during digest");
        const after = await file.stat({ bigint: true });
        for (const field of ["dev", "ino", "size", "mtimeNs", "ctimeNs"])
            assert.equal(after[field], before[field], "Original file changed during digest");
        return hash.digest("hex");
    } finally {
        await file.close();
    }
}
