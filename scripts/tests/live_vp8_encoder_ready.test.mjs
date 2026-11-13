// Controlled original child protocol tests; actual C/pixels have a separate test.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { LiveVp8Encoder } from "../lib/live_vp8_encoder.mjs";

const base = resolve(".local/gate-6/live-vp8-ready-worker");
const gone = pid => assert.throws(() => process.kill(pid, 0), error => error.code === "ESRCH");
async function fixture(run) {
    assert.ok(["linux", "darwin"].includes(process.platform));
    assert.equal(process.arch, "arm64");
    await mkdir(base, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(base, "owned-"));
    const raw = join(directory, "original.i420");
    await writeFile(raw, Buffer.alloc(640 * 360 * 3 / 2), { mode: 0o600 });
    try { await run(directory, raw); }
    finally { await rm(directory, { recursive: true }); }
}
async function worker(directory, source) {
    const path = join(directory, "controlled-encoder.mjs"), pid = join(directory, "child.pid");
    await writeFile(path, `#!${process.execPath}
import {writeFileSync} from 'node:fs';
writeFileSync(${JSON.stringify(pid)},String(process.pid),{mode:0o600});
if(process.argv.at(-1)!=='--ready-v1')process.exit(71);
${source}
`, { mode: 0o700 });
    return { path, pid };
}
const reply = `
process.stdin.once('data', request => {
 if(!initialized || request.length!==8 || request.readUInt32LE(0)!==0 || request.readUInt32LE(4)!==1)process.exit(72);
 const header=Buffer.alloc(16), body=Buffer.from([0,0,0,0x9d,1,0x2a,0x80,2,0x68,1]);
 header.write('V8F1');header.writeUInt32LE(0,4);header.writeUInt32LE(body.length,8);header.writeUInt32LE(1,12);
 process.stdout.write(Buffer.concat([header,body]));
});`;

test("encoder open waits for delayed READY before any original frame request", async () => fixture(async (directory, raw) => {
    const controlled = await worker(directory, `let initialized=false;
${reply}
setTimeout(()=>{initialized=true;process.stdout.write('V8R1');},50);`);
    const encoder = await LiveVp8Encoder.open(controlled.path, raw, 1);
    let closed;
    try {
        const frame = await encoder.frame(0, true, { timeoutMs: 200 });
        assert.equal(frame.index, 0); assert.equal(frame.timestamp, 0); assert.equal(frame.key, true);
    } finally { closed = await encoder.stopAndJoin(); }
    assert.equal(closed.joined, true); assert.equal(closed.clean, true); gone(encoder.pid);
}));

test("encoder initialization consumes exactly one fragmented marker without consuming frame zero", async () => fixture(async (directory, raw) => {
    const controlled = await worker(directory, `let initialized=false;
${reply}
process.stdout.write('V8');setImmediate(()=>{initialized=true;process.stdout.write('R1');});`);
    const encoder = await LiveVp8Encoder.open(controlled.path, raw, 1);
    let closed;
    try {
        const frame = await encoder.frame(0, true, { timeoutMs: 200 });
        assert.equal(frame.index, 0); assert.equal(frame.bytes.length, 10);
        await assert.rejects(encoder.frame(0, true));
    } finally { closed = await encoder.stopAndJoin(); }
    assert.equal(closed.joined, true); assert.equal(closed.clean, true); gone(encoder.pid);
}));

test("encoder startup rejects foreign partial EOF and extra output only after original child join", async () => fixture(async (directory, raw) => {
    const cases = ["process.stdout.write('NOPE');process.stdin.resume();",
        "process.stdout.write('V8');process.stdin.resume();",
        "process.stdout.end();process.stdin.resume();",
        "process.stdout.write('V8R1x');process.stdin.resume();"];
    for (let index = 0; index < cases.length; index++) {
        const childDirectory = join(directory, String(index)); await mkdir(childDirectory, { mode: 0o700 });
        // A partial marker closes its output immediately, so it exercises EOF
        // rather than borrowing the separate eight-second deadline case.
        const source = index === 1 ? "process.stdout.end('V8');process.stdin.resume();" : cases[index];
        const controlled = await worker(childDirectory, source);
        await assert.rejects(LiveVp8Encoder.open(controlled.path, raw, 1), /live encoder/);
        gone(Number(await readFile(controlled.pid, "utf8")));
    }
}));

test("encoder startup original eight-second deadline rejects and joins the same silent worker", { timeout: 15000 }, async () => fixture(async (directory, raw) => {
    const controlled = await worker(directory, "process.stdin.resume();");
    await assert.rejects(LiveVp8Encoder.open(controlled.path, raw, 1), /original setup deadline/);
    gone(Number(await readFile(controlled.pid, "utf8")));
}));
