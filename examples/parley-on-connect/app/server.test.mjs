import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, symlink } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { connect } from "node:net";
import { request, Agent } from "node:http";
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

function persistentRequest(f, agent, path, input = {}, overrides = {}, timeoutMs = 2000) {
    const body = JSON.stringify(input);
    return new Promise((done, reject) => {
        const call = request(f.server.origin + "/api/" + path, { agent, method: "POST", headers: {
            "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body),
            "X-Parley-Request": "1", Origin: f.server.origin, Cookie: f.cookie, ...overrides,
        } }, response => {
            const port = response.socket.localPort, chunks = []; let size = 0;
            response.on("data", chunk => { size += chunk.length; if (size > 65536) call.destroy(Error("Response bound")); else chunks.push(chunk); });
            response.on("error", reject);
            response.on("end", () => {
                let value; try { value = JSON.parse(Buffer.concat(chunks).toString()); } catch (error) { reject(error); return; }
                // Observe reuse after the Agent has received ownership back.
                setImmediate(() => done({ status: response.statusCode, headers: response.headers,
                    port, reused: call.reusedSocket, value }));
            });
        });
        call.setTimeout(timeoutMs, () => call.destroy(Error("HTTP test deadline")));
        call.on("error", reject); call.end(body);
    });
}

test("reused HTTP transports recheck origin, cookie and current actor for each request", async t => {
    const f = await fixture(t), agent = new Agent({ keepAlive: true, maxSockets: 1 }); t.after(() => agent.destroy());
    const first = await persistentRequest(f, agent, "state"); assert.equal(first.value.actor, "owner");
    const selected = await persistentRequest(f, agent, "actor", { role: "organizer" });
    assert.equal(selected.status, 200); assert.equal(selected.reused, true); assert.equal(selected.port, first.port);
    const current = await persistentRequest(f, agent, "state"); assert.equal(current.value.actor, "organizer");
    assert.equal(current.port, first.port);
    const missing = await persistentRequest(f, agent, "submit", { operationId: "original" }, { Cookie: "" });
    assert.equal(missing.status, 401); assert.equal(missing.port, first.port);
    assert.deepEqual(f.calls, []);
    const after = await persistentRequest(f, agent, "state"); assert.equal(after.value.actor, "organizer");
    const refused = await persistentRequest(f, agent, "prepare", { key: "original-key", action: {} }, { Origin: "https://foreign.example" });
    assert.equal(refused.status, 403); assert.equal(refused.reused, true); assert.equal(refused.port, after.port);
    assert.deepEqual(f.calls, []);
    const accepted = await persistentRequest(f, agent, "submit", { operationId: "original" });
    assert.equal(accepted.status, 200);
    assert.deepEqual(f.calls, [{ type: "recover", role: "organizer", operationId: "original" }]);
});

test("persistent HTTP closes at its request ceiling and shutdown joins its idle original socket", async t => {
    const f = await fixture(t), agent = new Agent({ keepAlive: true, maxSockets: 1 }); t.after(() => agent.destroy());
    const first = await persistentRequest(f, agent, "state"); let last;
    for (let i = 1; i < 128; i++) {
        last = await persistentRequest(f, agent, "state");
        assert.equal(last.status, 200); assert.equal(last.reused, true); assert.equal(last.port, first.port);
    }
    assert.equal(last.headers.connection, "close");
    const replacement = await persistentRequest(f, agent, "state");
    assert.equal(replacement.status, 200); assert.equal(replacement.reused, false);
    const sockets = Object.values(agent.freeSockets).flat(); assert.equal(sockets.length, 1);
    sockets[0].ref();
    const closed = new Promise(done => sockets[0].once("close", done));
    await f.server.close(); await closed; assert.equal(sockets[0].destroyed, true);
    assert.deepEqual(f.calls, [{ type: "close" }]);
});

test("transport age retires after the original active response finishes", async t => {
    const f = await fixture(t), agent = new Agent({ keepAlive: true, maxSockets: 1 }); t.after(() => agent.destroy());
    let release, entered, settled = false;
    const barrier = new Promise(done => { release = done; }), ready = new Promise(done => { entered = done; });
    f.gateway.conversations = async () => { entered(); await barrier; return { conversations: [], cursor: null }; };
    const pending = persistentRequest(f, agent, "state", {}, {}, 20000);
    void pending.then(() => { settled = true; }, () => { settled = true; });
    try {
        await ready; await new Promise(done => setTimeout(done, 15100));
        assert.equal(settled, false, "age retirement must not destroy an original active response");
        release(); const result = await pending;
        assert.equal(result.status, 200); assert.equal(result.headers.connection, "close");
        const next = await persistentRequest(f, agent, "state"); assert.equal(next.reused, false);
    } finally { release(); await pending.catch(() => {}); }
});

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
    const lifecycle = await fetch(f.server.origin + "/media-session.mjs");
    assert.equal(lifecycle.status, 200);
    assert.match(await lifecycle.text(), /export function hasLiveMediaBinding/);
    const playout = await fetch(f.server.origin + "/media-playout.mjs");
    assert.equal(playout.status, 200);
    assert.match(await playout.text(), /export class VideoPlayout/);
    const send = await fetch(f.server.origin + "/media-send.mjs");
    assert.equal(send.status, 200); assert.match(await send.text(), /export function takeMediaSendBatch/);
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


test("HTTP media retirement names the original lease and rejects extra untrusted scope", async t => {
    const f = await fixture(t), retired = [];
    f.gateway.mediaRetire = async (role, owner, input) => { retired.push({ role, owner, input }); return { stopped: true }; };
    assert.equal((await f.api("media/close", {})).status, 400);
    assert.equal((await f.api("media/close", { leaseId: "one", actor: "organizer" })).status, 400);
    assert.deepEqual(retired, []);
    assert.deepEqual(await (await f.api("media/close", { leaseId: "one" })).json(), { stopped: true });
    assert.equal(retired[0].role, "owner"); assert.deepEqual(retired[0].input, { leaseId: "one" });
    assert.equal(typeof retired[0].owner, "object");
});

test("HTTP packet batches retain private browser ownership and reject the old unbatched shape", async t => {
    const f = await fixture(t), requests = [];
    f.gateway.mediaSend = async (role, owner, input) => {
        requests.push({ role, owner, input }); return { submissions: [{ boundary: "local-media-queue", datagramSequence: 7 }] };
    };
    const input = { leaseId: "original-lease", streamId: "original-stream", packets: [
        { sequence: 7, remainingMs: 180, packet: { kind: "rtp", base64: "synthetic" } }] };
    const result = await f.api("media/send", input);
    assert.equal(result.status, 200); assert.equal((await result.json()).submissions[0].datagramSequence, 7);
    assert.equal(requests[0].role, "owner"); assert.equal(typeof requests[0].owner, "object");
    assert.deepEqual(requests[0].input, input);
    assert.equal((await f.api("media/send", { leaseId: "original-lease", streamId: "original-stream", sequence: 7,
        packet: input.packets[0].packet })).status, 400);
    assert.equal(requests.length, 1);
});

test("abandoned HTTP open retires only the lease returned by that original request", async t => {
    const f = await fixture(t); let release, entered, retired;
    const ready = new Promise(done => { entered = done; }), barrier = new Promise(done => { release = done; });
    const closed = new Promise(done => { retired = done; }); t.after(() => release());
    f.gateway.mediaOpen = async () => { entered(); await barrier; return { leaseId: "original-http-lease" }; };
    f.gateway.mediaRetire = async (role, owner, input) => { retired({ role, input }); return { stopped: true }; };
    const abort = new AbortController();
    const request = fetch(f.server.origin + "/api/media/open", { method: "POST", signal: abort.signal,
        headers: { "Content-Type": "application/json", "X-Parley-Request": "1", Origin: f.server.origin, Cookie: f.cookie },
        body: JSON.stringify({ conversationId: "c1", sessionId: "s1", connectionId: "conn1" }) });
    const rejected = assert.rejects(request, { name: "AbortError" }); await ready; abort.abort(); await rejected;
    await new Promise(done => setTimeout(done, 25)); release();
    const result = await Promise.race([closed, new Promise((_, reject) => { const timer = setTimeout(() => reject(Error("original-close-not-observed")), 2000); timer.unref(); })]);
    assert.deepEqual(result, { role: "owner", input: { leaseId: "original-http-lease" } });
});
