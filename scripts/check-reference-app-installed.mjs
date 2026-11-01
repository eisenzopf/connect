// Real installed SDK + production Connect, one-command app setup and browser.
// Synthetic local control only; never enables media, models or providers.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const root = fileURLToPath(new URL("../", import.meta.url));
const base = resolve(process.argv[2] ?? ""), output = resolve(process.argv[3] ?? "");
assert.ok(base.startsWith(join(root, ".local") + "/") && output.startsWith(join(root, ".local") + "/") && base !== output);
await mkdir(output, { recursive: false, mode: 0o700 });
const installation = JSON.parse(await readFile(join(base, "installation.json"), "utf8"));
assert.equal(installation.passed, true);
const consumer = installation.consumer, state = join(base, "consumer/.local/parley");
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
async function tree(directory) {
    const result = {};
    async function walk(path, prefix = "") {
        for (const item of (await readdir(path, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
            assert.ok(!item.isSymbolicLink()); const name = prefix + item.name;
            if (item.isDirectory()) await walk(join(path, item.name), name + "/");
            else { assert.ok(item.isFile()); result[name] = sha(await readFile(join(path, item.name))); }
        }
    }
    await walk(directory); return result;
}
const sourceInputs = { app: await tree(join(root, "examples/parley-on-connect/app")),
    installed_sdk: await tree(join(consumer, "node_modules/@rudeless/connect")),
    checker: sha(await readFile(fileURLToPath(import.meta.url))),
    installer: sha(await readFile(join(root, "scripts/install-reference-app-candidate.py"))) };
assert.deepEqual(await tree(join(consumer, "app")), sourceInputs.app);
const report = { format: "connect/reference-app-installed-control/1", passed: false, checks: [],
    source_revision: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
    source_inputs: sourceInputs, runtime_source_revision: installation.source_revision,
    binary_sha256: installation.binary_sha256, node_archive_sha256: installation.node_archive_sha256,
    node: process.version, platform: process.platform, task_complete: false, whole_gate_qualified: false,
    scope: "Actual production G6-069 installed runtime/Node SDK, current one-command application setup and owned browser. Synthetic local control, invitations, text, offline recovery and restart. No negotiated media, model, provider or WAN qualification." };
let processChild, finished, context, browser, page, release, stage = "startup";
const check = value => { report.checks.push(value); stage = value; process.stdout.write(JSON.stringify({ check: value }) + "\n"); };
async function deadline(promise, ms) {
    let timer;
    try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Object.assign(Error(), { code: "fixture-deadline" })), ms); })]); }
    finally { clearTimeout(timer); }
}
function groupMembers(pid) {
    return execFileSync("ps", ["-Ao", "pid=,pgid="], { encoding: "utf8" }).trim().split("\n").map(line => line.trim().split(/\s+/).map(Number)).filter(([, group]) => group === pid).map(([member]) => member);
}
async function start(number) {
    stage = "one-command-startup-" + number;
    processChild = spawn(process.execPath, [join(consumer, "app/run.mjs"), "--state", state, "--connect", installation.executable],
        { cwd: consumer, detached: true, env: Object.fromEntries(["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "SYSTEMROOT"].filter(key => process.env[key] !== undefined).map(key => [key, process.env[key]])), stdio: ["ignore", "pipe", "pipe"] });
    const original = processChild;
    finished = new Promise(done => { original.once("error", error => done({ error: error.code })); original.once("close", (code, signal) => done({ code, signal })); });
    let stdout = "", stderr = "", ready;
    const line = new Promise(done => { ready = done; });
    original.stdout.on("data", bytes => { stdout += bytes.toString(); if (stdout.length > 65536) original.kill("SIGTERM"); else {
        const match = /Parley local synthetic app: (http:\/\/127\.0\.0\.1:[0-9]+)\n/.exec(stdout); if (match) ready(match[1]);
    } });
    original.stderr.on("data", bytes => { stderr += bytes.toString(); if (stderr.length > 65536) original.kill("SIGTERM"); });
    try {
        const origin = await deadline(Promise.race([line, finished.then(() => { throw Object.assign(Error(), { code: "one-command-startup-failed" }); })]), 180000);
        report["owned_group_size_" + number] = groupMembers(original.pid).length;
        return origin;
    } finally { await writeFile(join(output, "run-" + number + "-output.log"), stdout + stderr, { mode: 0o600 }); }
}
async function stop() {
    if (!processChild) return;
    const original = processChild;
    original.kill("SIGTERM");
    const result = await deadline(finished, 15000);
    assert.deepEqual(result, { code: 0, signal: null });
    assert.deepEqual(groupMembers(original.pid), []);
    processChild = null;
}
try {
    const { chromium } = await import(pathToFileURL(join(root, "scripts/video/observer-tools/node_modules/playwright/index.mjs")).href);
    const origin = await start(1); check("one-command-four-actor-scoped-setup-ready");
    context = await chromium.launchPersistentContext(join(output, "profile"), { headless: true,
        ...(process.env.CONNECT_GATE6_CHROME ? { executablePath: process.env.CONNECT_GATE6_CHROME } : {}),
        viewport: { width: 1440, height: 1100 }, args: ["--disable-background-networking", "--disable-component-update", "--no-first-run", "--disable-sync", "--disable-extensions"] });
    browser = context.browser(); report.browser = browser.version(); page = context.pages()[0];
    await context.grantPermissions([], { origin });
    const errors = []; page.on("pageerror", error => errors.push(error.name));
    await page.goto(origin); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    assert.equal(await page.evaluate(() => document.cookie), "");
    assert.match(await page.locator("#media-capability").textContent(), /QUIC/); check("actual-installed-native-SDK-control-connected");
    // Native checkboxes and buttons, including the real participant selection.
    await page.getByRole("checkbox", { name: "Assistant (simulated)", exact: true }).focus(); await page.keyboard.press("Space");
    await page.getByRole("checkbox", { name: "Organizer", exact: true }).focus(); await page.keyboard.press("Space");
    await page.getByRole("button", { name: "Create conversation", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector("#participants").textContent.includes("Organizer") && !document.querySelector("#message-form button").disabled);
    check("keyboard-participant-selection-and-conversation-creation");
    const text = "<img src=x onerror=alert(1)> installed SDK text";
    await page.locator("#message").fill(text); await page.getByRole("button", { name: "Send message", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(text => document.querySelector("#timeline").textContent.includes(text), text);
    assert.equal(await page.locator("#timeline img").count(), 0); check("actual-authorized-text-safe-timeline");
    await page.getByRole("button", { name: "Invite", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => !document.querySelector("#create-form button").disabled && document.querySelector("#sessions li"));
    // Changing the synthetic receiving perspective is fixture preparation.
    await page.locator("#actor").selectOption("organizer");
    await page.waitForFunction(() => document.querySelector("#actor").value === "organizer" && document.querySelector("#conversations button"));
    await page.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.getByRole("button", { name: "Accept", exact: true }).waitFor();
    await page.getByRole("button", { name: "Accept", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => [...document.querySelectorAll("#operations li")].some(value => value.textContent.includes("Accept invitation · completed")));
    check("keyboard-incoming-invitation-and-actual-acceptance");
    for (const kind of ["microphone", "camera"]) {
        assert.equal(await page.evaluate(async name => (await navigator.permissions.query({ name })).state, kind), "denied");
        await page.locator("#" + kind).focus(); await page.keyboard.press("Enter");
        await page.waitForFunction(kind => document.querySelector("#permission").textContent.includes(kind + " permission denied"), kind);
    }
    check("actual-browser-microphone-and-camera-refusal");
    await page.screenshot({ path: join(output, "desktop.png"), fullPage: true });
    await page.setViewportSize({ width: 320, height: 850 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true); check("320px-no-horizontal-overflow");
    const marker = "one original installed message after lost HTTP response";
    let observed;
    const accepted = new Promise(done => { observed = done; });
    let originalOperationId;
    await page.route("**/api/submit", async route => {
        originalOperationId = route.request().postDataJSON().operationId;
        const response = await route.fetch(), result = await response.json();
        assert.equal(result.operation.operationId, originalOperationId); assert.equal(result.operation.state, "completed");
        observed(); await new Promise(done => { release = done; }); await route.abort().catch(() => {});
    }, { times: 1 });
    await page.locator("#message").fill(marker); await page.getByRole("button", { name: "Send message", exact: true }).focus(); await page.keyboard.press("Enter");
    await deadline(accepted, 15000); await context.setOffline(true);
    await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Offline"));
    const stored = await page.evaluate(() => Object.fromEntries(Object.entries(localStorage)));
    assert.ok(!JSON.stringify(stored).includes(marker) && !JSON.stringify(stored).includes(text));
    assert.equal(JSON.parse(stored["parley-original-action-organizer"]).operationId, originalOperationId);
    release(); release = null; check("actual-accepted-original-survives-lost-response-and-offline");
    await context.setOffline(false); await page.reload();
    await page.getByRole("button", { name: "Recover original action", exact: true }).waitFor();
    await page.getByRole("button", { name: "Reconnect", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    await page.getByRole("button", { name: "Recover original action", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => localStorage.getItem("parley-original-action-organizer") === null);
    await page.waitForFunction(marker => document.querySelector("#timeline").textContent.includes(marker), marker);
    assert.equal(await page.locator("#timeline li").filter({ hasText: marker }).count(), 1); check("keyboard-original-status-recovery-one-visible-message");
    await page.screenshot({ path: join(output, "narrow.png"), fullPage: true });
    const setupSha = sha(await readFile(join(state, "setup.json"))), appSha = sha(await readFile(join(state, "app.json")));
    await stop(); check("original-app-server-and-all-native-helper-processes-joined");
    const restarted = await start(2); assert.equal(restarted, origin);
    assert.equal(sha(await readFile(join(state, "setup.json"))), setupSha); assert.equal(sha(await readFile(join(state, "app.json"))), appSha);
    await page.reload(); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    await page.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(marker => document.querySelector("#timeline").textContent.includes(marker), marker);
    assert.equal(await page.locator("#timeline li").filter({ hasText: marker }).count(), 1); check("same-command-restart-preserves-original-setup-and-conversation");
    assert.deepEqual(errors, []);
    await context.close(); context = null; assert.equal(browser.isConnected(), false); check("original-owned-browser-joined");
    await stop(); check("restarted-app-server-and-all-native-helper-processes-joined");
    report.passed = true;
} catch (error) {
    report.failure = { stage, name: error.name, code: typeof error.code === "string" ? error.code : null };
    if (page && !page.isClosed()) {
        report.failure_notice = await page.locator("#notice").textContent().catch(() => null);
        await page.screenshot({ path: join(output, "failure.png"), fullPage: true }).catch(() => {});
    }
    process.exitCode = 1;
} finally {
    release?.();
    if (context) await context.close().catch(() => { report.browser_cleanup_failed = true; report.passed = false; });
    if (processChild) await stop().catch(async () => {
        report.process_cleanup_failed = true; report.passed = false; processChild?.kill("SIGKILL");
        await deadline(finished, 5000).catch(() => {});
    });
    report.sources_unchanged = JSON.stringify(await tree(join(root, "examples/parley-on-connect/app"))) === JSON.stringify(sourceInputs.app) &&
        JSON.stringify(await tree(join(consumer, "app"))) === JSON.stringify(sourceInputs.app) &&
        JSON.stringify(await tree(join(consumer, "node_modules/@rudeless/connect"))) === JSON.stringify(sourceInputs.installed_sdk) &&
        sha(await readFile(fileURLToPath(import.meta.url))) === sourceInputs.checker &&
        sha(await readFile(join(root, "scripts/install-reference-app-candidate.py"))) === sourceInputs.installer;
    if (!report.sources_unchanged) report.passed = false;
    if (!report.passed) process.exitCode = 1;
    await writeFile(join(output, "summary.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
    process.stdout.write(JSON.stringify({ passed: report.passed, checks: report.checks.length, failure: report.failure }) + "\n");
}
