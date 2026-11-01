import { BrowserAppMedia } from "./media.mjs";
import { browserCaptureSupport } from "/sdk/browser.js";
const $ = id => document.getElementById(id);
let actor = "owner", epoch = 0, working = false, switching = false, pending = null, storageBlocked = false, selectedId = null;
let historyCursor = null, nextHistory = null, nextConversations = null, current = null, contacts = [], timer;
const requests = new Set(), previews = new Set(); let captureEpoch = 0, media = null;
const label = { "conversation.create": "Create conversation", "message.send": "Send message", "session.invite": "Invite",
    "session.accept": "Accept invitation", "session.reject": "Decline invitation", "session.cancel": "Cancel invitation",
    "session.end": "End session", "connection.update": "Update media", "connection.offer": "Join media",
    "connection.end": "End connection", "stream.subscribe": "Listen", "stream.unsubscribe": "Stop listening" };
const explanations = {
    "offline-or-uncertain": "Connection unavailable or outcome uncertain. Recover the original action.",
    "permission-denied": "Access refused. Reconnect with current access before trying again.",
    "app-resync-required": "The saved view is no longer current. Refresh the conversation.",
    "resync-required": "The saved view is no longer current. Refresh the conversation.",
    "app-session-expired": "The local app restarted or your session expired. Reload this page.",
    "app-reconnecting": "Reconnecting. Wait for current access before recovering the original action.",
    "app-local-state": "Recovery state could not be saved. The action has not been sent.",
    "app-corrupt-local-state": "Saved recovery state is unreadable. Do not repeat the action.",
    "different-original-intent": "This saved action has different contents. Recover its original outcome.",
    "invalid-app-request": "Check the selected people and action, then try again.",
    "validation": "Check the selected people and action, then refresh the current conversation.",
    "conflict": "The conversation changed. Refresh to read its current state before choosing another action.",
    "configuration": "The configured client is unavailable. Check local setup and reconnect.",
    "unsupported": "This action is unavailable with the current capabilities.",
    "unsupported-app-action": "This action is unavailable with the current capabilities.",
    "app-media-denied": "Microphone or camera permission denied. You can continue with text.",
    "app-media-busy": "This connection is already sharing media in another page.",
    "app-media-stopped": "Media stopped. Refresh the connection before starting again.",
    "app-media-permission-timeout": "Permission is still pending. No device is active.",
    "app-media-playback": "Playback paused or unavailable. Start media again from this page.",
    "app-media-network": "Media connection unavailable. Reconnect before starting media again.",
    "app-media-device-ended": "The selected device stopped. Choose Start media to try again.",
    "app-media-capture": "Media capture unavailable. Check browser device settings.",
    "app-media-codec": "The negotiated media format is unavailable in this browser.",
    "action-refused": "The action was refused. Check the current conversation and permissions.",
};
function notice(text, error = false) { $("notice").textContent = text; $("notice").dataset.error = String(error); }
function problem(error) { if (error.code !== "view-changed") notice(explanations[error.code] ?? "Action unavailable. Refresh or recover the original action.", true); }
function node(tag, text) { const value = document.createElement(tag); if (text !== undefined) value.textContent = text; return value; }
function button(text, action, mutation = false) {
    const value = node("button", text); value.type = "button"; value.className = "secondary";
    if (mutation) value.dataset.mutation = "";
    value.addEventListener("click", () => Promise.resolve().then(action).catch(problem)); return value;
}
function recoveryButton(text, action) {
    const value = button(text, action); value.dataset.recovery = "";
    value.disabled = working || switching || navigator.onLine === false; return value;
}
const savedKey = () => "parley-original-action-" + actor;
function readPending() {
    storageBlocked = false;
    try {
        const raw = localStorage.getItem(savedKey()); pending = raw === null ? null : JSON.parse(raw);
        if (pending && (Object.keys(pending).sort().join() !== "key,operationId" ||
            typeof pending.key !== "string" || !/^[A-Za-z0-9._~-]{1,128}$/.test(pending.key) ||
            (pending.operationId !== null && (typeof pending.operationId !== "string" || !/^[A-Za-z0-9._~-]{1,128}$/.test(pending.operationId))))) throw Error();
    } catch { storageBlocked = true; problem({ code: "app-corrupt-local-state" }); }
}
function savePending(value) {
    try { if (value) localStorage.setItem(savedKey(), JSON.stringify(value)); else localStorage.removeItem(savedKey()); pending = value; }
    catch { storageBlocked = true; throw Object.assign(Error(), { code: "app-local-state" }); }
}
function controls() {
    for (const value of document.querySelectorAll("[data-mutation]")) value.disabled = working || switching || storageBlocked || Boolean(pending) ||
        navigator.onLine === false || (value.closest("#message-form,#invite-form") !== null && (!selectedId || !current || current.conversation.closed));
    $("stop-media").disabled = !media;
    $("reconnect").disabled = working || switching; $("actor").disabled = working || switching;
    for (const value of document.querySelectorAll("[data-recovery]")) value.disabled = working || switching || navigator.onLine === false;
}
function clearView() {
    void stopMedia(); stopPreview(); current = null;
    for (const id of ["timeline", "sessions", "connections", "participants"]) $(id).replaceChildren();
    $("conversation-title").textContent = selectedId ? "Conversation unavailable" : "Choose a conversation";
    $("more-messages").hidden = true; controls();
}
function retire() { epoch++; for (const request of requests) request.abort(); requests.clear(); }
async function api(path, input = {}) {
    const captured = epoch, controller = new AbortController(), timeout = setTimeout(() => controller.abort(), 8000);
    requests.add(controller);
    try {
        if (navigator.onLine === false) throw Error();
        const response = await fetch("/api/" + path, { method: "POST", credentials: "same-origin", redirect: "error",
            cache: "no-store", referrerPolicy: "no-referrer", headers: { "Content-Type": "application/json", "X-Parley-Request": "1" },
            body: JSON.stringify(input), signal: controller.signal });
        if (!response.body) throw Error();
        const reader = response.body.getReader(), chunks = []; let size = 0;
        try {
            while (true) {
                const next = await reader.read(); if (next.done) break;
                size += next.value.byteLength; if (size > 2 * 1024 * 1024) { await reader.cancel(); throw Error(); }
                chunks.push(next.value);
            }
        } finally { reader.releaseLock(); }
        const bytes = new Uint8Array(size); let offset = 0;
        for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
        const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
        if (captured !== epoch) throw Object.assign(Error(), { code: "view-changed" });
        if (!response.ok && path !== "state") throw Object.assign(Error(), { code: value.error?.code });
        return value;
    } catch (error) {
        // Fetch aborts can be DOMExceptions with a read-only numeric `code`.
        // Preserve our view fence without mutating browser-owned exceptions.
        throw Object.assign(Error(), { code: captured !== epoch ? "view-changed" :
            typeof error?.code === "string" ? error.code : "offline-or-uncertain" });
    } finally { clearTimeout(timeout); requests.delete(controller); }
}
function choices(id, people, context) {
    const value = $(id), signature = JSON.stringify([context, people]); if (value.dataset.people === signature) return;
    const initialized = value.dataset.context === context && value.dataset.initialized === "true";
    const selected = new Set([...value.querySelectorAll("input:checked")].map(input => input.value));
    value.replaceChildren(); value.dataset.people = signature; value.dataset.context = context;
    value.dataset.initialized = String(initialized || people.length > 0);
    for (const [index, person] of people.entries()) {
        const label = node("label"), input = node("input"); input.type = "checkbox"; input.value = person.role;
        input.checked = initialized ? selected.has(person.role) : index === 0;
        label.append(input, document.createTextNode(person.label)); value.append(label);
    }
}
function contactChoices() {
    choices("create-contacts", contacts, actor);
    const present = new Set(current?.participants.map(value => value.role) ?? []);
    for (const id of ["message-contacts", "invite-contacts"]) choices(id, contacts.filter(person => present.has(person.role)), actor + "/" + (selectedId ?? ""));
}
const selected = id => [...$(id).querySelectorAll("input:checked")].map(input => input.value);
function progress(values) {
    const list = $("operations"), signature = JSON.stringify(values);
    if (list.dataset.values !== signature) {
        list.dataset.values = signature; list.replaceChildren();
        for (const value of values) {
            const state = value.submission === "refused" ? "Not accepted" : value.state ??
                (value.submission === "prepared" ? "Saved, awaiting submission" : "Outcome uncertain");
            const item = node("li"); item.append(node("strong", label[value.command] ?? "Action"), document.createTextNode(" · " + state));
            if (value.needsRecovery || value.submission === "prepared") item.append(recoveryButton(
                value.submission === "prepared" ? "Submit saved action" : "Recover original", () => recover(value.operationId)));
            list.append(item);
        }
    }
    renderPending(); controls();
}
function renderPending() {
    const recovery = $("pending"); recovery.replaceChildren();
    if (pending) {
        recovery.append(node("p", "An original action needs observation. Recover it before starting another."), recoveryButton("Recover original action", recoverPending));
        if (pending.operationId === null) recovery.append(recoveryButton("Discard unsent preparation", () => { savePending(null); renderPending(); controls(); }));
    }
}
function actorSelection() {
    for (const input of $("actor").querySelectorAll("input[name=actor]")) input.checked = input.value === actor;
}
async function state(append = false) {
    const value = await api("state", append && nextConversations ? { cursor: nextConversations } : {});
    if (typeof value.actor !== "string" || !Array.isArray(value.roles)) { clearView(); problem(value.error ?? {}); return; }
    if (actor !== value.actor) { await stopMedia(); stopPreview(); selectedId = null; clearView(); actor = value.actor; readPending(); }
    const roles = JSON.stringify(value.roles), selector = $("actor-choices");
    if (selector.dataset.roles !== roles) {
        selector.dataset.roles = roles; selector.replaceChildren(...(value.roles ?? []).map(role => {
            const label = node("label"), input = node("input");
            input.type = "radio"; input.name = "actor"; input.value = role.role;
            label.append(input, document.createTextNode(role.label)); return label;
        }));
    }
    actorSelection(); contacts = value.contacts ?? []; progress(value.operations ?? []); contactChoices();
    if (value.error) { clearView(); problem(value.error); return; }
    const conversationList = $("conversations"), signature = JSON.stringify(value.conversations ?? []);
    if (!append && conversationList.dataset.page !== signature) conversationList.replaceChildren();
    if (!append) conversationList.dataset.page = signature;
    for (const conversation of value.conversations ?? []) {
        if ([...$("conversations").children].some(item => item.dataset.conversation === conversation.id)) continue;
        const item = node("li"); item.dataset.conversation = conversation.id;
        const select = button("Conversation " + ($("conversations").children.length + 1) + (conversation.closed ? " · closed" : ""), async () => {
            if (working || switching) return;
            if (selectedId !== conversation.id) $("message").value = "";
            await stopMedia(); retire(); stopPreview(); selectedId = conversation.id; historyCursor = null; clearView(); await view();
        });
        select.setAttribute("aria-current", String(conversation.id === selectedId)); item.append(select); $("conversations").append(item);
    }
    for (const item of conversationList.children) item.firstElementChild?.setAttribute("aria-current", String(item.dataset.conversation === selectedId));
    nextConversations = value.cursor; $("more-conversations").hidden = !nextConversations;
    const support = browserCaptureSupport(); $("microphone").disabled = !support.microphone; $("camera").disabled = !support.camera;
    $("media-capability").textContent = "Control connected through " + value.capabilities.transport.toUpperCase() + ". Microphone " +
        (support.microphone ? "available" : "unavailable") + "; camera " + (support.camera ? "available" : "unavailable") + ".";
    notice("Connected · Local synthetic participants. Outcomes below are last observed states.");
}
async function view() {
    if (!selectedId) return;
    try {
        const value = await api("view", { conversationId: selectedId, ...(historyCursor ? { cursor: historyCursor } : {}) });
        if (media && !media.closed && (!value.connections.some(c => c.id === media.binding?.connectionId &&
            c.sessionId === media.binding?.sessionId && ["ready", "active"].includes(c.state)) ||
            !value.sessions.some(s => s.id === media.binding?.sessionId && s.state === "accepted" && s.appearance === "present")))
            await stopMedia();
        const signature = JSON.stringify([value.conversation, value.participants, value.messages, value.sessions, value.connections, value.historyCursor]);
        if (current?.signature === signature) return;
        current = { ...value, signature }; contactChoices(); const cid = value.conversation.id;
        $("conversation-title").textContent = value.conversation.closed ? "Closed conversation" : "Conversation";
        $("participants").textContent = value.participants.map(person => person.label).join(" · ");
        $("timeline").replaceChildren(...value.messages.map(message => {
            const item = node("li"); item.append(node("strong", message.from), document.createTextNode(message.text)); return item;
        }));
        nextHistory = value.historyCursor; $("more-messages").hidden = !nextHistory;
        $("sessions").replaceChildren(...value.sessions.map(session => {
            const item = node("li"); item.append(node("strong", session.media.join(" + ") || "Session"), document.createTextNode(" · " + session.state));
            const action = type => mutate({ type, conversationId: cid, sessionId: session.id });
            if (session.appearance === "invited") item.append(button("Accept", () => action("session.accept"), true), button("Decline", () => action("session.reject"), true));
            if (session.state === "accepted" && session.appearance === "present" && session.media.includes("audio") &&
                !value.connections.some(v => v.sessionId === session.id && v.state !== "retired")) {
                const join = medium => mutate({ type: "connection.offer", conversationId: cid, sessionId: session.id, medium });
                item.append(button("Join audio", () => join("voice"), true));
                if (session.media.includes("video")) item.append(button("Join audio and video", () => join("voice+video"), true));
            }
            if (!["ended", "failed", "cancelled", "rejected"].includes(session.state)) item.append(button("End session", () => action("session.end"), true));
            return item;
        }));
        $("connections").replaceChildren(...value.connections.map(connection => {
            const item = node("li", "Connection · " + connection.state);
            if (["ready", "active"].includes(connection.state)) {
                const binding = { conversationId: cid, sessionId: connection.sessionId, connectionId: connection.id };
                item.append(button("Start microphone", () => startMedia(binding, false), true));
                if (connection.media?.includes("video")) item.append(button("Start microphone and camera", () => startMedia(binding, true), true));
                for (const [text, action] of [["Mute", "mute"], ["Unmute", "unmute"], ["Hold", "hold"], ["Resume", "resume"]])
                    item.append(button(text, () => mutate({ type: "connection.update", conversationId: cid, sessionId: connection.sessionId, connectionId: connection.id, action }), true));
                for (const peer of connection.peers ?? [])
                    item.append(button("Listen to " + peer.label, () => mutate({ type: "stream.subscribe", conversationId: cid, sessionId: connection.sessionId, connectionId: connection.id, peer: peer.role }), true));
                item.append(button("Stop listening", () => mutate({ type: "stream.unsubscribe", conversationId: cid, sessionId: connection.sessionId, connectionId: connection.id }), true));
                item.append(button("End connection", () => mutate({ type: "connection.end", conversationId: cid, sessionId: connection.sessionId, connectionId: connection.id }), true));
            }
            return item;
        })); controls();
    } catch (error) { if (error.code !== "view-changed") clearView(); throw error; }
}
const settled = value => value.operation?.submission === "refused" || ["failed", "cancelled"].includes(value.operation?.state) ||
    (value.operation?.state === "completed" && value.result !== null);
async function mutate(action) {
    if (working || switching || pending || storageBlocked) return; working = true; controls();
    try {
        // Stop local capture/render before a control can change its binding.
        if (action.type.startsWith("connection.") || action.type.startsWith("stream.") || action.type === "session.end") await stopMedia();
        savePending({ key: "ui_" + crypto.randomUUID(), operationId: null });
        const prepared = await api("prepare", { key: pending.key, action });
        savePending({ ...pending, operationId: prepared.operation.operationId });
        const value = await api("submit", { operationId: pending.operationId });
        if (settled(value)) savePending(null);
        if (value.result?.messageId && action.type === "message.send" && $("message").value === action.text) $("message").value = "";
        if (value.result?.conversationId) selectedId = value.result.conversationId;
        await state(); await view(); if (value.error) problem(value.error);
    } catch (error) { problem(error); } finally { working = false; controls(); }
}
async function recoverPending() {
    if (!pending || working || switching) return;
    if (!pending.operationId) {
        const value = await api("lookup", { key: pending.key });
        if (!value.operation) { notice("Preparation is still unobserved. No submission was issued by this page.", true); return; }
        savePending({ ...pending, operationId: value.operation.operationId });
    }
    await recover(pending.operationId);
}
async function recover(operationId) {
    if (working || switching) return; working = true; controls();
    try {
        const value = await api("recover", { operationId });
        if (settled(value) && pending?.operationId === operationId) savePending(null);
        if (value.result?.conversationId) selectedId = value.result.conversationId;
        await state(); await view(); if (value.error) problem(value.error);
    } finally { working = false; controls(); }
}
async function stopMedia() {
    const old = media; if (!old) return;
    media = null; controls();
    await old.stop();
    if (!media) {
        $("media-status").textContent = "Media stopped. No device is active.";
        $("media-status").dataset.metrics = JSON.stringify(old.diagnostics);
    }
}
async function startMedia(binding, video) {
    if (working || switching || pending || storageBlocked) return;
    working = true; controls(); stopPreview();
    try {
        await stopMedia();
        const next = new BrowserAppMedia({ onState: (state, metrics) => {
            if (media !== next) return;
            $("media-status").dataset.state = state;
            $("media-status").dataset.metrics = JSON.stringify(metrics);
            $("media-status").textContent = state === "active" ? "Microphone shared" + (video ? " with camera" : "") + ". " +
                (metrics.playedAudioFrames ? "Incoming audio playing." : "Waiting for incoming audio.") :
                state === "permission" ? "Waiting for microphone" + (video ? " and camera" : "") + " permission…" :
                state === "starting" ? "Starting media…" : explanations[state] ?? "Media stopped. No device is active.";
        } });
        media = next; controls();
        await next.start(binding, { video, preview: $("preview"), remote: $("remote-video") });
    } catch (error) { problem(error); }
    finally { working = false; controls(); }
}
function stopPreview() {
    captureEpoch++; for (const stream of previews) for (const track of stream.getTracks()) track.stop();
    previews.clear(); $("preview").srcObject = null;
}
async function preview(kind) {
    await stopMedia(); stopPreview(); const captured = captureEpoch; let expired = false;
    $("permission").textContent = "Waiting for " + kind + " permission…";
    const timeout = setTimeout(() => { expired = true; if (captured === captureEpoch) $("permission").textContent = "Permission is still pending. No device is active."; }, 8000);
    try {
        // Honor an existing browser denial before requesting hardware,
        // including on hosts without capture devices.
        let denied = false;
        try { denied = (await navigator.permissions?.query({ name: kind }))?.state === "denied"; } catch { /* This browser may not expose device permission queries. */ }
        if (expired || captured !== captureEpoch) return;
        if (denied) { $("permission").textContent = kind + " permission denied. You can continue with text."; return; }
        const stream = await navigator.mediaDevices.getUserMedia(kind === "microphone" ? { audio: true } : { video: true });
        if (expired || captured !== captureEpoch) { for (const track of stream.getTracks()) track.stop(); return; }
        previews.add(stream); if (kind === "camera") $("preview").srcObject = stream;
        $("permission").textContent = "Local " + kind + " preview active.";
    } catch (error) { if (captured === captureEpoch) $("permission").textContent = error.name === "NotAllowedError" ?
        kind + " permission denied. You can continue with text." : kind + " unavailable. Check browser device settings.";
    } finally { clearTimeout(timeout); }
}
$("actor").addEventListener("change", async event => {
    if (!(event.target instanceof HTMLInputElement) || event.target.name !== "actor") return;
    if (working || switching) { actorSelection(); return; }
    const requested = event.target.value, restoreFocus = document.activeElement === event.target;
    switching = true;
    retire(); await stopMedia(); stopPreview(); selectedId = null; historyCursor = null; contacts = []; $("message").value = "";
    $("conversations").replaceChildren(); delete $("conversations").dataset.page;
    clearView(); contactChoices();
    try { const value = await api("actor", { role: requested }); actor = value.actor; readPending(); await state(); }
    catch (error) { problem(error); } finally {
        switching = false; actorSelection(); controls();
        // A disabled fieldset can move focus to the page while access refreshes.
        // Return it to the selected participant unless the user moved elsewhere.
        if (restoreFocus && document.activeElement === document.body) $("actor").querySelector("input:checked")?.focus();
    }
});
$("create-form").addEventListener("submit", event => { event.preventDefault(); mutate({ type: "conversation.create", recipients: selected("create-contacts") }); });
$("message-form").addEventListener("submit", event => { event.preventDefault(); mutate({ type: "message.send", conversationId: selectedId, recipients: selected("message-contacts"), text: $("message").value }); });
$("invite-form").addEventListener("submit", event => { event.preventDefault(); mutate({ type: "session.invite", conversationId: selectedId, recipients: selected("invite-contacts"), medium: $("medium").value }); });
$("reconnect").addEventListener("click", async () => {
    if (working || switching) return; working = true; retire(); await stopMedia(); stopPreview(); controls();
    notice("Reconnecting. Reading current access and original outcomes…");
    try { await api("reconnect"); await state(); await view(); }
    catch (error) { clearView(); problem(error); } finally { working = false; controls(); }
});
$("refresh").addEventListener("click", async () => { historyCursor = null; try { await state(); await view(); } catch (error) { problem(error); } });
$("more-conversations").addEventListener("click", () => state(true).catch(problem));
$("more-messages").addEventListener("click", () => { historyCursor = nextHistory; view().catch(problem); });
$("microphone").addEventListener("click", () => preview("microphone")); $("camera").addEventListener("click", () => preview("camera"));
$("stop-media").addEventListener("click", () => { void stopMedia().catch(problem); });
$("stop-preview").addEventListener("click", () => { stopPreview(); $("permission").textContent = "Local preview stopped."; });
window.addEventListener("offline", () => { retire(); void stopMedia(); stopPreview(); clearView(); renderPending(); notice("Offline. Saved actions remain available for original-ID recovery.", true); });
window.addEventListener("online", () => notice("Network available. Reconnect to refresh current access and outcomes."));
window.addEventListener("pagehide", () => { clearTimeout(timer); retire(); void stopMedia(); stopPreview(); });
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") { void stopMedia(); stopPreview(); } });
async function tick() {
    if (!working && !switching && navigator.onLine !== false && document.visibilityState !== "hidden") try { await state(); await view(); } catch (error) { problem(error); }
    timer = setTimeout(tick, 5000);
}
readPending(); controls(); await tick();
