// One-command synthetic local setup and run. Only public installed SDK/CLI calls
// create identities, enroll children or communicate. No model or provider runs.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { open, lstat, realpath, copyFile, chmod, access, readdir, readFile } from "node:fs/promises";
import { join, resolve, dirname, delimiter } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer, createConnection } from "node:net";
import { createSocket } from "node:dgram";
import { AppIntents } from "./intents.mjs";
import { privateDirectory, digest, loadOrCreatePlan, configureActors, verifyBootstrap } from "./setup.mjs";

const fail = code => { throw Object.assign(Error(code), { code }); };
let stage = "arguments";
const pause = (ms, signal) => new Promise((done, reject) => {
    signal?.throwIfAborted();
    const abort = () => { clearTimeout(timer); reject(signal.reason); };
    const timer = setTimeout(() => { signal?.removeEventListener("abort", abort); done(); }, ms);
    signal?.addEventListener("abort", abort, { once: true });
});
async function deadline(promise, ms, signal) {
    let timer, abort;
    const timeout = new Promise((done, reject) => {
        timer = setTimeout(() => done(null), ms);
        abort = () => reject(signal.reason);
        signal?.addEventListener("abort", abort, { once: true });
    });
    try { signal?.throwIfAborted(); return await Promise.race([promise, timeout]); }
    finally { clearTimeout(timer); signal?.removeEventListener("abort", abort); }
}
async function executablePath(name) {
    const choices = name.includes("/") ? [resolve(name)] : (process.env.PATH ?? "").split(delimiter).filter(Boolean).map(path => resolve(path, name));
    for (const path of choices) {
        try {
            await access(path, constants.X_OK); const actual = await realpath(path), info = await lstat(actual);
            if (info.isFile() && info.size > 0 && info.size <= 512 * 1024 * 1024) return actual;
        } catch (error) { if (!["ENOENT", "EACCES", "ENOTDIR"].includes(error.code)) throw error; }
    }
    fail("installed-connect-required");
}
async function unusedPort(udp = false) {
    const socket = udp ? createSocket("udp4") : createServer();
    try {
        await new Promise((done, reject) => { socket.once("error", reject); udp ? socket.bind(0, "127.0.0.1", done) : socket.listen(0, "127.0.0.1", done); });
        return socket.address().port;
    } finally { await new Promise(done => socket.close(done)); }
}
async function main() {
    process.umask(0o077);
    let state = resolve(".local/parley"), selected = "connect";
    const args = process.argv.slice(2), seen = new Set();
    for (let index = 0; index < args.length; index += 2) {
        const flag = args[index], value = args[index + 1];
        if (!["--state", "--connect"].includes(flag) || !value || seen.has(flag)) fail("invalid-app-arguments");
        seen.add(flag); if (flag === "--state") state = resolve(value); else selected = value;
    }
    // Resolve a regular installed package before creating any Connect state.
    stage = "installed-sdk";
    const entry = fileURLToPath(import.meta.resolve("@rudeless/connect"));
    const packageRoot = dirname(dirname(entry));
    const packageInfo = JSON.parse(await readFile(join(packageRoot, "package.json"), "utf8"));
    if (!entry.split("/").includes("node_modules") || (await lstat(packageRoot)).isSymbolicLink() ||
        packageInfo.name !== "@rudeless/connect" || packageInfo.version !== "0.1.0") fail("installed-sdk-required");
    const { NodeAdministrativeClient, NodeClient } = await import("@rudeless/connect");
    const installed = await executablePath(selected), binarySha = await digest(installed);
    await privateDirectory(state);
    const entries = await readdir(state);
    if (entries.length && !entries.includes("setup.json")) fail("app-setup-incomplete");
    // Reuse the journal's transactional live-owner fencing. This is a separate,
    // empty application journal, never Connect's own authoritative database.
    const ownership = await AppIntents.open(join(state, "run-ownership"));
    const controller = new AbortController(), children = [], runId = randomUUID(); let sequence = 0, forced = false;
    const stop = () => controller.abort(Object.assign(Error(), { code: "app-stopped" }));
    process.once("SIGINT", stop); process.once("SIGTERM", stop);
    async function child(file, argv, name, maximum = 2 * 1024 * 1024) {
        controller.signal.throwIfAborted();
        const path = join(state, "logs", runId + "-" + sequence++ + "-" + name + ".log");
        const log = await open(path, "wx", 0o600);
        const env = Object.fromEntries(["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "SYSTEMROOT"].filter(key => process.env[key] !== undefined).map(key => [key, process.env[key]]));
        const processChild = spawn(file, argv, { env, stdio: ["ignore", "pipe", "pipe"] });
        const record = { process: processChild, failed: null, bytes: 0, writing: Promise.resolve() };
        let diagnosticBuffer = "";
        record.done = new Promise(done => {
            processChild.once("error", error => done({ error }));
            processChild.once("close", (code, signal) => done({ code, signal }));
        });
        for (const stream of [processChild.stdout, processChild.stderr]) stream.on("data", bytes => {
            record.bytes += bytes.length;
            if (record.bytes > maximum) { record.failed = "app-log-capacity"; processChild.kill("SIGTERM"); return; }
            record.writing = record.writing.then(() => log.writeFile(bytes)).catch(() => { record.failed = "app-log-write"; processChild.kill("SIGTERM"); });
            if (name === "app" && stream === processChild.stderr) {
                diagnosticBuffer += bytes.toString();
                if (diagnosticBuffer.length > 8192) diagnosticBuffer = "";
                let end;
                while ((end = diagnosticBuffer.indexOf("\n")) >= 0) {
                    const line = diagnosticBuffer.slice(0, end); diagnosticBuffer = diagnosticBuffer.slice(end + 1);
                    if (/^PARLEY_APP_FAILURE \{[^\n]+\}$/.test(line)) process.stderr.write(line + "\n");
                }
            }
        });
        record.finished = record.done.then(async result => {
            try { await record.writing; await log.sync(); }
            catch { record.failed = "app-log-write"; }
            finally { await log.close().catch(() => { record.failed = "app-log-write"; }); }
            return result;
        });
        children.push(record); return record;
    }
    async function bounded(record, timeoutMs) {
        const settled = await deadline(record.finished, timeoutMs, controller.signal);
        if (!settled || settled.code !== 0 || settled.signal || settled.error || record.failed) fail("app-command-incomplete");
    }
    async function ready(record, port) {
        const until = Date.now() + 15000;
        while (Date.now() < until) {
            controller.signal.throwIfAborted();
            if (record.process.exitCode !== null || record.process.signalCode !== null || record.failed) fail("app-server-unavailable");
            const connected = await new Promise(done => {
                const socket = createConnection({ host: "127.0.0.1", port }); let settled = false;
                const finish = value => { if (!settled) { settled = true; socket.destroy(); done(value); } };
                socket.setTimeout(250); socket.on("connect", () => finish(true)); socket.on("error", () => finish(false)); socket.on("timeout", () => finish(false));
            });
            if (connected) return;
            await pause(50, controller.signal);
        }
        fail("app-server-unavailable");
    }
    try {
        stage = "runtime-and-plan";
        await privateDirectory(join(state, "runtime")); await privateDirectory(join(state, "logs"));
        const runtime = join(state, "runtime", "connect");
        let existing = false;
        try { await lstat(runtime); existing = true; } catch (error) { if (error.code !== "ENOENT") throw error; }
        if (!existing) {
            // Copy-on-write where supported; the user's installed inode never
            // becomes mutable application state through a hard link.
            await copyFile(installed, runtime, constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE);
            await chmod(runtime, 0o700);
            const copied = await open(runtime, "r"); try { await copied.sync(); } finally { await copied.close(); }
        }
        const info = await lstat(runtime);
        if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || info.size < 1 || info.size > 512 * 1024 * 1024 ||
            (info.mode & 0o077) || !(info.mode & 0o100) ||
            await realpath(runtime) !== runtime || await digest(runtime) !== binarySha || await digest(installed) !== binarySha) fail("app-runtime-changed");
        const ports = { native: await unusedPort(true), http: await unusedPort(), ui: await unusedPort() };
        // HTTP and UI are distinct TCP listeners, even if sequential ephemeral
        // reservations happen to return the same number.
        for (let attempt = 0; ports.ui === ports.http && attempt < 8; attempt++) ports.ui = await unusedPort();
        if (ports.ui === ports.http) fail("app-port-reservation");
        const plan = await loadOrCreatePlan(state, binarySha, ports);
        const execute = async args => bounded(await child(runtime, args, args[0], 128 * 1024), 30000);
        const installation = join(state, "installation");
        let initialized = false;
        try { await lstat(installation); initialized = true; } catch (error) { if (error.code !== "ENOENT") throw error; }
        stage = "initialize";
        if (!initialized) await execute(["init", "--directory", installation, "--bind", "127.0.0.1:" + plan.ports.native,
            "--http-bind", "127.0.0.1:" + plan.ports.http]);
        // Only the public initializer owns these files. An incomplete existing
        // installation is preserved and refused, never replaced or repaired.
        await verifyBootstrap(state, !initialized);
        stage = "server-startup";
        const server = await child(runtime, ["serve", "--config", join(installation, "server.toml")], "server");
        await ready(server, plan.ports.http);
        stage = "scoped-actor-setup";
        await configureActors({ directory: state, plan, execute, signal: controller.signal,
            createAdministrativeClient: config => new NodeAdministrativeClient({ config, executable: runtime }),
            createParticipantClient: config => new NodeClient({ config, executable: runtime }) });
        stage = "application-startup";
        const app = await child(process.execPath, [fileURLToPath(new URL("./main.mjs", import.meta.url)), "--config", join(state, "app.json")], "app");
        await ready(app, plan.ports.ui);
        controller.signal.throwIfAborted();
        process.stdout.write("Parley local synthetic app: http://127.0.0.1:" + plan.ports.ui + "\n");
        stage = "application-running";
        // A stopped component ends this owned run. Never silently respawn it.
        await Promise.race([server.finished, app.finished, new Promise(done => controller.signal.addEventListener("abort", done, { once: true }))]);
        if (!controller.signal.aborted) fail("app-component-stopped");
    } finally {
        controller.abort();
        for (const record of [...children].reverse()) {
            const running = record.process.exitCode === null && record.process.signalCode === null;
            if (running) record.process.kill("SIGTERM");
            let result = await deadline(record.finished, 10000);
            if (!result) { forced = true; record.process.kill("SIGKILL"); result = await record.finished; }
            // A command that already exited unsuccessfully is the primary
            // failure, not evidence that its joined process leaked at cleanup.
            if (running && (result.code !== 0 || result.signal || result.error || record.failed)) forced = true;
        }
        process.removeListener("SIGINT", stop); process.removeListener("SIGTERM", stop); ownership.close();
        if (forced) fail("app-cleanup-incomplete");
    }
}
await main().catch(error => {
    if (error.code === "app-stopped") return;
    const location = /\/app\/([a-z_]+\.mjs):(\d+):(\d+)/.exec(String(error.stack));
    const code = typeof error.code === "string" && /^app-[a-z-]{1,48}$/.test(error.code) ? error.code : "setup-action-failed";
    const kinds = ["validation", "configuration", "authentication", "authorization", "deadline", "cancelled", "protocol", "remote-error", "transport-uncertain"];
    process.stderr.write("PARLEY_SETUP_FAILURE " + JSON.stringify({ stage, code,
        kind: kinds.includes(error.kind) ? error.kind : null,
        location: location ? { file: location[1], line: Number(location[2]), column: Number(location[3]) } : null }) + "\n");
    process.stderr.write("Local setup/run did not complete during " + stage + ". Preserve its original .local state and inspect its private logs before restarting.\n");
    process.exitCode = 1;
});
