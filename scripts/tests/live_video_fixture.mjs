// Finite sparse black source plus the actual C encoder for local ARM tests.
// No network or media transport authority is supplied by this fixture.
import assert from "node:assert/strict";
import { open, link, writeFile, readFile } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

export async function addLiveVideoFixture(directory, manifest) {
    assert.equal(process.platform, "linux"); assert.equal(process.arch, "arm64");
    const binary = join(directory, "actual-encoder"), executable = join(directory, "live-vp8-encoder");
    const result = spawnSync("python3", ["scripts/video/build_live_vp8_encoder.py", "--output", binary], {
        encoding: "utf8", timeout: 60000,
        env: { PATH: process.env.PATH, HOME: process.env.HOME, LANG: "C", PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(result.status, 0, result.stderr);
    const quote = text => "'" + text.replaceAll("'", "'\\''") + "'";
    const pidLog = join(directory, "encoder-pids");
    // exec preserves the recorded PID, so retirement is checked against the
    // actual encoder process, not a wrapper that could leave an orphan.
    await writeFile(executable, `#!/bin/sh\nprintf '%s\\n' "$$" >> ${quote(pidLog)}\nexec ${quote(binary)} "$@"\n`, { mode: 0o700 });
    const hash = createHash("sha256"), first = join(directory, "source-0.i420");
    const file = await open(first, "wx", 0o600);
    try { await file.truncate(1860 * 640 * 360 * 3 / 2); } finally { await file.close(); }
    for await (const chunk of createReadStream(first)) hash.update(chunk);
    const digest = hash.digest("hex");
    for (let index = 0; index < 6; index++) {
        const name = "source-" + index + ".i420";
        if (index) await link(first, join(directory, name));
        Object.assign(manifest.sources[index], { raw_video_file: name, raw_video_sha256: digest });
    }
    manifest.live_encoder = { file: "live-vp8-encoder",
        executable_sha256: createHash("sha256").update(await readFile(executable)).digest("hex") };
    return async () => (await readFile(pidLog, "utf8")).trim().split("\n").map(Number);
}
