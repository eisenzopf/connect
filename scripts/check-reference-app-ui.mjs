// Real owned-browser UI component only. The application backend below is
// explicitly simulated; this never qualifies the installed Node/Connect path.
import assert from "node:assert/strict";
import { mkdir, writeFile, readFile, readdir, lstat } from "node:fs/promises";
import { createHash } from "node:crypto";
import { resolve, join, relative } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { startAppServer } from "../examples/parley-on-connect/app/server.mjs";
const root = fileURLToPath(new URL("../", import.meta.url));
const args = process.argv.slice(2), options = {};
for (let index = 0; index < args.length; index += 2) {
    assert.ok(["--sdk", "--output", "--chrome"].includes(args[index]) && args[index + 1]); options[args[index]] = args[index + 1];
}
assert.ok(options["--sdk"] && options["--output"] && options["--chrome"]);
const output = resolve(options["--output"]); assert.ok(output.startsWith(join(root, ".local") + "/"));
await mkdir(output, { recursive: false, mode: 0o700 });
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const sdkDirectory = resolve(options["--sdk"]);
async function sources() {
    const result = {};
    async function walk(directory, prefix) {
        for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
            const path = join(directory, entry.name), key = prefix + "/" + entry.name;
            assert.ok(!entry.isSymbolicLink());
            if (entry.isDirectory()) await walk(path, key);
            else if (entry.isFile()) result[key] = { sha256: hash(await readFile(path)), mode: (await lstat(path)).mode & 0o777 };
        }
    }
    await walk(join(root, "examples/parley-on-connect/app"), "app");
    await walk(sdkDirectory, "installed-sdk");
    result[relative(root, fileURLToPath(import.meta.url))] = { sha256: hash(await readFile(fileURLToPath(import.meta.url))) };
    return result;
}
const sourceInputs = await sources();
const report = { format: "connect/reference-app-browser-component/1", passed: false, checks: [],
    scope: "Owned actual browser/installed browser SDK with a simulated application backend. No Node helper, Connect server, media capture/transmission, model or provider qualification.",
    task_complete: false, whole_gate_qualified: false };
report.source_revision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
report.source_inputs = sourceInputs;
const roles = [{ role: "owner", label: "Owner" }, { role: "organizer", label: "Organizer" }];
const model = { records: new Map(), conversations: [], messages: [], sessions: [], effects: 0, reconnects: 0, holds: [], holdNext: false, closed: 0 };
const error = code => Object.assign(Error(code), { code });
const progress = value => ({ operationId: value.id, actor: value.actor, command: value.action.type,
    submission: value.state ? "observed" : "prepared", state: value.state, revision: value.state ? 1 : null, needsRecovery: false });
const gateway = {
    contacts: actor => roles.filter(value => value.role !== actor),
    capabilities: async () => ({ transport: "quic", features: ["original-id-operations", "incoming-invitations"] }),
    progress: actor => [...model.records.values()].filter(value => value.actor === actor).map(progress),
    conversations: async actor => {
        if (actor === "organizer" && !model.delayedPerspective) {
            model.delayedPerspective = true; await new Promise(done => setTimeout(done, 200));
        }
        return { conversations: model.conversations.filter(value => value.people.includes(actor)).map(value => ({ id: value.id, closed: false })), cursor: null };
    },
    prepareIntent: async (actor, key, action) => {
        let value = model.records.get(key);
        if (value) { assert.equal(value.actor, actor); assert.deepEqual(value.action, action); }
        else { value = { id: "original_" + model.records.size, actor, action: structuredClone(action), state: null, result: null }; model.records.set(key, value); }
        return { operation: progress(value) };
    },
    lookupKey: (actor, key) => { const value = model.records.get(key); return value?.actor === actor ? progress(value) : null; },
    recover: async (actor, operationId) => {
        const value = [...model.records.values()].find(value => value.id === operationId && value.actor === actor);
        if (!value) throw error("unknown-app-operation");
        if (!value.state) {
            const action = value.action; model.effects++;
            if (action.type === "conversation.create") {
                const id = "conversation_" + model.conversations.length;
                model.conversations.push({ id, people: [actor, ...action.recipients] }); value.result = { conversationId: id };
            } else if (action.type === "message.send") {
                const id = "message_" + model.messages.length; model.messages.push({ id, actor, ...action });
                value.result = { conversationId: action.conversationId, messageId: id };
            } else if (action.type === "session.invite") {
                const id = "session_" + model.sessions.length;
                model.sessions.push({ id, cid: action.conversationId, state: "invited", appearances: { [actor]: "present", ...Object.fromEntries(action.recipients.map(role => [role, "invited"])) } });
                value.result = { conversationId: action.conversationId, sessionId: id };
            } else {
                const session = model.sessions.find(session => session.id === action.sessionId);
                assert.ok(session); session.state = action.type === "session.accept" ? "accepted" : "ended";
                session.appearances[actor] = action.type === "session.accept" ? "present" : "left";
                value.result = { conversationId: action.conversationId, sessionId: session.id };
            }
            value.state = "completed";
            if (model.holdNext) { model.holdNext = false; await new Promise(done => model.holds.push(done)); }
        }
        return { operation: progress(value), result: value.result };
    },
    view: async (actor, cid) => {
        const conversation = model.conversations.find(value => value.id === cid && value.people.includes(actor));
        assert.ok(conversation);
        return { conversation: { id: cid, closed: false }, participants: conversation.people.map(role => ({ id: "participant_" + role, role, label: roles.find(value => value.role === role).label })),
            messages: model.messages.filter(value => value.conversationId === cid && (value.actor === actor || value.recipients.includes(actor)))
                .map(value => ({ id: value.id, from: roles.find(role => role.role === value.actor).label, text: value.text })),
            sessions: model.sessions.filter(value => value.cid === cid).map(value => ({ id: value.id, state: value.state, media: ["data"], appearance: value.appearances[actor] ?? "unavailable" })),
            connections: [], historyCursor: null, sampledAt: Date.now() };
    },
    reconnect: async () => {
        model.reconnecting = true; await new Promise(done => setTimeout(done, 200));
        model.reconnects++; model.reconnecting = false; return { transport: "quic" };
    },
    close: async () => { model.closed++; for (const done of model.holds.splice(0)) done(); },
};
let app, context, browser, page;
const check = name => { report.checks.push(name); process.stdout.write(JSON.stringify({ check: name }) + "\n"); };
async function state() {
    return page.evaluate(() => ({ actor: document.querySelector("#actor")?.value,
        actorOptions: [...document.querySelectorAll("#actor option")].map(value => ({ value: value.value, selected: value.selected })),
        notice: document.querySelector("#notice")?.textContent, permission: document.querySelector("#permission")?.textContent,
        conversationButtons: document.querySelectorAll("#conversations button").length,
        activeElement: document.activeElement?.id, pending: Boolean(document.querySelector("#pending button")) }));
}
const wait = async predicate => { const until = Date.now() + 10000; while (!predicate()) { assert.ok(Date.now() < until, "Bounded fixture observation timed out"); await new Promise(done => setTimeout(done, 25)); } };
try {
    const { chromium } = await import(pathToFileURL(join(root, "scripts/video/observer-tools/node_modules/playwright/index.mjs")).href);
    app = await startAppServer({ gateway, sdkDirectory, roles });
    context = await chromium.launchPersistentContext(join(output, "profile"), { executablePath: resolve(options["--chrome"]), headless: true,
        viewport: { width: 1440, height: 1100 }, args: ["--disable-background-networking", "--disable-component-update", "--no-first-run", "--disable-sync", "--disable-extensions"] });
    browser = context.browser(); report.browser = browser.version();
    await context.grantPermissions([], { origin: app.origin });
    page = context.pages()[0]; const errors = [];
    page.on("pageerror", error => errors.push(error.name));
    await page.goto(app.origin);
    await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Connected"));
    assert.equal(await page.evaluate(() => document.cookie), ""); check("http-only-app-session");
    await page.getByRole("button", { name: "Create conversation", exact: true }).focus(); await page.keyboard.press("Enter");
    await wait(() => model.conversations.length === 1);
    await page.waitForFunction(() => !document.querySelector("#message-form button").disabled);
    assert.equal(model.effects, 1); check("desktop-keyboard-create-and-participant-selection");
    const text = "<img src=x onerror=alert(1)> keyboard message";
    await page.locator("#message").fill(text); await page.getByRole("button", { name: "Send message", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(text => document.querySelector("#timeline").textContent.includes(text), text);
    assert.equal(await page.locator("#timeline img").count(), 0); check("keyboard-message-and-text-safe-timeline");
    await page.getByRole("button", { name: "Invite", exact: true }).focus(); await page.keyboard.press("Enter");
    await wait(() => model.sessions.length === 1); await page.waitForFunction(() => !document.querySelector("#create-form button").disabled);
    // Set the fixture's receiving perspective through the actual native select.
    // This is fixture preparation, not a keyboard-select qualification: macOS
    // headless Chrome also ignores these keys on an isolated stock <select>.
    await page.locator("#actor").selectOption("organizer");
    report.perspective_selection = "Native selectOption fixture preparation; keyboard perspective switching is unqualified.";
    report.after_actor_keyboard = await state();
    await page.waitForFunction(() => document.querySelector("#actor").value === "organizer" && document.querySelector("#conversations button"));
    await page.getByRole("button", { name: "Conversation 1", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.getByRole("button", { name: "Accept", exact: true }).waitFor();
    await page.getByRole("button", { name: "Accept", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector("#sessions").textContent.includes("accepted")); check("keyboard-incoming-invitation-and-decision");
    assert.deepEqual(await page.locator("#message-contacts input:checked").evaluateAll(values => values.map(value => value.value)), ["owner"]);
    check("delayed-perspective-change-retains-current-peer-audience");
    for (const [kind, permission] of [["microphone", "microphone"], ["camera", "camera"]]) {
        assert.equal(await page.evaluate(async name => (await navigator.permissions.query({ name })).state, permission), "denied");
        await page.locator("#" + kind).focus(); await page.keyboard.press("Enter");
        await page.waitForFunction(kind => document.querySelector("#permission").textContent.includes(kind + " permission denied"), kind);
        check("actual-browser-" + kind + "-permission-denial");
    }
    const desktop = join(output, "desktop.png"); await page.screenshot({ path: desktop, fullPage: true }); report.desktop_sha256 = hash(await readFile(desktop));
    await page.setViewportSize({ width: 320, height: 850 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true); check("320px-no-horizontal-overflow");
    const marker = "original message after lost HTTP response"; model.holdNext = true;
    await page.locator("#message").fill(marker); await page.getByRole("button", { name: "Send message", exact: true }).focus(); await page.keyboard.press("Enter");
    await wait(() => model.holds.length === 1); const originalEffects = model.effects;
    const original = [...model.records.values()].find(value => value.action.text === marker);
    assert.ok(original);
    await context.setOffline(true); await page.waitForFunction(() => document.querySelector("#notice").textContent.startsWith("Offline"));
    for (const done of model.holds.splice(0)) done();
    const stored = await page.evaluate(() => Object.fromEntries(Object.entries(localStorage)));
    assert.ok(!JSON.stringify(stored).includes(marker)); assert.ok(!JSON.stringify(stored).includes(text));
    assert.equal(JSON.parse(stored["parley-original-action-organizer"]).operationId, original.id);
    check("offline-keeps-only-original-ids-in-browser-storage");
    await context.setOffline(false); await page.reload();
    await page.getByRole("button", { name: "Recover original action", exact: true }).waitFor();
    await page.getByRole("button", { name: "Reconnect", exact: true }).focus(); await page.keyboard.press("Enter");
    await wait(() => model.reconnecting);
    assert.equal(await page.getByRole("button", { name: "Recover original action", exact: true }).isDisabled(), true);
    check("recovery-waits-for-delayed-reconnect");
    await wait(() => model.reconnects === 1);
    await page.waitForFunction(() => !document.querySelector("#reconnect").disabled);
    await page.getByRole("button", { name: "Recover original action", exact: true }).focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => localStorage.getItem("parley-original-action-organizer") === null);
    assert.equal(model.effects, originalEffects); assert.equal(model.messages.filter(value => value.text === marker).length, 1);
    check("keyboard-reload-reconnect-original-recovery-without-duplicate");
    const narrow = join(output, "narrow.png"); await page.screenshot({ path: narrow, fullPage: true }); report.narrow_sha256 = hash(await readFile(narrow));
    assert.deepEqual(errors, []); check("no-page-exceptions");
    await context.close(); context = null; assert.equal(browser.isConnected(), false); check("original-owned-browser-joined");
    await app.close(); app = null; assert.equal(model.closed, 1); check("original-app-server-and-simulated-backend-joined");
    report.passed = true;
} catch (error) {
    report.failure = { name: error.name, code: error.code ?? null, message: String(error.message).slice(0, 600) };
    if (page && !page.isClosed()) {
        report.failure_state = await state().catch(() => null);
        const path = join(output, "failure.png");
        await page.screenshot({ path, fullPage: true }).then(async () => { report.failure_screenshot_sha256 = hash(await readFile(path)); }).catch(() => {});
    }
    throw error;
} finally {
    if (context) await context.close().catch(() => {});
    if (app) await app.close().catch(() => {});
    report.sources_unchanged = JSON.stringify(await sources()) === JSON.stringify(sourceInputs);
    if (!report.sources_unchanged) { report.passed = false; process.exitCode = 1; }
    await writeFile(join(output, "summary.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
    process.stdout.write(JSON.stringify({ passed: report.passed, checks: report.checks.length, scope: report.scope }) + "\n");
}
