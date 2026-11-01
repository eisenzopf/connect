import { createServer } from "node:http";
import { randomBytes } from "node:crypto";
import { readFile, lstat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { publicError } from "./gateway.mjs";
const assets = fileURLToPath(new URL("./ui/", import.meta.url));
const fail = code => { const error = Error(code); error.code = code; throw error; };
function object(value, keys, optional = []) {
    if (!value || Array.isArray(value) || Object.getPrototypeOf(value) !== Object.prototype ||
        keys.some(key => !Object.hasOwn(value, key)) || Object.keys(value).some(key => ![...keys, ...optional].includes(key))) fail("invalid-app-request");
}
async function body(request) {
    if (!/^application\/json(?:;\s*charset=utf-8)?$/i.test(request.headers["content-type"] ?? "") ||
        Number(request.headers["content-length"] ?? 0) > 65536) fail("invalid-app-request");
    const chunks = []; let size = 0;
    for await (const chunk of request) {
        size += chunk.length; if (size > 65536) fail("invalid-app-request"); chunks.push(chunk);
    }
    try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks))); }
    catch { fail("invalid-app-request"); }
}

/** Local fixture UI only. Operator/provider credentials and SDK configurations
 * are never a route, response field, URL parameter or browser storage value. */
export async function startAppServer({ gateway, sdkDirectory, roles, port = 0, onError = () => {} }) {
    if (!gateway || !Array.isArray(roles) || roles.length > 5 || !roles.some(v => v.role === "owner") ||
        roles.some(v => typeof v.label !== "string" || !["owner", "assistant", "booker", "organizer", "observer"].includes(v.role)) ||
        !Number.isSafeInteger(port) || port < 0 || port > 65535) fail("invalid-app-request");
    const sdk = resolve(sdkDirectory), sessions = new Map(), active = new Set(), sockets = new Set();
    if (!(await lstat(sdk)).isDirectory() || (await lstat(sdk)).isSymbolicLink()) fail("invalid-app-request");
    let origin, cookieName, closing = false, closed;
    const headers = response => {
        response.setHeader("Cache-Control", "no-store"); response.setHeader("Pragma", "no-cache");
        response.setHeader("Referrer-Policy", "no-referrer"); response.setHeader("X-Content-Type-Options", "nosniff");
        response.setHeader("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; media-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
        response.setHeader("Permissions-Policy", "camera=(self), microphone=(self), display-capture=(self)");
        response.setHeader("Connection", "close");
    };
    const json = (response, status, value) => {
        if (response.destroyed || response.writableEnded) return;
        response.writeHead(status, { "Content-Type": "application/json; charset=utf-8" }); response.end(JSON.stringify(value));
    };
    const server = createServer({ maxHeaderSize: 8192 }, (request, response) => {
        headers(response);
        if (closing || active.size >= 16) { json(response, 503, { error: { code: "app-request-capacity" } }); request.resume(); return; }
        const work = handle(request, response).catch(error => { onError(error); json(response, error?.code === "app-reconnecting" ? 503 : 400, { error: publicError(error) }); })
            .finally(() => { active.delete(work); if (!request.complete) request.destroy(); });
        active.add(work);
    });
    server.maxConnections = 64; server.maxRequestsPerSocket = 1;
    server.headersTimeout = 5000; server.requestTimeout = 10000; server.keepAliveTimeout = 1000;
    server.on("connection", socket => {
        sockets.add(socket); const timer = setTimeout(() => socket.destroy(), 15000); timer.unref();
        socket.on("close", () => { clearTimeout(timer); sockets.delete(socket); });
    });
    server.on("upgrade", (_request, socket) => socket.destroy());
    await new Promise((done, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", () => { server.off("error", reject); done(); }); });
    origin = "http://127.0.0.1:" + server.address().port; cookieName = "parley_" + server.address().port;
    function count(request, name) { return request.rawHeaders.filter((value, index) => index % 2 === 0 && value.toLowerCase() === name).length; }
    async function handle(request, response) {
        if (request.httpVersion !== "1.1" || count(request, "host") !== 1 || request.headers.host !== new URL(origin).host ||
            count(request, "origin") > 1 || (request.headers.origin !== undefined && request.headers.origin !== origin) ||
            ["cross-site", "same-site"].includes(request.headers["sec-fetch-site"])) {
            json(response, 403, { error: { code: "app-origin-refused" } }); return;
        }
        const url = new URL(request.url, origin);
        if (url.origin !== origin || url.search || url.hash) { json(response, 400, { error: { code: "invalid-app-request" } }); return; }
        if (request.method === "GET" && url.pathname === "/") {
            for (const [key, value] of sessions) if (value.expires <= Date.now()) {
                await gateway.mediaCloseOwner?.(value.mediaOwner); sessions.delete(key);
            }
            const existing = (request.headers.cookie ?? "").split(";").map(v => v.trim()).filter(v => v.startsWith(cookieName + "="));
            let token = existing.length === 1 ? existing[0].slice(cookieName.length + 1) : null;
            if (!sessions.has(token)) {
                if (sessions.size >= 16) { json(response, 503, { error: { code: "app-request-capacity" } }); return; }
                token = randomBytes(32).toString("hex"); sessions.set(token, { role: "owner", expires: Date.now() + 3600000, mediaOwner: {} });
            }
            response.setHeader("Set-Cookie", cookieName + "=" + token + "; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600");
            await serve(response, join(assets, "index.html"), "text/html"); return;
        }
        if (request.method === "GET" && ["/app.mjs", "/app.css"].includes(url.pathname)) {
            await serve(response, join(assets, url.pathname.slice(1)), url.pathname.endsWith(".css") ? "text/css" : "text/javascript"); return;
        }
        if (request.method === "GET" && url.pathname.startsWith("/sdk/")) {
            const leaf = url.pathname.slice(5);
            if (!/^[a-zA-Z0-9._-]{1,128}\.js$/.test(leaf)) { json(response, 404, { error: { code: "not-found" } }); return; }
            await serve(response, join(sdk, leaf), "text/javascript"); return;
        }
        if (request.method !== "POST" || !url.pathname.startsWith("/api/")) {
            json(response, 404, { error: { code: "not-found" } }); return;
        }
        if (request.headers.origin !== origin || request.headers["x-parley-request"] !== "1" || count(request, "cookie") > 1) {
            json(response, 403, { error: { code: "app-origin-refused" } }); return;
        }
        const cookies = (request.headers.cookie ?? "").split(";").map(v => v.trim()).filter(v => v.startsWith(cookieName + "="));
        const session = cookies.length === 1 ? sessions.get(cookies[0].slice(cookieName.length + 1)) : undefined;
        if (!session || session.expires <= Date.now()) { json(response, 401, { error: { code: "app-session-expired" } }); return; }
        const actor = session.role, mediaOwner = session.mediaOwner, input = await body(request); let value;
        const mediaAction = async action => {
            if (session.role !== actor || session.mediaOwner !== mediaOwner) fail("app-media-stopped");
            const result = await action();
            if (session.role !== actor || session.mediaOwner !== mediaOwner) {
                await gateway.mediaCloseOwner(mediaOwner); fail("app-media-stopped");
            }
            return result;
        };
        switch (url.pathname) {
            case "/api/state": {
                object(input, [], ["cursor"]);
                try {
                    const [capabilities, conversations] = await Promise.all([gateway.capabilities(actor), gateway.conversations(actor, input.cursor)]);
                    value = { actor, mode: "fixture", roles, contacts: gateway.contacts(actor), capabilities,
                        operations: gateway.progress(actor), ...conversations };
                } catch (error) {
                    json(response, 503, { actor, mode: "fixture", roles, contacts: gateway.contacts(actor),
                        operations: gateway.progress(actor), error: publicError(error) }); return;
                }
                break;
            }
            case "/api/actor":
                object(input, ["role"]); if (!roles.some(v => v.role === input.role)) fail("invalid-app-request");
                session.mediaOwner = {}; session.role = input.role;
                await gateway.mediaCloseOwner?.(mediaOwner);
                value = { actor: input.role, mode: "fixture" }; break;
            case "/api/media/open":
                object(input, ["conversationId", "sessionId", "connectionId"]);
                value = await mediaAction(() => gateway.mediaOpen(actor, mediaOwner, input)); break;
            case "/api/media/send":
                object(input, ["leaseId", "streamId", "sequence", "packet"]);
                value = await mediaAction(() => gateway.mediaSend(actor, mediaOwner, input)); break;
            case "/api/media/receive":
                object(input, ["leaseId", "streamId"]);
                value = await mediaAction(() => gateway.mediaReceive(actor, mediaOwner, input)); break;
            case "/api/media/close":
                object(input, []);
                await gateway.mediaCloseOwner?.(mediaOwner); value = { stopped: true }; break;
            case "/api/view": object(input, ["conversationId"], ["cursor"]); value = await gateway.view(actor, input.conversationId, input.cursor); break;
            case "/api/prepare": object(input, ["key", "action"]); value = await gateway.prepareIntent(actor, input.key, input.action); break;
            case "/api/lookup": object(input, ["key"]); value = { operation: gateway.lookupKey(actor, input.key) }; break;
            case "/api/submit": object(input, ["operationId"]); value = await gateway.recover(actor, input.operationId); break;
            case "/api/recover": object(input, ["operationId"]); value = await gateway.recover(actor, input.operationId); break;
            case "/api/reconnect": object(input, []); value = await gateway.reconnect(actor); break;
            default: json(response, 404, { error: { code: "not-found" } }); return;
        }
        json(response, 200, value);
    }
    async function serve(response, path, type) {
        try {
            const file = await lstat(path);
            if (!file.isFile() || file.isSymbolicLink() || file.size > 2 * 1024 * 1024) throw Error();
            const bytes = await readFile(path); response.writeHead(200, { "Content-Type": type + "; charset=utf-8" }); response.end(bytes);
        } catch { json(response, 404, { error: { code: "not-found" } }); }
    }
    return { origin, async close() {
        if (!closed) closed = (async () => {
            closing = true;
            const stopped = new Promise((done, reject) => server.close(error => error ? reject(error) : done()));
            for (const socket of sockets) socket.destroy();
            const result = await Promise.allSettled([gateway.close(), stopped]);
            await Promise.allSettled([...active]); sessions.clear();
            if (result.some(v => v.status === "rejected")) fail("app-cleanup-incomplete");
        })();
        return closed;
    } };
}
