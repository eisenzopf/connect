import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import { LiveVp8Encoder } from "../lib/live_vp8_encoder.mjs";

const base = resolve(".local/gate-6/live-vp8-worker");
const env = { PATH: process.env.PATH, HOME: process.env.HOME, LANG: "C", PYTHONDONTWRITEBYTECODE: "1" };
const gone = pid => assert.throws(() => process.kill(pid, 0), error => error.code === "ESRCH");
async function fixture(run) {
    assert.equal(process.platform, "linux"); assert.equal(process.arch, "arm64");
    await mkdir(base, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(base, "owned-"));
    const raw = join(directory, "original.i420"), plane = 640 * 360;
    const originals = Buffer.alloc(plane * 3 / 2 * 12, 128);
    for (let index = 0; index < 12; index++) originals.fill(60 + 3 * index, index * plane * 3 / 2, index * plane * 3 / 2 + plane);
    await writeFile(raw, originals, { mode: 0o600 });
    try { await run(directory, raw); } finally { await rm(directory, { recursive: true }); }
}
async function fake(directory, source) {
    const path = join(directory, "controlled-encoder");
    await writeFile(path, `#!${process.execPath}\n${source}\n`, { mode: 0o700 });
    return path;
}
test("owned encoder keeps one original reply across a raced wait and joins actual fresh-frame source", async () => fixture(async (directory, raw) => {
    const binary = join(directory, "encoder");
    const build = spawnSync("python3", ["scripts/video/build_live_vp8_encoder.py", "--output", binary], { env, timeout: 60000, encoding: "utf8" });
    assert.equal(build.status, 0, build.stderr);
    const encoder = await LiveVp8Encoder.open(binary, raw, 12);
    let outcome;
    try {
        const original = encoder.frame(0, true, { timeoutMs: 200 });
        // An unrelated control completes before its media wait. The original
        // promise still owns the pipe reply; no new command may replace it.
        assert.equal(await Promise.race([original.then(() => "media"), Promise.resolve("control")]), "control");
        await assert.rejects(encoder.frame(1, false), /one encoder request/);
        const first = await original;
        assert.equal(first.index, 0); assert.equal(first.timestamp, 0); assert.equal(first.key, true);
        const fresh = await encoder.frame(7, true, { timeoutMs: 200 });
        assert.equal(fresh.index, 7); assert.equal(fresh.timestamp, 21000); assert.equal(fresh.key, true);
        const header = Buffer.alloc(32); header.write("DKIF"); header.writeUInt16LE(32, 6); header.write("VP80", 8);
        header.writeUInt16LE(640, 12); header.writeUInt16LE(360, 14); header.writeUInt32LE(30, 16); header.writeUInt32LE(1, 20); header.writeUInt32LE(1, 24);
        const row = Buffer.alloc(12); row.writeUInt32LE(fresh.bytes.length); row.writeBigUInt64LE(7n, 4);
        const ivf = join(directory, "fresh.ivf"); await writeFile(ivf, Buffer.concat([header, row, fresh.bytes]), { mode: 0o600 });
        const decode = spawnSync("ffmpeg", ["-nostdin", "-hide_banner", "-loglevel", "error", "-i", ivf,
            "-vf", "crop=16:16:0:0", "-pix_fmt", "yuv420p", "-frames:v", "1", "-f", "rawvideo", "pipe:1"], { env, timeout: 10000, maxBuffer: 4096 });
        assert.equal(decode.status, 0, String(decode.stderr)); assert.equal(decode.stdout.length, 384);
        const mean = decode.stdout.subarray(0, 256).reduce((a,b) => a+b, 0) / 256;
        assert.ok(Math.abs(mean - 81) <= 3, "independent decoder must recover requested original frame7");
        await assert.rejects(encoder.frame(7, false));
    } finally { outcome = await encoder.stopAndJoin(); }
    assert.equal(outcome.clean, true); assert.equal(outcome.joined, true); gone(encoder.pid);
}));

test("owned encoder rejects partial, foreign and oversized replies and joins cancellation without reissuing", async () => fixture(async (directory, raw) => {
    for (const mode of ["partial", "foreign", "oversize", "cancel"]) {
        const binary = await fake(directory, `
process.stdin.once('data', request => {
 const header=Buffer.alloc(16); header.write('V8F1'); header.writeUInt32LE(request.readUInt32LE() + ${mode === 'foreign' ? 1 : 0},4);
 header.writeUInt32LE(${mode === 'oversize' ? 262145 : 10},8); header.writeUInt32LE(1,12);
 process.stdout.write(header.subarray(0, ${mode === 'partial' ? 9 : 16}));
 setInterval(()=>{},1000);
});`);
        const encoder = await LiveVp8Encoder.open(binary, raw, 12), abort = new AbortController();
        const pending = encoder.frame(0, true, { signal: abort.signal, timeoutMs: 200 });
        const cancelled = mode === "cancel" ? setTimeout(() => abort.abort(), 30) : undefined;
        try { await assert.rejects(pending, /live encoder/); }
        finally { clearTimeout(cancelled); const result = await encoder.stopAndJoin(); assert.equal(result.joined, true); assert.equal(result.clean, false); }
        gone(encoder.pid); await assert.rejects(encoder.frame(1, false));
    }
}));
