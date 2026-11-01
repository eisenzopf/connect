// Opt-in macOS stack sampling of the exact server in this rehearsal's own group.
// Raw command lines, stack files, PIDs and absolute paths stay in ignored state.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, lstat, realpath } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { availableParallelism } from "node:os";
import { join, resolve, sep } from "node:path";
const execute = promisify(execFile), fail = () => { throw Error("Owned native sampling target required"); };
export function selectOwnedServer(lines, group, command) {
    if (!Number.isSafeInteger(group) || group < 1) fail();
    const matches = lines.split("\n").map(line => /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(line)).filter(Boolean)
        .filter(row => Number(row[2]) === group && row[3] === command);
    if (matches.length !== 1) fail();
    return Number(matches[0][1]);
}
export async function finishNativeBeforeStop(sample, stop) {
    let observation;
    if (sample) {
        try { observation = await sample.finish(); }
        catch { observation = { observed: false, reason: "native-sample-result-unavailable" }; }
    }
    // Even an unavailable observation must release the original server. Its
    // stop callback is never invoked while this observer remains outstanding.
    await stop();
    return observation;
}
export function summarizeNativeSample(text, pid, demangled = new Map()) {
    if (Buffer.byteLength(text) > 8 * 1024 * 1024 || Number(/^Process:\s+.*\[(\d+)\]\s*$/m.exec(text)?.[1]) !== pid) fail();
    const begin = text.indexOf("Call graph:"), end = text.indexOf("Total number in stack", begin);
    if (begin < 0) fail();
    const graph = text.slice(begin, end < 0 ? text.length : end), counts = new Map();
    const waits = new Set(["__psynch_mutexwait", "__psynch_cvwait", "__ulock_wait2", "mach_msg2_trap", "poll", "kevent", "kevent64", "read", "write", "fsync", "pread", "pwrite", "close"]);
    for (const line of graph.split("\n")) {
        const row = /^\s*[+!:| ]*(\d+)\s+(.+?)\s+\(in [^)]+\)/.exec(line);
        if (!row) continue;
        const name = demangled.get(row[2]) ?? row[2], count = Number(row[1]);
        if (!Number.isSafeInteger(count) || count > 1000000000 || name.length > 500 || !/^[A-Za-z0-9_:<>$#& ,.*()\[\]{}=-]+$/.test(name)) continue;
        if (!/\b(?:connect_[a-z_]+|rvoip_[a-z_]+|tokio|rusqlite|parking_lot_core|parking_lot|std|core|alloc)::/.test(name) &&
            !/^sqlite3_[A-Za-z0-9_]+$/.test(name) && !waits.has(name)) continue;
        counts.set(name, (counts.get(name) ?? 0) + count);
    }
    const ranked = [...counts].sort((a, b) => b[1] - a[1]), entries = rows => rows.map(([symbol, stack_samples]) => ({ symbol, stack_samples }));
    return { format: "connect/native-server-stack-sample/1", raw_sample_sha256: createHash("sha256").update(text).digest("hex"),
        symbols_observed: counts.size, top_stacks: entries(ranked.slice(0, 48)),
        component_stacks: entries(ranked.filter(([name]) => /^<?(?:connect_[a-z_]+::|rvoip_[a-z_]+::|rusqlite::|sqlite3_)/.test(name)).slice(0, 32)),
        scope: "Inclusive sampled stack counts across server threads; not CPU seconds or a transport/quality pass. Only fixed runtime waits and known Rust/SQLite symbols; PIDs, paths and raw stacks excluded." };
}
export function normalizeNativeSymbol(name) {
    // GNU includes Rust crate disambiguators; Apple's LLVM omits them. They
    // identify compiler instances, not different sampled call sites here.
    return name.replace(/\b([A-Za-z_][A-Za-z0-9_]*)\[[0-9a-f]{16}\](?=::)/g, "$1");
}
export async function demangleNativeSymbols(text) {
    if (Buffer.byteLength(text) > 8 * 1024 * 1024) fail();
    // Feed only bounded Rust symbol tokens to the platform demangler. Raw stack
    // lines and their paths/addresses never enter command arguments or output.
    const names = [...new Set([...text.matchAll(/\s(_R[A-Za-z0-9_.$]{1,2000})\s+\(in connect\)/g)].map(row => row[1]))];
    if (names.length > 4096 || names.join("\n").length > 2 * 1024 * 1024) fail();
    if (!names.length) return new Map();
    const result = await new Promise((done, reject) => {
        const child = execFile("/usr/bin/c++filt", ["--no-strip-underscore"],
            { timeout: 3000, killSignal: "SIGKILL", maxBuffer: 4 * 1024 * 1024 },
            (error, stdout) => error ? reject(error) : done(stdout));
        child.stdin.on("error", () => {});
        child.stdin.end(names.join("\n") + "\n");
    });
    const lines = result.trimEnd().split("\n");
    if (lines.length !== names.length || lines.some(line => line.length > 16000)) fail();
    return new Map(names.map((name, index) => [name, normalizeNativeSymbol(lines[index])]));
}
function cpuMilliseconds(value) {
    const parts = value.split(":").map(Number);
    if (parts.length < 2 || parts.length > 3 || parts.some(n => !Number.isFinite(n) || n < 0)) fail();
    return Math.round(parts.reduce((sum, n) => sum * 60 + n, 0) * 1000);
}
export async function startNativeServerSample({ group, runtime, config, expectedSha256, output }) {
    if (process.platform !== "darwin" || !/^[0-9a-f]{64}$/.test(expectedSha256) ||
        ![runtime, config, output].every(p => p === resolve(p) && p.split(sep).includes(".local"))) fail();
    const directory = await lstat(output), executable = await lstat(runtime);
    if (!directory.isDirectory() || directory.mode & 0o077 || await realpath(output) !== output ||
        !executable.isFile() || executable.isSymbolicLink() || executable.size > 512 * 1024 * 1024 || await realpath(runtime) !== runtime) fail();
    const hash = createHash("sha256"); for await (const bytes of createReadStream(runtime)) hash.update(bytes);
    if (hash.digest("hex") !== expectedSha256) fail();
    const table = await execute("/bin/ps", ["-Ao", "pid=,pgid="], { timeout: 2000, maxBuffer: 1024 * 1024 });
    const ids = table.stdout.split("\n").map(line => line.trim().split(/\s+/).map(Number)).filter(([, pgid]) => pgid === group).map(([pid]) => pid);
    if (!ids.length || ids.length > 64) fail();
    const original = await execute("/bin/ps", ["-ww", "-p", ids.join(","), "-o", "pid=,pgid=,command="], { timeout: 2000, maxBuffer: 65536 });
    const command = runtime + " serve --config " + config, pid = selectOwnedServer(original.stdout, group, command);
    const resource = async () => {
        const { stdout } = await execute("/bin/ps", ["-p", String(pid), "-o", "pid=,pgid=,time=,rss="], { timeout: 2000, maxBuffer: 4096 });
        const row = stdout.trim().split(/\s+/);
        if (Number(row[0]) !== pid || Number(row[1]) !== group || row.length !== 4 || !/^\d+$/.test(row[3])) fail();
        return { cpu_ms: cpuMilliseconds(row[2]), resident_bytes: Number(row[3]) * 1024 };
    };
    const before = await resource(), began = performance.now(), path = join(output, "native-server-sample.txt");
    await writeFile(path, "", { flag: "wx", mode: 0o600 });
    // execFile's callback joins this original observer, including on its hard
    // timeout. It never kills or restarts the server being observed.
    const finished = new Promise(done => execFile("/usr/bin/sample", [String(pid), "8", "10", "-file", path],
        { timeout: 12000, killSignal: "SIGKILL", maxBuffer: 65536 }, error => done({ code: error?.code ?? 0, forced: error?.killed === true })));
    const observation = finished.then(async status => {
        const ended = performance.now();
        if (status.code !== 0) return { observed: false, observer_joined: true, forced: status.forced, reason: "native-sample-incomplete" };
        const current = await execute("/bin/ps", ["-ww", "-p", String(pid), "-o", "pid=,pgid=,command="], { timeout: 2000, maxBuffer: 4096 });
        if (selectOwnedServer(current.stdout, group, command) !== pid) fail();
        const after = await resource(), info = await lstat(path);
        if (!info.isFile() || info.isSymbolicLink() || info.size > 8 * 1024 * 1024 || info.mode & 0o077 || after.cpu_ms < before.cpu_ms) fail();
        const text = await readFile(path, "utf8"), demangled = await demangleNativeSymbols(text);
        return { observed: true, observer_joined: true, forced: false, seconds_requested: 8, interval_ms: 10,
            elapsed_ms: Math.round(ended - began), available_parallelism: availableParallelism(),
            server_cpu_delta_ms: after.cpu_ms - before.cpu_ms, resident_bytes_before: before.resident_bytes,
            resident_bytes_after: after.resident_bytes, rust_symbols_demangled: demangled.size,
            ...summarizeNativeSample(text, pid, demangled) };
    });
    void observation.catch(() => {});
    return Object.freeze({ finish: () => observation });
}
