import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, chmod } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { startCpuProfile } from "./cpu-profile.mjs";
import { spawn } from "node:child_process";
import { once } from "node:events";

test("explicit CPU observation stops at its bound, writes private bytes and joins one original session", async () => {
    const base = fileURLToPath(new URL("../../../.local/gate-6/reference-app/cpu-tests/", import.meta.url));
    await mkdir(base, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(base, "unit-"));
    try {
        const owner = await startCpuProfile(directory, 60);
        const until = performance.now() + 30;
        while (performance.now() < until) Math.sqrt(Math.random());
        await new Promise(done => setTimeout(done, 100));
        assert.equal((await readdir(directory)).length, 1, "deadline stops without requiring caller shutdown");
        const result = await owner.stop(); assert.equal(await owner.stop(), result);
        assert.ok(result.samples > 0); assert.ok(result.bytes > 0 && result.bytes <= 8 * 1024 * 1024);
        const profile = JSON.parse(await readFile(result.file));
        assert.equal(profile.timeDeltas.length, profile.samples.length);
        assert.equal((await stat(result.file)).mode & 0o077, 0);
        await chmod(directory, 0o755);
        await assert.rejects(startCpuProfile(directory, 60));
    } finally { await rm(directory, { recursive: true, force: true }); }
});

test("original process shutdown stops and writes its CPU observation before the child joins", async () => {
    const base = fileURLToPath(new URL("../../../.local/gate-6/reference-app/cpu-tests/", import.meta.url));
    await mkdir(base, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(base, "close-"));
    const source = `import {startCpuProfile} from ${JSON.stringify(new URL("./cpu-profile.mjs", import.meta.url).href)};
        const owner=await startCpuProfile(process.argv[1]);
        const timer=setInterval(()=>{},1000);
        process.once('SIGTERM',()=>{clearInterval(timer);void owner.stop().then(()=>{process.exitCode=0;},()=>{process.exitCode=1;});});
        process.stdout.write('ready');`;
    const child = spawn(process.execPath, ["--input-type=module", "-e", source, directory], { stdio: ["ignore", "pipe", "ignore"] });
    const finished = once(child, "close");
    let deadlineTimer;
    try {
        await once(child.stdout, "data", { signal: AbortSignal.timeout(3000) });
        child.kill("SIGTERM");
        const deadline = new Promise((_, reject) => { deadlineTimer = setTimeout(() => reject(Error("CPU profile shutdown deadline")), 3000); deadlineTimer.unref(); });
        assert.deepEqual(await Promise.race([finished, deadline]), [0, null]);
        assert.throws(() => process.kill(child.pid, 0), error => error.code === "ESRCH");
        const names = await readdir(directory); assert.equal(names.length, 1);
        const profile = JSON.parse(await readFile(join(directory, names[0])));
        assert.ok(Array.isArray(profile.samples));
    } finally {
        clearTimeout(deadlineTimer);
        if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
        await finished; await rm(directory, { recursive: true, force: true });
    }
});
