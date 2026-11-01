import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, symlink } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { connect } from "node:net";
import { request } from "node:http";
import { startAppServer } from "./server.mjs";
const root = fileURLToPath(new URL("../../../.local/gate-6/reference-app/", import.meta.url));
await mkdir(root, { recursive: true, mode: 0o700 });
async function fixture(t) {
    const directory = await mkdtemp(join(root, "http-")), sdk = join(directory, "sdk"); await mkdir(sdk);
    await writeFile(join(sdk, "browser.js"), "export const browserCaptureSupport=()=>({microphone:true,camera:true});");
    const calls = [], roles = [{ role: "owner", label: "Owner" }, { role: "organizer", label: "Organizer" }];
    const gateway = { contacts: () => roles, progress: () => [], conversations: async role => ({ conversations: [], cursor: null }),
        capabilities: async role => ({ transport: "quic", features: ["original-id-operations"] }),
        prepareIntent: async (role, key, action) => { calls.push({ type: "prepare", role, key, action }); return { operation: { operationId: "original-operation", submission: "prepared" } }; },
        recover: async (role, operationId) => { calls.push({ type: "recover", role, operationId }); return { operation: { operationId, state: "completed" }, result: { conversationId: "conversation-1" } }; },
        reconnect: async role => ({ transport: "quic" }), lookupKey: () => null,
        view: async role => ({ conversation: { id: "conversation-1", closed: false }, participants: [], messages: [], sessions: [], connections: [], historyCursor: null }),
        close: async () => calls.push({ type: "close" }) };
    const server = await startAppServer({ gateway, sdkDirectory: sdk, roles }); t.after(() => server.close());
    const page = await fetch(server.origin); const cookie = page.headers.get("set-cookie").split(";")[0];
    const html = await page.text();
    const api = async (path, value = {}, overrides = {}) => fetch(server.origin + "/api/" + path, { method: "POST",
        headers: { "Content-Type": "application/json", "X-Parley-Request": "1", Origin: server.origin, Cookie: cookie, ...overrides },
        body: JSON.stringify(value) });
    return { server, api, cookie, sdk, calls, gateway, page, html };
}

test("local page sets private same-site session and reload preserves the selected fixture actor", async t => {
    const f = await fixture(t);
    assert.match(f.html, /Local synthetic participants/);
    assert.match(f.page.headers.get("set-cookie"), /HttpOnly; SameSite=Strict/);
    assert.match(f.page.headers.get("content-security-policy"), /frame-ancestors 'none'/);
    assert.equal((await f.api("actor", { role: "organizer" })).status, 200);
    const reloaded = await fetch(f.server.origin, { headers: { Cookie: f.cookie } }); await reloaded.text();
    assert.equal(reloaded.headers.get("set-cookie").split(";")[0], f.cookie);
    const state = await (await f.api("state")).json(); assert.equal(state.actor, "organizer");
    assert.equal(state.mode, "fixture"); assert.ok(!JSON.stringify(state).includes("private-owner.toml"));
});

test("foreign origin missing session and wrong Host refuse before gateway work", async t => {
    const f = await fixture(t);
    assert.equal((await f.api("prepare", { key: "key", action: {} }, { Origin: "https://foreign.example" })).status, 403);
    assert.equal((await f.api("prepare", { key: "key", action: {} }, { Cookie: "" })).status, 401);
    assert.equal((await f.api("prepare", { key: "key", action: {} }, { "X-Parley-Request": "" })).status, 403);
    const response = await new Promise((done, reject) => {
        const call = request(f.server.origin, { headers: { Host: "foreign.example" } }, value => {
            value.resume(); value.on("end", () => done(value.statusCode));
        }); call.on("error", reject); call.end();
    });
    assert.equal(response, 403); assert.deepEqual(f.calls, []);
});

test("static SDK serving refuses traversal private configuration and symlink targets", async t => {
    const f = await fixture(t);
    const privateFile = join(f.sdk, "private.toml"); await writeFile(privateFile, "private-credential-marker");
    await symlink(privateFile, join(f.sdk, "linked.js"));
    for (const path of ["/private.toml", "/sdk/private.toml", "/sdk/linked.js", "/sdk/%2e%2e%2fprivate.toml"])
        assert.equal((await fetch(f.server.origin + path)).status, 404, path);
    assert.equal((await fetch(f.server.origin + "/sdk/browser.js")).status, 200);
});

test("HTTP actions keep preparation and submission separate and bind the captured actor", async t => {
    const f = await fixture(t);
    const action = { type: "conversation.create", recipients: ["organizer"] };
    const prepared = await (await f.api("prepare", { key: "original-key", action })).json();
    assert.equal(prepared.operation.submission, "prepared");
    assert.deepEqual(f.calls, [{ type: "prepare", role: "owner", key: "original-key", action }]);
    await (await f.api("submit", { operationId: prepared.operation.operationId })).json();
    assert.deepEqual(f.calls[1], { type: "recover", role: "owner", operationId: "original-operation" });
    assert.equal((await f.api("submit", { operationId: "original-operation", credential: "private" })).status, 400);
    assert.equal(f.calls.length, 2);
});

test("owned shutdown closes an actual abandoned header socket and joins the gateway", async t => {
    const f = await fixture(t), url = new URL(f.server.origin);
    const socket = connect(Number(url.port), "127.0.0.1"); await new Promise((done, reject) => { socket.once("connect", done); socket.once("error", reject); });
    socket.write("P");
    const ended = new Promise(done => { socket.once("close", done); socket.on("error", () => {}); });
    const started = Date.now(); await f.server.close(); await ended;
    assert.ok(Date.now() - started < 2000); assert.deepEqual(f.calls, [{ type: "close" }]);
    await f.server.close(); assert.equal(f.calls.length, 1);
});


test("media HTTP routes keep owner identity private and actor switch fences late lease publication", async t => {
    const f = await fixture(t); let release, entered;
    const ready = new Promise(r => { entered = r; }), barrier = new Promise(r => { release = r; });
    const owners = [];
    f.gateway.mediaOpen = async (role, owner, binding) => {
        owners.push(owner); f.calls.push({ type: "media-open", role, binding }); entered(); await barrier;
        return { leaseId: "old-lease", negotiation: { streams: [] } };
    };
    f.gateway.mediaCloseOwner = async owner => { f.calls.push({ type: "media-close", owner }); };
    const opening = f.api("media/open", { conversationId: "c1", sessionId: "s1", connectionId: "conn1" });
    await ready;
    assert.equal((await f.api("actor", { role: "organizer" })).status, 200);
    release(); const late = await opening; assert.equal(late.status, 400);
    assert.deepEqual(await late.json(), { error: { code: "app-media-stopped" } });
    assert.ok(f.calls.filter(c => c.type === "media-close").every(c => c.owner === owners[0]));
    f.gateway.mediaOpen = async (role, owner) => { owners.push(owner); return { leaseId: "new-lease", actor: role }; };
    const fresh = await (await f.api("media/open", { conversationId: "c1", sessionId: "s1", connectionId: "conn1" })).json();
    assert.deepEqual(fresh, { leaseId: "new-lease", actor: "organizer" }); assert.notEqual(owners[0], owners[1]);
    const count = owners.length;
    assert.equal((await f.api("media/open", { conversationId: "c1", sessionId: "s1", connectionId: "conn1" }, { Origin: "https://foreign.example" })).status, 403);
    assert.equal(owners.length, count);
});
