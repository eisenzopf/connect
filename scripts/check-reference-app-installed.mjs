// Real installed SDK + production Connect, one-command app setup and browser.
// Synthetic local control, native audio negotiation and two owned fake-device
// browser audio capture/playback paths. No model, provider or personal devices.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { newSetupPlan, digest } from "../examples/parley-on-connect/app/setup.mjs";
const root = fileURLToPath(new URL("../", import.meta.url));
const base = resolve(process.argv[2] ?? ""), output = resolve(process.argv[3] ?? "");
assert.ok(base.startsWith(join(root, ".local") + "/") && output.startsWith(join(root, ".local") + "/") && base !== output);
await mkdir(output, { recursive: false, mode: 0o700 });
const installation = JSON.parse(await readFile(join(base, "installation.json"), "utf8"));
assert.equal(installation.passed, true);
const producers = {
    "connect/reference-app-candidate-installation/1": { kind: "sealed-sdk-candidate", installer: "scripts/install-reference-app-candidate.py" },
    "connect/reference-app-local-components/1": { kind: "verified-local-components", installer: "scripts/install-reference-app-components.py" },
};
assert.ok(Object.hasOwn(producers, installation.format), "known-actual-installation-producer-required");
const producer = producers[installation.format];
if (producer.kind === "verified-local-components") {
    assert.equal(installation.complete_sdk_candidate_qualified, false);
    assert.equal(installation.source_inputs_matched, true);
    assert.equal(installation.regular_offline_install, true);
}
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
    runtime: await digest(installation.executable),
    checker: sha(await readFile(fileURLToPath(import.meta.url))),
    installer: sha(await readFile(join(root, producer.installer))) };
if (producer.kind === "verified-local-components") assert.equal(installation.installer_sha256, sourceInputs.installer);
assert.deepEqual(await tree(join(consumer, "app")), sourceInputs.app);
assert.equal(sourceInputs.runtime, installation.binary_sha256);
const report = { format: "connect/reference-app-installed-control/1", passed: false, checks: [],
    source_revision: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
    source_inputs: sourceInputs, runtime_source_revision: installation.source_revision,
    installation_kind: producer.kind, complete_sdk_candidate_qualified: producer.kind === "sealed-sdk-candidate",
    installation_proof_sha256: sha(await readFile(join(base, "installation.json"))),
    binary_sha256: installation.binary_sha256, node_archive_sha256: installation.node_archive_sha256,
    node: process.version, platform: process.platform, task_complete: false, whole_gate_qualified: false,
    scope: (producer.kind === "sealed-sdk-candidate" ? "Actual production G6-069 installed runtime/Node SDK." :
        "Actual production runtime and public Node SDK installed from independently verified local archives; complete SDK candidate remains unqualified.") +
        " Current one-command application setup and owned browser. Synthetic local control, invitations, text, offline recovery, restart and two native audio Connection negotiation/subscription/gate/end controls. Owned synthetic browser audio capture/decoded playback when its explicit checks pass. Identified video capture/playback is separately reported when its explicit checks pass. Full lifecycle/failure, model, provider and WAN qualification remain open." };
let processChild, finished, context, browser, page, release, chrome, stage = "startup";
const apiReads = new Set(), mediaBrowsers = [], mediaPages = [];
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
    original.stderr.on("data", bytes => {
        stderr += bytes.toString(); if (stderr.length > 65536) original.kill("SIGTERM");
        report.app_failures = [...stderr.matchAll(/^PARLEY_APP_FAILURE (\{[^\n]+\})$/gm)].slice(0, 16).map(match => JSON.parse(match[1]));
    });
    try {
        const origin = await deadline(Promise.race([line, finished.then(() => { throw Object.assign(Error(), { code: "one-command-startup-failed" }); })]), 180000);
        report["owned_group_size_" + number] = groupMembers(original.pid).length;
        return origin;
    } finally {
        await writeFile(join(output, "run-" + number + "-output.log"), stdout + stderr, { mode: 0o600 });
        const diagnostic = /^PARLEY_SETUP_FAILURE (\{[^\n]+\})$/m.exec(stderr);
        if (diagnostic) report["startup_failure_" + number] = JSON.parse(diagnostic[1]);
    }
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
    stage = "actual-public-provisioning-validation";
    const sample = newSetupPlan(installation.binary_sha256, { native: 24431, http: 24432, ui: 24433 });
    for (const actor of sample.actors) {
        const command = { type: "identity", operation_id: actor.identityOperationId, alias: actor.alias, kind: actor.kind, display_name: actor.label };
        const validate = input => JSON.parse(execFileSync(installation.executable, ["sdk-validate-provisioning"], {
            input: JSON.stringify(input), timeout: 2000, maxBuffer: 128 * 1024, encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }));
        const before = validate({ ...command, alias: command.alias.replaceAll("-", "_") });
        assert.equal(before.profile, "connect-sdk-provisioning-validation/1"); assert.equal(before.valid, false);
        const result = validate(command);
        assert.equal(result.profile, "connect-sdk-provisioning-validation/1"); assert.equal(result.valid, true);
        assert.equal(result.value.alias, actor.alias);
    }
    check("actual-public-helper-validates-all-generated-identity-intents");
    const envelope = { v: 2, id: "snapshot-contract-probe", ts: new Date().toISOString(), type: "conversation.get", cid: "conversation-contract-probe" };
    for (const [limit, valid] of [[128, false], [127, true], [undefined, true]]) {
        const result = JSON.parse(execFileSync(installation.executable, ["sdk-validate-envelope"], {
            input: JSON.stringify({ ...envelope, payload: limit === undefined ? {} : { limit } }), timeout: 2000,
            maxBuffer: 128 * 1024, encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }));
        assert.equal(result.profile, "connect-sdk-validation/1"); assert.equal(result.valid, valid);
    }
    check("actual-public-helper-refuses-128-and-accepts-127-and-server-default-pages");
    const { chromium } = await import(pathToFileURL(join(root, "scripts/video/observer-tools/node_modules/playwright/index.mjs")).href);
    // Use the installed full Chromium browser in headless mode. The separate
    // headless shell does not support this file-backed capture without a fake
    // permission UI; real grants/revocation must remain in the browser path.
    chrome = process.env.CONNECT_GATE6_CHROME ? resolve(process.env.CONNECT_GATE6_CHROME) : chromium.executablePath();
    report.browser_product = process.env.CONNECT_GATE6_CHROME ? "explicit-owned-chromium-executable" : "full-chromium";
    report.browser_launcher_sha256 = await digest(chrome);
    const origin = await start(1); check("one-command-four-actor-scoped-setup-ready");
    stage = "trusted-native-media-endpoint-original-recovery";
    const setup = JSON.parse(await readFile(join(state, "setup.json"), "utf8"));
    const configured = JSON.parse(await readFile(join(state, "app.json"), "utf8"));
    assert.equal(setup.profile, "parley-local-setup/3");
    const { NodeAdministrativeClient } = await import(pathToFileURL(join(consumer, "node_modules/@rudeless/connect/dist/index.js")).href);
    const admin = new NodeAdministrativeClient({ config: join(state, "installation/operator.sdk.toml"), executable: installation.executable });
    const endpointIds = new Set();
    try {
        for (const actor of setup.actors) {
            const local = configured.actors[actor.role], commands = await Promise.all(["endpoint", "verify"].map(async name =>
                JSON.parse(await readFile(join(state, "requests", actor.role + "-" + name + ".json"), "utf8"))));
            assert.equal(commands[0].operation_id, actor.endpointOperationId);
            assert.equal(commands[1].operation_id, actor.verificationOperationId);
            assert.equal(commands[0].device.id, local.mediaEndpoint.deviceId);
            assert.equal(commands[1].endpoint_id, local.mediaEndpoint.id);
            for (const command of commands) {
                const original = await admin.provision(admin.validateProvisioning(command), { timeoutMs: 8000, operationId: command.operation_id });
                const result = await original.wait({ timeoutMs: 8000 });
                assert.equal(result.snapshot.state, "completed");
                assert.equal(result.value.kind, "endpoint");
                assert.equal(result.value.endpoint_id, local.mediaEndpoint.id);
                assert.equal(result.value.identity_id, local.identityId);
                assert.equal(result.value.device_id, local.mediaEndpoint.deviceId);
                assert.equal(result.value.verification_state, "verified");
                assert.equal(result.value.revision, local.mediaEndpoint.revision);
                assert.deepEqual(result.value.capabilities, ["audio", "data", "video"]);
            }
            endpointIds.add(local.mediaEndpoint.id);
        }
    } finally { await admin.close({ timeoutMs: 5000 }); }
    assert.equal(endpointIds.size, 4);
    report.current_native_media_endpoints_verified = 4;
    report.original_endpoint_decision_recovery_verified = true;
    check("four-installed-native-devices-have-verified-media-endpoints-with-original-decision-recovery");
    context = await chromium.launchPersistentContext(join(output, "profile"), { headless: true,
        executablePath: chrome,
        viewport: { width: 1440, height: 1100 }, args: ["--disable-background-networking", "--disable-component-update", "--no-first-run", "--disable-sync", "--disable-extensions"] });
    browser = context.browser(); report.browser = browser.version(); page = context.pages()[0];
    await context.grantPermissions([], { origin });
    const errors = []; page.on("pageerror", error => errors.push(error.name));
    report.api_observations = [];
    const observeApi = (target, source) => target.on("response", response => {
        const path = new URL(response.url()).pathname;
        if (!/^\/api\/(?:state|view|prepare|lookup|submit|recover|reconnect|actor)$/.test(path)) return;
        const reading = (async () => {
            const value = await response.json();
            report.api_observations.push({ source, path, status: response.status(), error: value.error?.code ?? null,
                command: value.operation?.command ?? null, submission: value.operation?.submission ?? null,
                state: value.operation?.state ?? null, result_keys: value.result ? Object.keys(value.result).sort() : [] });
            if (report.api_observations.length > 64) report.api_observations.shift();
        })().catch(() => {}).finally(() => apiReads.delete(reading));
        apiReads.add(reading);
    });
    observeApi(page, "main");
    await page.goto(origin); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    assert.equal(await page.evaluate(() => document.cookie), "");
    assert.match(await page.locator("#media-capability").textContent(), /QUIC/); check("actual-installed-native-SDK-control-connected");
    report.native_video_playout_component = await page.evaluate(async () => {
        const { VideoPlayout } = await import("/media-playout.mjs");
        const input = new OffscreenCanvas(8, 8), source = input.getContext("2d");
        source.fillStyle = "rgb(0,255,0)"; source.fillRect(0, 0, 8, 8);
        const output = new OffscreenCanvas(8, 8), target = output.getContext("2d");
        let painted = 0, dropped = 0;
        const queue = new VideoPlayout({ now: () => performance.now() / 1000,
            paint: frame => { target.drawImage(frame, 0, 0); painted++; }, drop: () => { dropped++; } });
        try {
            const start = performance.now() / 1000;
            for (let i = 0; i < 3; i++) {
                const frame = new VideoFrame(input, { timestamp: i * 33333 });
                try { queue.enqueue(frame, start + .08 + i / 30); } finally { frame.close(); }
            }
            await new Promise(done => setTimeout(done, 300));
            return { painted, dropped, queued: queue.size, pixel: [...target.getImageData(4, 4, 1, 1).data] };
        } finally { queue.close(); }
    });
    assert.ok(report.native_video_playout_component.painted >= 1);
    assert.equal(report.native_video_playout_component.painted + report.native_video_playout_component.dropped, 3);
    assert.equal(report.native_video_playout_component.queued, 0);
    assert.deepEqual(report.native_video_playout_component.pixel, [0, 255, 0, 255]);
    check("actual-browser-native-timers-present-owned-video-clones-after-original-frames-close");
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
    await page.locator("#actor input:checked").focus();
    await page.keyboard.press("ArrowUp");
    await page.waitForFunction(() => document.querySelector("#actor input:checked")?.value === "organizer" && !document.querySelector("#actor").disabled && document.querySelector("#conversations button"));
    assert.equal(await page.locator("#actor input:checked").evaluate(input => input === document.activeElement), true);
    report.keyboard_participant_switch_verified = true;
    await page.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.getByRole("button", { name: "Accept", exact: true }).waitFor();
    await page.getByRole("button", { name: "Accept", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => [...document.querySelectorAll("#operations li")].some(value => value.textContent.includes("Accept invitation · completed")));
    assert.deepEqual(await page.locator("#message-contacts input:checked").evaluateAll(values => values.map(value => value.value)), ["owner"]);
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
    await page.waitForFunction(() => !document.querySelector("#message-form button").disabled);
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
    await page.waitForFunction(() => !document.querySelector("#reconnect").disabled && document.querySelector("#notice").textContent.startsWith("Connected"));
    await page.getByRole("button", { name: "Recover original action", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => localStorage.getItem("parley-original-action-organizer") === null);
    await page.waitForFunction(marker => document.querySelector("#timeline").textContent.includes(marker), marker);
    assert.equal(await page.locator("#timeline li").filter({ hasText: marker }).count(), 1); check("keyboard-original-status-recovery-one-visible-message");
    // These controls negotiate two real native audio Connections. Browser
    // microphone/camera remain denied; this does not claim decoded media IO.
    report.native_control_actions = [];
    const activate = async (name, command, target = page) => {
        stage = "keyboard-action-" + name;
        const until = Date.now() + 30000, remaining = () => {
            const value = until - Date.now(); assert.ok(value > 0, "native-control-observation-deadline"); return value;
        };
        const submitted = target.waitForRequest(request => new URL(request.url()).pathname === "/api/submit" && request.method() === "POST", { timeout: remaining() });
        void submitted.catch(() => {});
        await target.getByRole("button", { name, exact: true }).focus(); await target.keyboard.press("Enter");
        const operationId = (await submitted).postDataJSON().operationId;
        assert.match(operationId, /^[A-Za-z0-9._~-]{1,128}$/);
        const observeUi = async () => (await target.waitForFunction(operationId => {
            const original = JSON.parse(document.querySelector("#operations").dataset.values ?? "[]").find(v => v.operationId === operationId);
            const role = document.querySelector("#actor input:checked").value;
            const pending = JSON.parse(localStorage.getItem("parley-original-action-" + role) ?? "null");
            if (document.querySelector("#actor").disabled) return false;
            if (original?.state === "completed" || pending?.operationId === operationId || original?.submission === "refused" ||
                ["failed", "cancelled"].includes(original?.state)) return { completed: original?.state === "completed", pending: pending?.operationId === operationId };
            return false;
        }, operationId, { timeout: remaining() })).jsonValue();
        let ui = await observeUi();
        for (let read = 0; ui.pending && read < 3; read++) {
            await target.getByRole("button", { name: "Recover original action", exact: true }).focus(); await target.keyboard.press("Enter");
            ui = await observeUi();
            report.native_control_original_recoveries = (report.native_control_original_recoveries ?? 0) + 1;
        }
        assert.equal(ui.completed, true);
        assert.equal(ui.pending, false);
        // Observe the same completed original through the public app API. CDP
        // may no longer retain a no-store Fetch body after the UI consumed it.
        // This is a status observation, never another offer/control mutation.
        const observed = await target.evaluate(async ({ operationId, timeout }) => {
            const response = await fetch("/api/recover", { method: "POST", credentials: "same-origin", redirect: "error", cache: "no-store",
                referrerPolicy: "no-referrer", headers: { "Content-Type": "application/json", "X-Parley-Request": "1" },
                body: JSON.stringify({ operationId }), signal: AbortSignal.timeout(timeout) });
            return { status: response.status, value: await response.json() };
        }, { operationId, timeout: Math.min(8000, remaining()) });
        assert.equal(observed.status, 200);
        const outcome = observed.value;
        assert.equal(Boolean(outcome.error), false);
        assert.equal(outcome.operation.operationId, operationId);
        assert.equal(outcome.operation.state, "completed");
        assert.ok(outcome.result);
        assert.ok(remaining() > 0);
        report.native_control_actions.push({ name, command });
        return outcome;
    };
    const perspective = async (role, key) => {
        await page.locator("#actor input:checked").focus(); await page.keyboard.press(key);
        await page.waitForFunction(role => document.querySelector("#actor input:checked")?.value === role &&
            !document.querySelector("#actor").disabled && document.querySelector("#conversations button"), role);
        await page.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await page.keyboard.press("Enter");
    };
    await page.locator("#medium").selectOption("voice");
    await activate("Invite", "Invite");
    await perspective("owner", "ArrowDown");
    await activate("Accept", "Accept invitation");
    const ownerAudio = (await activate("Join audio", "Join media")).result;
    assert.equal(await page.locator("#connections li").count(), 1);
    await perspective("organizer", "ArrowUp");
    const organizerAudio = (await activate("Join audio", "Join media")).result;
    assert.equal(await page.locator("#connections li").count(), 1);
    assert.equal(organizerAudio.conversationId, ownerAudio.conversationId);
    assert.equal(organizerAudio.sessionId, ownerAudio.sessionId);
    assert.notEqual(organizerAudio.connectionId, ownerAudio.connectionId);
    report.actual_native_audio_connections_opened = 2;
    check("keyboard-two-participants-open-distinct-native-audio-connections-in-the-accepted-session");
    assert.deepEqual((await activate("Listen to Owner", "Listen")).result, organizerAudio);
    assert.deepEqual((await activate("Stop listening", "Stop listening")).result, organizerAudio);
    report.native_stream_subscription_roundtrip_verified = true;
    check("keyboard-current-peer-stream-subscribe-and-unsubscribe-with-negotiated-revisions");
    for (const name of ["Mute", "Unmute", "Hold", "Resume"]) assert.deepEqual((await activate(name, "Update media")).result, organizerAudio);
    report.native_connection_gate_controls_verified = true;
    check("keyboard-native-mute-unmute-hold-resume-complete-through-public-sdk");
    await page.setViewportSize({ width: 1440, height: 1100 });
    await page.screenshot({ path: join(output, "native-controls.png"), fullPage: true });
    await page.setViewportSize({ width: 320, height: 850 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.screenshot({ path: join(output, "native-controls-narrow.png"), fullPage: true });
    report.ui_screenshots = Object.fromEntries(await Promise.all(["native-controls.png", "native-controls-narrow.png"].map(async name =>
        [name, sha(await readFile(join(output, name)))])));
    check("negotiated-connection-controls-fit-320px-without-horizontal-overflow");

    // Two separate owned Chromium processes consume different local PCM files.
    // This exercises real getUserMedia/AudioWorklet/WebCodecs and installed
    // Node/native packet handles; no personal devices or providers are used.
    stage = "synthetic-browser-audio-capture-playback";
    const tones = [{ role: "owner", frequency: 440, peer: "Organizer", other: 880 },
        { role: "organizer", frequency: 880, peer: "Owner", other: 440 }];
    async function openMediaPages(video = false) {
    mediaPages.length = 0;
    for (const tone of tones) {
        const samples = 96000, wav = Buffer.alloc(44 + samples * 2);
        wav.write("RIFF"); wav.writeUInt32LE(wav.length - 8, 4); wav.write("WAVEfmt ", 8);
        wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22);
        wav.writeUInt32LE(48000, 24); wav.writeUInt32LE(96000, 28); wav.writeUInt16LE(2, 32);
        wav.writeUInt16LE(16, 34); wav.write("data", 36); wav.writeUInt32LE(samples * 2, 40);
        for (let i = 0; i < samples; i++) wav.writeInt16LE(Math.round(10000 * Math.sin(2 * Math.PI * tone.frequency * i / 48000)), 44 + i * 2);
        const audioPath = join(output, tone.role + "-synthetic.wav"); await writeFile(audioPath, wav, { mode: 0o600 });
        let cameraPath;
        if (video) {
            cameraPath = join(output, tone.role + "-synthetic.y4m");
            const pixels = 640 * 360, yuv = tone.role === "owner" ? [81, 90, 240] : [145, 54, 34];
            const frames = [Buffer.from("YUV4MPEG2 W640 H360 F30:1 Ip A1:1 C420jpeg\n")];
            for (let frame = 0; frame < 60; frame++) {
                const y = Buffer.alloc(pixels, yuv[0]);
                // A moving dark strip makes these actual synthetic camera
                // frames distinct in time while leaving the center marker.
                for (let row = 0; row < 40; row++) y.fill(16, row * 640 + frame * 8, row * 640 + frame * 8 + 40);
                frames.push(Buffer.from("FRAME\n"), y, Buffer.alloc(pixels / 4, yuv[1]), Buffer.alloc(pixels / 4, yuv[2]));
            }
            await writeFile(cameraPath, Buffer.concat(frames), { mode: 0o600 });
        }
        const owned = await chromium.launch({ headless: true,
            executablePath: chrome,
            args: ["--disable-background-networking", "--disable-component-update", "--no-first-run", "--disable-sync", "--disable-extensions",
                "--use-fake-device-for-media-stream", "--use-file-for-fake-audio-capture=" + audioPath,
                ...(cameraPath ? ["--use-file-for-fake-video-capture=" + cameraPath] : [])] });
        mediaBrowsers.push(owned);
        assert.equal(owned.version(), report.browser);
        const captureContext = await owned.newContext({ viewport: { width: 1440, height: 1100 } });
        await captureContext.grantPermissions(["microphone", ...(video ? ["camera"] : [])], { origin });
        await captureContext.addInitScript(() => {
            // Observe actual scheduled output buffers only after their rendering
            // interval ends in a running AudioContext. No PCM is retained.
            // Observe real owned browser tracks. A controlled withheld result
            // models the non-cancellable device promise after local cancellation;
            // it does not replace media tracks, codecs or packet transport.
            const devices = { captures: [], hold: false };
            Object.defineProperty(window, "__parley_device_probe__", { value: devices });
            const getUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
            navigator.mediaDevices.getUserMedia = async constraints => {
                const stream = await getUserMedia(constraints), entry = { tracks: stream.getTracks(), pending: false, endedEvents: 0 };
                for (const track of entry.tracks) track.addEventListener("ended", () => { entry.endedEvents++; }, { once: true });
                devices.captures.push(entry);
                if (devices.hold) {
                    entry.pending = true;
                    await new Promise(resolve => { entry.release = resolve; });
                    delete entry.release; entry.pending = false;
                }
                return stream;
            };
            const stats = { frames: 0, energy: 0, tone440: 0, tone880: 0 };
            Object.defineProperty(window, "__parley_audio_probe__", { value: stats });
            const original = AudioBufferSourceNode.prototype.start;
            AudioBufferSourceNode.prototype.start = function (...args) {
                const value = original.apply(this, args), buffer = this.buffer;
                if (buffer?.sampleRate === 48000 && buffer.numberOfChannels === 1 && buffer.length === 960) {
                    const end = Math.max(this.context.currentTime, args[0] ?? 0) + 0.02;
                    this.addEventListener("ended", () => {
                        if (this.context.state !== "running" || this.context.currentTime + 1 / 48000 < end) return;
                        const pcm = buffer.getChannelData(0); let energy = 0, a = 0, b = 0, c = 0, d = 0;
                        for (let i = 0; i < pcm.length; i++) {
                            energy += pcm[i] * pcm[i];
                            a += pcm[i] * Math.sin(2 * Math.PI * 440 * i / 48000);
                            b += pcm[i] * Math.cos(2 * Math.PI * 440 * i / 48000);
                            c += pcm[i] * Math.sin(2 * Math.PI * 880 * i / 48000);
                            d += pcm[i] * Math.cos(2 * Math.PI * 880 * i / 48000);
                        }
                        stats.frames++; stats.energy += energy / 960;
                        stats.tone440 += (a * a + b * b) / (960 * 960);
                        stats.tone880 += (c * c + d * d) / (960 * 960);
                    }, { once: true });
                }
                return value;
            };
        });
        const target = await captureContext.newPage(); mediaPages.push({ ...tone, page: target, context: captureContext });
        observeApi(target, tone.role + "-owned-media-browser");
        target.on("pageerror", error => errors.push(error.name));
        await target.goto(origin); await target.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
        if (tone.role === "organizer") {
            await target.locator("#actor input:checked").focus(); await target.keyboard.press("ArrowUp");
            await target.waitForFunction(() => document.querySelector("#actor input:checked").value === "organizer" &&
                !document.querySelector("#actor").disabled && document.querySelector("#conversations button"));
        }
        await target.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await target.keyboard.press("Enter");
        await activate("Listen to " + tone.peer, "Listen", target);
        if (video) await activate("Watch " + tone.peer, "Listen", target);
    }
    }
    const stoppedMedia = async target => {
        const handle = await target.waitForFunction(() => {
            const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            const devices = window.__parley_device_probe__;
            if (metrics?.stopped && metrics.activeTracks === 0 && metrics.activeContexts === 0 &&
                metrics.queuedAudioSources === 0 && metrics.queuedSendFrames === 0 &&
                metrics.queuedVideoFrames === 0 && metrics.pendingRequests === 0 &&
                devices.captures.every(c => c.tracks.every(t => t.readyState === "ended")))
                return { metrics, actual_tracks: devices.captures.flatMap(c => c.tracks).length,
                    last_capture_ended_events: devices.captures.at(-1)?.endedEvents ?? 0,
                    ended_tracks: devices.captures.flatMap(c => c.tracks).filter(t => t.readyState === "ended").length };
            return false;
        });
        return handle.jsonValue();
    };
    const startMicrophone = async target => {
        await target.getByRole("button", { name: "Start microphone", exact: true }).focus(); await target.keyboard.press("Enter");
        await target.waitForFunction(() => {
            const m = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            return m?.phase === "active" && !m.stopped && m.capturedAudioFrames >= 3;
        });
    };
    await openMediaPages();
    stage = "synthetic-browser-audio-capture-playback";
    await Promise.all(mediaPages.map(async ({ page: target }) => {
        await target.getByRole("button", { name: "Start microphone", exact: true }).focus(); await target.keyboard.press("Enter");
    }));
    report.synthetic_browser_audio = [];
    for (const tone of mediaPages) {
        const result = await (await tone.page.waitForFunction(() => {
            const raw = document.querySelector("#media-status").dataset.metrics, metrics = raw && JSON.parse(raw);
            const output = window.__parley_audio_probe__;
            if (metrics?.stopped || (metrics?.playedAudioFrames >= 12 && metrics?.clockReportsSent >= 1 &&
                metrics?.clockReportsReceived >= 1 && output.frames >= 12))
                return { metrics, output, state: document.querySelector("#media-status").dataset.state, status: document.querySelector("#media-status").textContent };
            return false;
        }, undefined, { timeout: 30000 })).jsonValue();
        report.synthetic_browser_audio.push({ role: tone.role, expected_received_hz: tone.other, ...result });
        assert.equal(result.metrics.stopped, false);
        assert.ok(result.metrics.capturedAudioFrames > 0 && result.metrics.sentRtp > 0 && result.metrics.receivedRtp > 0);
        assert.ok(result.metrics.decodedAudioFrames >= 12 && result.metrics.playedAudioFrames >= 12 && result.output.frames >= 12);
        assert.ok(result.output.energy / result.output.frames > 0.0001);
        const desired = result.output["tone" + tone.other], excluded = result.output["tone" + tone.frequency];
        assert.ok(desired > 8 * Math.max(excluded, 1e-12), "identified-remote-audio-without-local-source-crossover");
    }
    report.synthetic_capture_decoded_audio_verified = true;
    check("keyboard-synthetic-microphones-publish-and-identified-opus-audio-plays-in-both-directions");
    report.active_session_capture_survival = await Promise.all(mediaPages.map(async ({ role, page: target }) => {
        const before = await (await target.waitForFunction(() => {
            const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            return document.querySelector("#media-status").dataset.sessionState === "active" && !metrics?.stopped &&
                metrics?.phase === "active" ? metrics.capturedAudioFrames : false;
        }, undefined, { timeout: 15000 })).jsonValue();
        const after = await (await target.waitForFunction(before => {
            const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            return !metrics?.stopped && metrics?.capturedAudioFrames >= before + 3 ? metrics.capturedAudioFrames : false;
        }, before, { timeout: 5000 })).jsonValue();
        return { role, observed_session_state: "active", captured_before: before, captured_after: after };
    }));
    check("current-active-session-snapshots-preserve-original-browser-media-capture");
    await mediaPages[0].page.screenshot({ path: join(output, "capture-controls.png"), fullPage: true });
    await mediaPages[0].page.setViewportSize({ width: 320, height: 850 });
    assert.equal(await mediaPages[0].page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await mediaPages[0].page.screenshot({ path: join(output, "capture-controls-narrow.png"), fullPage: true });
    for (const name of ["capture-controls.png", "capture-controls-narrow.png"])
        report.ui_screenshots[name] = sha(await readFile(join(output, name)));
    stage = "original-audio-connection-restart-queue-freshness";
    const pausedReceiver = mediaPages[0], continuingPublisher = mediaPages[1];
    await pausedReceiver.page.getByRole("button", { name: "Stop media", exact: true }).focus();
    await pausedReceiver.page.keyboard.press("Enter"); await stoppedMedia(pausedReceiver.page);
    const publisherBefore = await continuingPublisher.page.evaluate(() =>
        JSON.parse(document.querySelector("#media-status").dataset.metrics).sentRtp);
    // More than the native inbox's32packet capacity arrives while this original
    // Connection is not being read. No new offer, SDK owner or logical ID.
    await continuingPublisher.page.waitForFunction(before => {
        const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics);
        return !metrics.stopped && metrics.sentRtp >= before + 75;
    }, publisherBefore, { timeout: 10000 });
    await continuingPublisher.page.getByRole("button", { name: "Stop media", exact: true }).focus();
    await continuingPublisher.page.keyboard.press("Enter"); await stoppedMedia(continuingPublisher.page);
    await new Promise(done => setTimeout(done, 500));
    await pausedReceiver.page.evaluate(() => {
        const output = window.__parley_audio_probe__;
        for (const key of Object.keys(output)) output[key] = 0;
    });
    await startMicrophone(pausedReceiver.page);
    await new Promise(done => setTimeout(done, 2000));
    report.original_connection_restart = await pausedReceiver.page.evaluate(() => ({
        metrics: JSON.parse(document.querySelector("#media-status").dataset.metrics),
        output: { ...window.__parley_audio_probe__ },
    }));
    assert.equal(report.original_connection_restart.metrics.stopped, false);
    // The peer has stopped publishing. Old identified tone must not be made
    // fresh by resetting the page's RTP/playout origin on local Start.
    assert.ok(report.original_connection_restart.output.energy < 0.0001, "no-old-remote-audio-after-original-connection-restart");
    await startMicrophone(continuingPublisher.page);
    report.original_connection_restart.resumed_remote_audio = await (await pausedReceiver.page.waitForFunction(() => {
        const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics), output = window.__parley_audio_probe__;
        return !metrics.stopped && output.frames >= 12 && output.tone880 > 8 * Math.max(output.tone440, 1e-12) &&
            output.energy / output.frames > .0001 ? { metrics, output: { ...output } } : false;
    }, undefined, { timeout: 10000 })).jsonValue();
    check("original-connection-local-restart-does-not-play-queued-audio-from-the-stopped-peer");
    await Promise.all(mediaPages.map(async ({ page: target }) => {
        await target.getByRole("button", { name: "Stop media", exact: true }).focus(); await target.keyboard.press("Enter");
        await target.waitForFunction(() => {
            const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            return metrics?.stopped && metrics.activeTracks === 0 && metrics.activeContexts === 0 &&
                metrics.queuedAudioSources === 0 && metrics.queuedSendFrames === 0 &&
                metrics.queuedVideoFrames === 0 && metrics.pendingRequests === 0;
        });
    }));
    report.synthetic_capture_render_owners_joined = true;
    check("keyboard-media-stop-joins-owned-browser-capture-render-and-packet-work");
    for (const { page: target } of mediaPages) await stoppedMedia(target);
    report.owned_group_size_after_media = groupMembers(processChild.pid).length;
    await Promise.all(mediaBrowsers.map(owned => owned.close()));
    assert.ok(mediaBrowsers.every(owned => !owned.isConnected())); mediaBrowsers.length = 0;
    check("additional-owned-synthetic-media-browser-owners-close");

    assert.deepEqual((await activate("End connection", "End connection")).result, organizerAudio);
    await perspective("owner", "ArrowDown");
    assert.deepEqual((await activate("End connection", "End connection")).result, ownerAudio);
    report.original_native_connection_end_receipts_verified = 2;
    check("keyboard-both-original-native-audio-connections-end-with-the-original-session-scope");
    // A separate accepted AV Session preserves every original audio check.
    await page.locator("#medium").selectOption("voice+video");
    await activate("Invite", "Invite");
    await perspective("organizer", "ArrowUp");
    await activate("Accept", "Accept invitation");
    const organizerVideo = (await activate("Join audio and video", "Join media")).result;
    await perspective("owner", "ArrowDown");
    const ownerVideo = (await activate("Join audio and video", "Join media")).result;
    assert.equal(ownerVideo.sessionId, organizerVideo.sessionId);
    assert.notEqual(ownerVideo.sessionId, ownerAudio.sessionId);
    assert.notEqual(ownerVideo.connectionId, organizerVideo.connectionId);
    report.actual_native_av_connections_opened = 2;
    check("keyboard-separate-accepted-av-session-and-two-original-native-connections");
    await openMediaPages(true);
    stage = "synthetic-browser-video-capture-playback";
    await Promise.all(mediaPages.map(async ({ page: target }) => {
        await target.getByRole("button", { name: "Start microphone and camera", exact: true }).focus(); await target.keyboard.press("Enter");
    }));
    report.synthetic_browser_video = [];
    for (const tone of mediaPages) {
        const result = await (await tone.page.waitForFunction(role => {
            const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            const colors = [...document.querySelectorAll("#remote-video canvas")].map(canvas => {
                const rgba = canvas.getContext("2d").getImageData(310, 170, 20, 20).data;
                const sums = [0, 0, 0];
                for (let i = 0; i < rgba.length; i += 4) for (let c = 0; c < 3; c++) sums[c] += rgba[i + c] / 400;
                return sums;
            });
            const desired = role === "owner" ? 1 : 0, excluded = 1 - desired;
            const marker = colors.find(color => color[desired] > 140 && color[desired] > 2 * Math.max(color[excluded], color[2], 1));
            const wrongSource = colors.some(color => color[excluded] > 140 && color[excluded] > 2 * Math.max(color[desired], color[2], 1));
            if (metrics?.stopped || (metrics?.decodedVideoFrames >= 3 && metrics?.paintedVideoFrames >= 3 &&
                metrics?.videoClockReportsSent >= 1 && metrics?.videoClockReportsReceived >= 1 && marker && !wrongSource))
                return { metrics, marker, colors, wrongSource, state: document.querySelector("#media-status").dataset.state };
            return false;
        }, tone.role, { timeout: 30000 })).jsonValue();
        assert.equal(result.metrics.stopped, false);
        assert.ok(result.metrics.capturedVideoFrames >= 3 && result.metrics.decodedVideoFrames >= 3 && result.metrics.paintedVideoFrames >= 3);
        assert.ok(result.metrics.videoClockReportsSent >= 1 && result.metrics.videoClockReportsReceived >= 1);
        assert.ok(result.marker);
        assert.equal(result.wrongSource, false);
        report.synthetic_browser_video.push({ role: tone.role, expected_remote_marker: tone.role === "owner" ? "green" : "red", ...result });
    }
    report.synthetic_capture_decoded_video_verified = true;
    check("keyboard-synthetic-cameras-publish-and-identified-vp8-video-paints-in-both-directions");
    // A short video marker can pass while the concurrent audio has stalled.
    // Observe new, completed output in every two-second window on both pages;
    // cumulative counters from the earlier audio-only Session cannot satisfy it.
    stage = "sustained-combined-browser-audio-video";
    report.sustained_browser_av = [];
    const avSample = target => target.evaluate(() => ({
        at: performance.now(),
        metrics: JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null"),
        output: { ...window.__parley_audio_probe__ },
    }));
    let previousAv = await Promise.all(mediaPages.map(({ page: target }) => avSample(target)));
    for (let window = 0; window < 5; window++) {
        await new Promise(done => setTimeout(done, 2000));
        const next = await Promise.all(mediaPages.map(({ page: target }) => avSample(target)));
        const observed = next.map((sample, index) => {
            const before = previousAv[index], tone = mediaPages[index];
            return { role: tone.role, elapsed_ms: sample.at - before.at,
                completed_audio_frames: sample.output.frames - before.output.frames,
                painted_video_frames: sample.metrics.paintedVideoFrames - before.metrics.paintedVideoFrames,
                audio_energy: sample.output.energy - before.output.energy,
                desired_tone: sample.output["tone" + tone.other] - before.output["tone" + tone.other],
                excluded_tone: sample.output["tone" + tone.frequency] - before.output["tone" + tone.frequency],
                metrics: sample.metrics };
        });
        report.sustained_browser_av.push(observed);
        for (const sample of observed) {
            assert.equal(sample.metrics.stopped, false);
            assert.ok(sample.completed_audio_frames >= 50, "combined-av-completed-audio-progress");
            assert.ok(sample.painted_video_frames >= 6, "combined-av-painted-video-progress");
            assert.ok(sample.audio_energy / sample.completed_audio_frames > 0.0001);
            assert.ok(sample.desired_tone > 8 * Math.max(sample.excluded_tone, 1e-12), "combined-av-original-remote-audio-marker");
        }
        previousAv = next;
    }
    check("both-original-av-connections-sustain-identified-audio-and-video-in-every-observation-window");
    await mediaPages[0].page.screenshot({ path: join(output, "video-controls.png"), fullPage: true });
    await mediaPages[0].page.setViewportSize({ width: 320, height: 850 });
    assert.equal(await mediaPages[0].page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await mediaPages[0].page.screenshot({ path: join(output, "video-controls-narrow.png"), fullPage: true });
    for (const name of ["video-controls.png", "video-controls-narrow.png"])
        report.ui_screenshots[name] = sha(await readFile(join(output, name)));
    await Promise.all(mediaPages.map(async ({ page: target }) => {
        await target.getByRole("button", { name: "Stop media", exact: true }).focus(); await target.keyboard.press("Enter");
        await target.waitForFunction(() => {
            const m = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
            return m?.stopped && m.activeTracks === 0 && m.activeContexts === 0 && m.queuedAudioSources === 0 &&
                m.queuedSendFrames === 0 && m.queuedVideoFrames === 0 && m.pendingRequests === 0 &&
                document.querySelectorAll("#remote-video canvas").length === 0;
        });
        await activate("Stop watching", "Stop listening", target);
    }));
    report.synthetic_av_owners_joined_and_video_unsubscribed = true;
    check("keyboard-av-stop-joins-all-capture-render-and-packet-work-and-removes-only-incoming-video");
    // Lifecycle cases keep the original physical Connection and SDK owner.
    // No new logical operation ID, retry or replacement offer hides a failure.
    report.media_lifecycle = {};
    for (const { page: target } of mediaPages) await stoppedMedia(target);
    const lifecycle = mediaPages[0], target = lifecycle.page;
    stage = "late-owned-device-result-after-keyboard-cancellation";
    await target.evaluate(() => { window.__parley_device_probe__.hold = true; });
    await target.getByRole("button", { name: "Start microphone", exact: true }).focus(); await target.keyboard.press("Enter");
    await target.waitForFunction(() => window.__parley_device_probe__.captures.at(-1)?.pending === true);
    await target.getByRole("button", { name: "Stop media", exact: true }).focus(); await target.keyboard.press("Enter");
    await target.waitForFunction(() => JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null")?.stopped);
    // Deliver the actual stream only after the app cancelled its device wait.
    await target.evaluate(() => {
        const probe = window.__parley_device_probe__; probe.hold = false; probe.captures.at(-1).release();
    });
    report.media_lifecycle.late_device_result = await stoppedMedia(target);
    assert.equal(report.media_lifecycle.late_device_result.metrics.capturedAudioFrames, 0);
    assert.equal(report.media_lifecycle.late_device_result.metrics.sentRtp, 0);
    check("keyboard-cancellation-stops-a-late-actual-browser-device-result-before-publication");

    stage = "actual-capture-permission-revocation";
    await startMicrophone(target);
    // Chromium's actual permission override revokes this owned microphone.
    // No synthetic ended event or substituted track is used.
    await lifecycle.context.clearPermissions();
    await lifecycle.context.grantPermissions(["camera"], { origin });
    await target.waitForFunction(async () => (await navigator.permissions.query({ name: "microphone" })).state === "denied");
    report.media_lifecycle.permission_revocation = await stoppedMedia(target);
    assert.equal(report.media_lifecycle.permission_revocation.metrics.stopReason, "app-media-device-ended");
    assert.ok(report.media_lifecycle.permission_revocation.last_capture_ended_events >= 1);
    check("actual-microphone-permission-revocation-ends-tracks-and-joins-media");
    await lifecycle.context.clearPermissions();
    await lifecycle.context.grantPermissions(["microphone"], { origin });

    stage = "active-capture-participant-switch-retirement";
    await startMicrophone(target);
    await target.locator("#actor input:checked").focus(); await target.keyboard.press("ArrowUp");
    await target.waitForFunction(() => document.querySelector("#actor input:checked").value === "organizer" && !document.querySelector("#actor").disabled);
    report.media_lifecycle.participant_switch = await stoppedMedia(target);
    check("keyboard-participant-switch-joins-the-old-actors-actual-device-and-media-work");
    await target.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await target.keyboard.press("Enter");
    stage = "active-capture-offline-retirement";
    await startMicrophone(target);
    await lifecycle.context.setOffline(true);
    report.media_lifecycle.offline = await stoppedMedia(target);
    check("actual-browser-offline-transition-stops-capture-render-and-packet-work");
    await lifecycle.context.setOffline(false);
    await target.waitForFunction(() => navigator.onLine && document.querySelector("#notice").textContent.startsWith("Network available") && document.querySelector("#actor").disabled && !document.querySelector("#reconnect").disabled);

    const ownerAvEnd = await activate("End connection", "End connection");
    assert.deepEqual(ownerAvEnd.result, ownerVideo);
    await perspective("organizer", "ArrowUp");
    const organizerAvEnd = await activate("End connection", "End connection");
    assert.deepEqual(organizerAvEnd.result, organizerVideo);
    // Reconnect replaces the SDK owner, so first end both original native
    // Connections explicitly. Then observe the original completed outcome.
    stage = "explicit-offline-reconnect-original-outcome";
    await target.getByRole("button", { name: "Reconnect", exact: true }).focus(); await target.keyboard.press("Enter");
    await target.waitForFunction(id => {
        const metrics = JSON.parse(document.querySelector("#media-status").dataset.metrics ?? "null");
        const original = JSON.parse(document.querySelector("#operations").dataset.values ?? "[]").find(v => v.operationId === id);
        return document.querySelector("#notice").textContent.startsWith("Connected") &&
            !document.querySelector("#actor").disabled && metrics?.stopped && original?.state === "completed";
    }, organizerAvEnd.operation.operationId);
    const recovered = await target.evaluate(async operationId => {
        const response = await fetch("/api/recover", { method: "POST", credentials: "same-origin", redirect: "error",
            cache: "no-store", headers: { "Content-Type": "application/json", "X-Parley-Request": "1" },
            body: JSON.stringify({ operationId }), signal: AbortSignal.timeout(8000) });
        return { status: response.status, value: await response.json() };
    }, organizerAvEnd.operation.operationId);
    assert.equal(recovered.status, 200); assert.equal(Boolean(recovered.value.error), false);
    assert.equal(recovered.value.operation.operationId, organizerAvEnd.operation.operationId);
    assert.equal(recovered.value.operation.state, "completed"); assert.deepEqual(recovered.value.result, organizerVideo);
    report.media_lifecycle.offline_original_recovery = { original_operation_preserved: true, current_access_observed: true,
        ...await stoppedMedia(target) };
    check("keyboard-explicit-offline-reconnect-observes-original-av-end-without-resuming-capture");
    await Promise.all(mediaBrowsers.map(owned => owned.close()));
    assert.ok(mediaBrowsers.every(owned => !owned.isConnected())); mediaBrowsers.length = 0;
    report.original_native_av_connection_end_receipts_verified = 2;
    check("both-original-av-connections-and-additional-owned-browsers-join");
    report.negotiated_capture_playback_verified = false; // Full lifecycle/failure/restart freshness remains unqualified.
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
    report.failure = { stage, name: error.name, code: typeof error.code === "string" ? error.code : null,
        source_stack: [...String(error.stack ?? "").matchAll(/check-reference-app-installed\.mjs:(\d+):(\d+)/g)]
            .slice(0, 4).map(value => ({ line: Number(value[1]), column: Number(value[2]) })) };
    report.failure_media_pages = await Promise.all(mediaPages.map(async tone => ({ role: tone.role,
        observation: await tone.page.evaluate(() => ({ session_state: document.querySelector("#media-status")?.dataset.sessionState ?? null, current_actor: document.querySelector("#actor input:checked")?.value ?? null, state: document.querySelector("#media-status")?.dataset.state, status: document.querySelector("#media-status")?.textContent,
            metrics: JSON.parse(document.querySelector("#media-status")?.dataset.metrics ?? "null"),
            output: window.__parley_audio_probe__ })).catch(() => null) })));
    if (page && !page.isClosed()) {
        report.failure_notice = await page.locator("#notice").textContent().catch(() => null);
        report.failure_permission = await page.locator("#permission").textContent().catch(() => null);
        await page.screenshot({ path: join(output, "failure.png"), fullPage: true }).catch(() => {});
    }
    process.exitCode = 1;
} finally {
    release?.();
    await Promise.all(mediaBrowsers.map(owned => owned.close().catch(() => { report.synthetic_browser_cleanup_failed = true; report.passed = false; })));
    if (context) await context.close().catch(() => { report.browser_cleanup_failed = true; report.passed = false; });
    await deadline(Promise.allSettled([...apiReads]), 2000).catch(() => { report.api_diagnostics_incomplete = true; });
    if (processChild) await stop().catch(async () => {
        report.controller_exit_code = processChild.exitCode;
        report.process_cleanup_failed = groupMembers(processChild.pid).length !== 0;
        report.passed = false;
        if (report.process_cleanup_failed) processChild.kill("SIGKILL");
        await deadline(finished, 5000).catch(() => {});
    });
    report.sources_unchanged = JSON.stringify(await tree(join(root, "examples/parley-on-connect/app"))) === JSON.stringify(sourceInputs.app) &&
        JSON.stringify(await tree(join(consumer, "app"))) === JSON.stringify(sourceInputs.app) &&
        JSON.stringify(await tree(join(consumer, "node_modules/@rudeless/connect"))) === JSON.stringify(sourceInputs.installed_sdk) &&
        sha(await readFile(fileURLToPath(import.meta.url))) === sourceInputs.checker &&
        sha(await readFile(join(root, "scripts/install-reference-app-candidate.py"))) === sourceInputs.installer;
    report.runtime_unchanged = await digest(installation.executable) === sourceInputs.runtime;
    report.browser_launcher_unchanged = chrome ? await digest(chrome) === report.browser_launcher_sha256 : null;
    report.application_runtime_matches = await digest(join(state, "runtime/connect")).then(value => value === sourceInputs.runtime).catch(() => null);
    if (!report.runtime_unchanged || report.application_runtime_matches !== true) report.passed = false;
    if (!report.sources_unchanged) report.passed = false;
    if (chrome && report.browser_launcher_unchanged !== true) report.passed = false;
    if (!report.passed) process.exitCode = 1;
    await writeFile(join(output, "summary.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
    process.stdout.write(JSON.stringify({ passed: report.passed, checks: report.checks.length, failure: report.failure }) + "\n");
}
