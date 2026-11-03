// Verify this review kit before installing or starting its runtime.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, lstat, realpath } from "node:fs/promises";
import { resolve, join, sep } from "node:path";
const root = await realpath(process.cwd());
const manifest = JSON.parse(await readFile(join(root, "quickstart-manifest.json"), "utf8"));
assert.equal(manifest.format, "connect/first-conversation-kit/1");
assert.equal(manifest.platform, process.platform + "-" + process.arch, "matching-ARM-runtime-required");
const [major, minor] = process.versions.node.split(".").map(Number);
assert.ok(major > 22 || major === 22 && minor >= 16, "Node22.16-or-newer-required");
assert.ok(process.getuid() > 0, "use-an-ordinary-user");
assert.ok(Object.keys(manifest.files).length > 10 && Object.keys(manifest.files).length <= 2000);
for (const [name, expected] of Object.entries(manifest.files)) {
    assert.ok(!name.startsWith("/") && !name.split("/").some(p => ["", ".", ".."].includes(p)));
    const path = resolve(root, name);
    assert.ok(path.startsWith(root + sep));
    const info = await lstat(path);
    assert.ok(info.isFile() && !info.isSymbolicLink() && info.size <= 512 * 1024 * 1024);
    assert.equal(await realpath(path), path);
    assert.equal(createHash("sha256").update(await readFile(path)).digest("hex"), expected.sha256, name);
    assert.equal(info.mode & 0o111, expected.mode & 0o111, name);
}
const binary = await readFile(join(root, "runtime/bin/connect"));
if (process.platform === "linux") {
    assert.equal(binary.subarray(0, 6).toString("hex"), "7f454c460201");
    assert.equal(binary.readUInt16LE(18), 183, "AArch64-ELF-required");
} else {
    assert.equal(binary.subarray(0, 4).toString("hex"), "cffaedfe");
    assert.equal(binary.readUInt32LE(4), 0x0100000c, "ARM64-MachO-required");
}
console.log("Verified Connect first-conversation kit for " + manifest.platform);
