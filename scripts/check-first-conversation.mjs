// Execute the documented kit commands and visible user flow outside the checkout.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile, cp, rm, realpath, stat } from "node:fs/promises";
import { join, resolve, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
const root = fileURLToPath(new URL("../", import.meta.url));
const archive = resolve(process.argv[2] ?? ""), kitProof = resolve(process.argv[3] ?? ""), output = resolve(process.argv[4] ?? "");
assert.equal(process.argv.length, 5);
assert.ok(output.startsWith(join(root, ".local") + "/"));
await mkdir(output, { mode: 0o700 }); process.umask(0o077);
const sha = value => createHash("sha256").update(value).digest("hex");
const proof = JSON.parse(await readFile(kitProof, "utf8"));
assert.equal(proof.passed, true); assert.equal(sha(await readFile(archive)), proof.archive_sha256);
const temporary = await mkdtemp(join(tmpdir(), "connect-first-"));
assert.ok(!(await realpath(temporary)).startsWith(await realpath(root) + "/"));
const consumer = join(temporary, "connect-first"), home = join(temporary, "home"); await mkdir(home);
const env = { PATH: process.env.PATH, HOME: home, TMPDIR: temporary, LANG: "en_US.UTF-8" };
const report = { format: "connect/first-conversation-rehearsal/1", passed: false,
    source_revision: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
    kit_source_revision: proof.source_revision, archive_sha256: proof.archive_sha256,
    platform: process.platform, architecture: process.arch, node: process.version,
    consumer_outside_source_checkout: true, empty_home: true, empty_npm_cache: true,
    commands: [], checks: [], task_complete: false, whole_gate_qualified: false };
let child, joined, browser, context, stage = "create-project", sequence = 0;
const began = performance.now();
function check(name) { report.checks.push(name); stage = name; }
async function bounded(promise, ms) {
    let timer;
    try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error("quickstart-deadline")), ms); })]); }
    finally { clearTimeout(timer); }
}
async function run(argv, cwd = consumer, expected = 0) {
    const processChild = spawn(argv[0], argv.slice(1), { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    let bytes = Buffer.alloc(0);
    for (const stream of [processChild.stdout, processChild.stderr]) stream.on("data", body => {
        bytes = Buffer.concat([bytes, body]); if (bytes.length > 2 * 1024 * 1024) processChild.kill("SIGTERM");
    });
    const exit = await bounded(new Promise((done, reject) => { processChild.once("error", reject); processChild.once("close", code => done(code)); }), 120000);
    await writeFile(join(output, "command-" + sequence++ + ".log"), bytes, { mode: 0o600 });
    report.commands.push({ argv: argv.map(value => value === archive ? "<reviewed-kit.tar.gz>" : value), cwd_outside_checkout: true, exit_code: exit });
    assert.equal(exit, expected); return bytes.toString();
}
const groupMembers = pid => execFileSync("ps", ["-Ao", "pid=,pgid="], { encoding: "utf8" }).trim().split("\n")
    .map(line => line.trim().split(/\s+/).map(Number)).filter(([, group]) => group === pid).map(([id]) => id);
async function start() {
    stage = "node-app-run";
    child = spawn(process.execPath, ["app/run.mjs"], { cwd: consumer, env, detached: true, stdio: ["ignore", "pipe", "pipe"] });
    let bytes = "", ready;
    const started = new Promise(done => { ready = done; });
    joined = new Promise((done, reject) => { child.once("error", reject); child.once("close", (code, signal) => done({ code, signal })); });
    void joined.catch(() => {});
    for (const stream of [child.stdout, child.stderr]) stream.on("data", body => {
        bytes += body; if (bytes.length > 2 * 1024 * 1024) child.kill("SIGTERM");
        const match = /Parley local synthetic app: (http:\/\/127\.0\.0\.1:[0-9]+)\n/.exec(bytes); if (match) ready(match[1]);
    });
    const url = await bounded(Promise.race([started, joined.then(() => { throw Error("quickstart-app-startup"); })]), 180000);
    await writeFile(join(output, "app-start-" + sequence++ + ".log"), bytes, { mode: 0o600 });
    report.commands.push({ argv: ["node", "app/run.mjs"], cwd_outside_checkout: true, ready: true }); return url;
}
async function stop() {
    if (!child) return;
    const original = child; process.kill(-original.pid, "SIGINT");
    const exit = await bounded(joined, 15000);
    assert.ok(exit.code === 0 || exit.code === 130 || exit.signal === "SIGINT");
    const until = Date.now() + 10000;
    while (groupMembers(original.pid).length && Date.now() < until) await new Promise(done => setTimeout(done, 50));
    assert.deepEqual(groupMembers(original.pid), []); child = null;
    report.commands.push({ action: "Ctrl-C", original_group_joined: true, exit });
}
try {
    await run(["mkdir", "connect-first"], temporary);
    await run(["tar", "-xzf", archive, "--strip-components=1"]);
    assert.equal(sha(await readFile(join(consumer, "quickstart-manifest.json"))), proof.manifest_sha256);
    await run([process.execPath, "verify.mjs"]); check("reviewed-kit-and-native-ABI-verified");
    const manifest = JSON.parse(await readFile(join(consumer, "quickstart-manifest.json"), "utf8"));
    const deps = Object.keys(manifest.files).filter(name => /^sdk\/dependencies\/[^/]+\.tgz$/.test(name)).sort();
    assert.equal(deps.length, 2);
    await run(["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--cache", ".local/npm-cache", "./sdk/rudeless-connect-0.1.0.tgz", ...deps.map(name => "./" + name)]);
    check("cold-offline-public-SDK-install");
    env.PATH = join(consumer, "runtime/bin") + ":" + env.PATH;
    report.commands.push({ action: 'export PATH="$PWD/runtime/bin:$PATH"', cwd_outside_checkout: true });
    const origin = await start(); report.ready_elapsed_ms = performance.now() - began;
    const { chromium } = await import(pathToFileURL(join(root, "scripts/video/observer-tools/node_modules/playwright/index.mjs")).href);
    const chrome = process.env.CONNECT_GATE6_CHROME ?? chromium.executablePath(); report.browser_launcher_sha256 = sha(await readFile(chrome));
    browser = await chromium.launch({ executablePath: chrome, headless: true }); report.browser = browser.version();
    context = await browser.newContext({ viewport: { width: 1440, height: 1100 } }); const page = await context.newPage();
    await page.goto(origin); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    await page.getByRole("checkbox", { name: "Assistant (simulated)", exact: true }).uncheck();
    await page.getByRole("checkbox", { name: "Organizer", exact: true }).check();
    await page.getByRole("button", { name: "Create conversation", exact: true }).click();
    await page.waitForFunction(() => !document.querySelector("#message-form button").disabled);
    await page.locator("#message").fill("Hello Connect"); await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.getByText("Hello Connect", { exact: true }).waitFor();
    await page.getByRole("radio", { name: "Organizer", exact: true }).check();
    await page.getByRole("button", { name: "Conversation 1", exact: true }).click();
    await page.getByText("Hello Connect", { exact: true }).waitFor();
    await page.locator("#message").fill("Hello back"); await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.getByText("Hello back", { exact: true }).waitFor();
    await page.getByRole("radio", { name: "Owner", exact: true }).check();
    await page.getByRole("button", { name: "Conversation 1", exact: true }).click();
    for (const text of ["Hello Connect", "Hello back"]) {
        await page.getByText(text, { exact: true }).waitFor();
        assert.equal(await page.locator("#timeline li").filter({ hasText: text }).count(), 1);
    }
    report.message_and_reply_elapsed_ms = performance.now() - began; assert.ok(report.message_and_reply_elapsed_ms < 600000);
    check("visible-Owner-Organizer-message-reply-without-copied-IDs");
    await stop(); const restarted = await start(); assert.equal(restarted, origin);
    await page.reload(); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    await page.getByRole("button", { name: "Conversation 1", exact: true }).click();
    for (const text of ["Hello Connect", "Hello back"]) {
        await page.getByText(text, { exact: true }).waitFor(); assert.equal(await page.locator("#timeline li").filter({ hasText: text }).count(), 1);
    }
    check("same-command-restart-retains-one-message-and-one-reply");
    await stop(); await context.close(); context = null; await browser.close(); browser = null;
    await cp(join(consumer, ".local/parley"), join(output, "private-state"), { recursive: true });
    await run([process.execPath, "verify.mjs"]); check("kit-bytes-unchanged-after-complete-journey");
    report.passed = true;
} catch (error) {
    report.failure = { stage, name: error.name, message: String(error.message).slice(0, 500) }; report.failed_consumer = consumer; process.exitCode = 1;
} finally {
    if (context) await context.close().catch(() => {}); if (browser) await browser.close().catch(() => {});
    if (child) await stop().catch(() => { report.forced_cleanup = true; process.kill(-child.pid, "SIGKILL"); report.passed = false; });
    report.elapsed_ms = performance.now() - began;
    await writeFile(join(output, "summary.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
    if (report.passed) await rm(temporary, { recursive: true });
    console.log(JSON.stringify({ passed: report.passed, checks: report.checks.length, failure: report.failure }));
}
