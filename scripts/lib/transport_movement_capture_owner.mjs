// Offline capture interpretation owns a separate heap. The original Node SDK
// owner remains live; every interpreter is reaped before its terminal marker.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { open, realpath, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { monotonicNow, programmeElapsed } from "./transport_programme_clock.mjs";

const file = fileURLToPath(import.meta.url);
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const encode = value => JSON.stringify(value, null, 2) + "\n";
const hashPattern = /^[0-9a-f]{64}$/;

async function privateDirectory(path) {
    assert.equal(typeof path, "string");
    assert.ok(resolve(path).split("/").includes(".local"));
    assert.equal(await realpath(path), path);
    return path;
}

async function original(path, maximum, json = false) {
    assert.equal(await realpath(path), path, "original capture path cannot be substituted");
    const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
        const before = await handle.stat();
        assert.ok(before.isFile() && !(before.mode & 0o077) && before.size > 0 && before.size <= maximum);
        const hash = createHash("sha256"), parts = json ? [] : undefined;
        const buffer = Buffer.alloc(64 * 1024);
        let size = 0;
        for (;;) {
            const { bytesRead } = await handle.read(buffer, 0, buffer.length, size);
            if (!bytesRead) break;
            size += bytesRead; assert.ok(size <= maximum);
            const bytes = buffer.subarray(0, bytesRead); hash.update(bytes);
            if (parts) parts.push(Buffer.from(bytes));
        }
        const after = await handle.stat();
        for (const key of ["dev", "ino", "size", "mode", "mtimeMs", "ctimeMs"]) assert.equal(after[key], before[key]);
        assert.equal(size, before.size);
        return { sha256: hash.digest("hex"), ...(parts ? { value: JSON.parse(Buffer.concat(parts, size)) } : {}) };
    } finally { await handle.close(); }
}

function originalCutoff(options) {
    assert.equal(options.packetWorkersJoined, true, "original packet owners must join before interpretation");
    const clock = options.movement.original_program_clock;
    const elapsed = programmeElapsed(clock, 67);
    assert.ok(elapsed >= 62000, "original programme must finish before offline interpretation");
    const cutoff = Number(BigInt(clock.origin_ns)) / 1e6 + 67000;
    assert.equal(options.movement.current_owner_end.original_cutoff_monotonic_ms, cutoff);
    return cutoff;
}

async function interpret(path) {
    const input = await original(path, 256 * 1024, true), options = input.value;
    originalCutoff(options);
    await privateDirectory(options.directory); await privateDirectory(options.outputDirectory);
    const plan = await original(join(options.directory, "plan.json"), 2 * 1024 * 1024, true);
    const { physicalMovementSegments } = await import("./transport_movement_observation.mjs");
    const normalized = await physicalMovementSegments({ ...options, plan: plan.value });
    const captures = [];
    for (let index = 0; index < 6; index++) {
        originalCutoff(options);
        captures.push((await original(join(options.outputDirectory, "received-" + index + ".json"), 96 * 1024 * 1024)).sha256);
    }
    const bytes = encode(normalized);
    originalCutoff(options);
    const output = join(options.outputDirectory, "movement-segments.json");
    await writeFile(output, bytes, { flag: "wx", mode: 0o600 });
    const written = await original(output, 96 * 1024 * 1024);
    assert.equal(written.sha256, sha(bytes));
    originalCutoff(options);
    process.stdout.write(JSON.stringify({ format: normalized.format, sha256: written.sha256,
        original_capture_sha256: captures, original_move_records_bound: normalized.original_move_records_bound,
        reliable_data_exchanges_bound: normalized.reliable_data_exchanges_bound,
        interpreter_input_sha256: input.sha256 }) + "\n");
}

export async function writeJoinedMovementCapture(options, { signal } = {}) {
    const cutoff = originalCutoff(options);
    signal?.throwIfAborted();
    await privateDirectory(options.directory); await privateDirectory(options.outputDirectory);
    const inputPath = join(options.outputDirectory, "movement-capture-input.json"), bytes = encode(options);
    assert.ok(Buffer.byteLength(bytes) <= 256 * 1024, "only bounded original metadata crosses to the interpreter");
    await writeFile(inputPath, bytes, { flag: "wx", mode: 0o600 });
    signal?.throwIfAborted(); originalCutoff(options);
    const child = spawn(process.execPath, [file, "--interpret", inputPath], { stdio: ["ignore", "pipe", "pipe"] });
    let output = Buffer.alloc(0), diagnostic = Buffer.alloc(0), failure, forced = false, closed = false;
    const receipt = { format: "connect/native-movement-capture-owner/1", parent_pid: process.pid, pid: child.pid ?? null,
        interpreter_input_sha256: sha(bytes), original_program_clock: options.movement.original_program_clock,
        original_cutoff_monotonic_ms: cutoff, started_monotonic_ms: monotonicNow(),
        joined: false, forced: false, exit_code: null, signal: null };
    const stop = error => {
        failure ??= error;
        if (!closed && child.exitCode === null && child.signalCode === null) {
            forced = true; child.kill("SIGKILL");
        }
    };
    child.stdout.on("data", chunk => {
        if (output.length + chunk.length > 2048) stop(Error("movement interpreter metadata bound"));
        else output = Buffer.concat([output, chunk]);
    });
    child.stderr.on("data", chunk => {
        if (diagnostic.length + chunk.length > 2048) stop(Error("movement interpreter diagnostic bound"));
        else diagnostic = Buffer.concat([diagnostic, chunk]);
    });
    child.stdout.on("error", stop); child.stderr.on("error", stop);
    child.on("error", error => { failure ??= error; });
    const joined = new Promise(done => child.once("close", (code, stopped) => {
        closed = true; receipt.joined = Number.isSafeInteger(child.pid);
        receipt.exit_code = code; receipt.signal = stopped;
        receipt.closed_monotonic_ms = monotonicNow(); done();
    }));
    const abort = () => stop(signal.reason ?? Error("original capture owner cancelled"));
    const interrupt = () => stop(Error("original capture owner interrupted"));
    signal?.addEventListener("abort", abort, { once: true });
    process.once("SIGTERM", interrupt); process.once("SIGINT", interrupt);
    const timer = setTimeout(() => stop(Error("original programme cleanup cutoff elapsed")), Math.max(0, cutoff - monotonicNow()));
    if (signal?.aborted) abort();
    try {
        await writeFile(join(options.outputDirectory, "movement-capture-owner.json"), encode(receipt), { flag: "wx", mode: 0o600 });
        await joined;
    } catch (error) {
        stop(error); await joined;
    } finally {
        clearTimeout(timer); signal?.removeEventListener("abort", abort);
        process.removeListener("SIGTERM", interrupt); process.removeListener("SIGINT", interrupt);
        receipt.forced = forced;
        await writeFile(join(options.outputDirectory, "movement-capture-joined.json"), encode(receipt), { flag: "wx", mode: 0o600 });
        if (diagnostic.length) await writeFile(join(options.outputDirectory, "movement-capture-error.json"), diagnostic, { flag: "wx", mode: 0o600 });
    }
    if (failure) throw failure;
    assert.equal(receipt.joined, true); assert.equal(receipt.exit_code, 0); assert.equal(receipt.signal, null);
    assert.ok(receipt.closed_monotonic_ms < cutoff, "interpreter must join inside the original cleanup cutoff");
    const result = JSON.parse(output);
    assert.deepEqual(Object.keys(result).sort(), ["format", "sha256", "original_capture_sha256",
        "original_move_records_bound", "reliable_data_exchanges_bound", "interpreter_input_sha256"].sort());
    assert.equal(result.format, "connect/physical-movement-segments/1");
    assert.equal(result.interpreter_input_sha256, sha(bytes)); assert.match(result.sha256, hashPattern);
    assert.equal(result.original_move_records_bound, options.movement.original_moves_completed);
    assert.ok(Number.isSafeInteger(result.reliable_data_exchanges_bound) && result.reliable_data_exchanges_bound >= 0);
    assert.equal(result.original_capture_sha256.length, 6);
    for (const hash of result.original_capture_sha256) assert.match(hash, hashPattern);
    return { ...result, capture_interpreter_joined: true, interpreter_process_sha256: sha(encode(receipt)) };
}

if (process.argv[1] && resolve(process.argv[1]) === file && process.argv[2] === "--interpret") {
    try {
        assert.equal(process.argv.length, 4);
        await interpret(resolve(process.argv[3]));
    } catch (error) {
        // Raw packet or private assertion values never cross the small control
        // channel. Their original files remain available for investigation.
        process.stderr.write(JSON.stringify({ failure: /^[A-Za-z]{1,64}$/.test(error.name) ? error.name : "Error" }) + "\n");
        process.exitCode = 1;
    }
}
