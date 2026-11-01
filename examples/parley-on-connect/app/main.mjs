// Run from an ordinary installed-package consumer. No repository SDK import.
import { readFile, lstat, realpath } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { AppIntents } from "./intents.mjs";
import { AppGateway } from "./gateway.mjs";
import { startAppServer } from "./server.mjs";
import { appDiagnostic } from "./diagnostics.mjs";
import { startCpuProfile } from "./cpu-profile.mjs";

const fail = code => { const error = Error(code); error.code = code; throw error; };
async function digest(path) {
    const hash = createHash("sha256"); for await (const block of createReadStream(path)) hash.update(block); return hash.digest("hex");
}
function inside(root, name) {
    if (typeof name !== "string" || !name || name.length > 512 || name.includes("\\")) fail("invalid-app-configuration");
    const path = resolve(root, name), child = relative(root, path);
    if (!child || child.startsWith(".." + sep) || child === ".." || resolve(name) === name) fail("invalid-app-configuration");
    return path;
}
async function checkedAsset(root, name, expected, executable = false) {
    const path = inside(root, name), info = await lstat(path);
    if (!info.isFile() || info.isSymbolicLink() || await realpath(path) !== path ||
        info.size > (executable ? 512 * 1024 * 1024 : 65536) ||
        (executable ? !(info.mode & 0o100) : Boolean(info.mode & 0o077)) ||
        !/^[a-f0-9]{64}$/.test(expected) || await digest(path) !== expected) fail("app-asset-changed");
    return path;
}
async function main() {
    process.umask(0o077);
    const args = process.argv.slice(2);
    if (args.length !== 2 || args[0] !== "--config") fail("invalid-app-configuration");
    const configPath = resolve(args[1]), directory = dirname(configPath), info = await lstat(configPath);
    if (!directory.split(sep).includes(".local") || !info.isFile() || info.isSymbolicLink() ||
        info.size > 65536 || (info.mode & 0o077) || (await lstat(directory)).mode & 0o077 ||
        await realpath(directory) !== directory) fail("invalid-app-configuration");
    const config = JSON.parse(await readFile(configPath, "utf8"));
    if (config.profile !== "parley-local-app/1" || config.mode !== "fixture" || config.sdkVersion !== "0.1.0" ||
        !config.actors?.owner || Object.keys(config.actors).length > 5) fail("invalid-app-configuration");
    const entry = fileURLToPath(import.meta.resolve("@rudeless/connect"));
    const packageRoot = dirname(dirname(entry));
    const packageInfo = JSON.parse(await readFile(resolve(packageRoot, "package.json"), "utf8"));
    if (!(await lstat(packageRoot)).isDirectory() || (await lstat(packageRoot)).isSymbolicLink() ||
        !entry.split(sep).includes("node_modules") || packageInfo.name !== "@rudeless/connect" ||
        packageInfo.version !== config.sdkVersion) fail("installed-sdk-required");
    const { NodeClient, SdkRtpPacket } = await import("@rudeless/connect");
    const executable = await checkedAsset(directory, config.runtime.executable, config.runtime.sha256, true);
    const actors = {};
    for (const [role, actor] of Object.entries(config.actors)) {
        if (!actor.assets || !Object.hasOwn(actor.assets, actor.config) || Object.keys(actor.assets).length > 32) fail("invalid-app-configuration");
        for (const [name, expected] of Object.entries(actor.assets)) await checkedAsset(directory, name, expected);
        actors[role] = { identityId: actor.identityId, label: actor.label, config: inside(directory, actor.config) };
    }
    const journal = await AppIntents.open(resolve(directory, "application-state"));
    let profiler;
    try { profiler = process.env.PARLEY_APP_CPU_PROFILE === "1" ? await startCpuProfile(resolve(directory, "logs")) : null; }
    catch (error) { journal.close(); throw error; }
    const onError = (error, context) => process.stderr.write("PARLEY_APP_FAILURE " + JSON.stringify(appDiagnostic(error, context)) + "\n");
    const gateway = new AppGateway({ journal, actors, createRtpPacket: (sequence, bytes) => new SdkRtpPacket(sequence, bytes), createClient: actor => new NodeClient({ config: actor.config, executable }), onError });
    let app, closing;
    async function close(failed = false) {
        if (!closing) closing = (async () => {
            try { if (app) await app.close(); else await gateway.close(); }
            finally { try { await profiler?.stop(); } finally { journal.close(); } }
        })();
        try { await closing; process.exitCode = failed ? 1 : 0; }
        catch { process.stderr.write("Local app cleanup did not finish; preserve its original state.\n"); process.exitCode = 1; }
    }
    try {
        app = await startAppServer({ gateway, sdkDirectory: dirname(fileURLToPath(import.meta.resolve("@rudeless/connect/browser"))),
            port: config.port, roles: Object.entries(actors).map(([role, value]) => ({ role, label: value.label })), onError });
        process.once("SIGINT", () => { void close(); }); process.once("SIGTERM", () => { void close(); });
        process.once("uncaughtException", () => { void close(true); }); process.once("unhandledRejection", () => { void close(true); });
        process.stdout.write("Parley local synthetic app: " + app.origin + "\n");
    } catch { await close(true); throw Error(); }
}
await main().catch(() => { process.stderr.write("Local app could not start. Preserve its state and check setup.\n"); process.exitCode = 1; });
