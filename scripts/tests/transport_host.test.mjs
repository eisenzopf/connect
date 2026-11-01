import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { nativeHost, executableHash, childPids, processExists } from "../lib/transport_host.mjs";

test("actual native host and process image match the current Node executable", async () => {
    assert.equal(nativeHost().platform, process.platform === "darwin" ? "macos-arm64" : "ubuntu24-x86_64");
    assert.equal(await executableHash(process.pid), createHash("sha256").update(await readFile(process.execPath)).digest("hex"));
    assert.equal(await processExists(process.pid), true);
});

test("owned child is observed under its actual parent and disappears after its original join", async () => {
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
    const joined = once(child, "exit");
    try {
        await once(child, "spawn");
        assert.ok(childPids(process.pid).includes(child.pid));
        assert.equal(await executableHash(child.pid), await executableHash(process.pid));
        assert.equal(await processExists(child.pid), true);
    } finally {
        child.kill("SIGTERM");
        await joined;
    }
    assert.equal(await processExists(child.pid), false);
    await assert.rejects(executableHash(child.pid), error => ["ENOENT", "ESRCH"].includes(error.code));
});
