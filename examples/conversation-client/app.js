import { IntentJournal, historyPage, configuredVoiceBinding } from "./workflow.js";
import {
    BrowserHttpClient,
    BrowserVoiceSession,
    ConnectError,
} from "../../sdk/typescript/dist/browser.js";

const byId = (id) => document.getElementById(id);
let client;
let actionRunning = false;
let voice;
const value = (id) => byId(id).value.trim();
let intents;
let historyCursor = null;
let historyConversation = null;
let historyDone = false;
let deliveryCursor = null;
function journal() {
    if (!intents) throw new Error("Connect the local API first.");
    return intents;
}
async function mutate(slot, operation, retain = false) {
    const outcome = await journal().run(needClient(), slot, operation);
    if (!retain) journal().forget(slot);
    showPending();
    return outcome.data;
}
function showPending() {
    const entries = intents ? Object.values(intents.read()) : [];
    byId("pending").textContent = `${entries.filter(x=>!x.result).length} unresolved actions; saved inputs stay in this tab across refresh. Re-select the same file to resume an upload.`;
}

function report(state, detail, error = false) {
    byId("state").textContent = state;
    byId("detail").textContent = detail;
    byId("state").style.color = error ? "#ff8ca3" : "#d899ff";
}
function needClient() {
    if (!client) throw new Error("Connect the local API first.");
    return client;
}
function needConversation() {
    const id = value("conversation");
    if (!id) throw new Error("Choose or create a conversation.");
    return id;
}
async function act(label, action) {
    if (actionRunning) return;
    actionRunning = true;
    try {
        report(label, "Working…");
        const result = await action();
        report(label, "Complete.");
        return result;
    } catch (error) {
        const detail =
            error instanceof ConnectError
                ? `${error.code}: ${error.message}`
                : error.message;
        report("Action failed", detail, true);
        showPending();
    } finally { actionRunning = false; }
}

byId("connect").onclick = () =>
    act("Connected", async () => {
        client = new BrowserHttpClient(value("http"), value("credential"));
        const identity = value("identity");
        if (!identity) throw new Error("Enter your identity ID so saved intent stays scoped to this identity.");
        intents = new IntentJournal(sessionStorage, `${value("http")}:${identity}`);
        const health = await client.health();
        showPending();
        report("Connected", `Schema ${health.schema} · ${health.profile}`);
    });
byId("create").onclick = () =>
    act("Conversation created", async () => {
        const conversation = await mutate("create", {op: "create", participants: [value("participant")]});
        byId("conversation").value = conversation.id;
        await refresh();
    });
byId("refresh").onclick = () => act("Timeline refreshed", refresh);

async function refresh() {
    const id = needConversation();
    if (historyConversation !== id) {
        historyConversation = id; historyCursor = null; historyDone = false;
        byId("messages").replaceChildren();
    }
    // Continue from the durable cursor; refresh never resets to the oldest page.
    await loadHistory();
    const context = await needClient().contextView(id, 100);
    if (id !== needConversation()) return;
    byId("watermark").textContent = `watermark ${context.watermark}`;
    byId("context-state").textContent = `${context.omitted} source items omitted from the bounded model snapshot; ${context.deliveries_omitted || 0} older delivery receipts. History remains retained.`;
    const events = byId("context-events"); events.replaceChildren();
    for (const item of context.items.filter(item => item.kind !== "message")) {
        const row = document.createElement("li");
        row.textContent = `${item.kind} · sequence ${item.sequence} · ${item.body}`;
        events.append(row);
    }
    deliveryCursor = null;
    byId("deliveries").replaceChildren();
    await loadDeliveries();
    showPending();
}
async function loadHistory() {
    const id = needConversation();
    const page = await historyPage(needClient(), id, historyCursor);
    if (id !== needConversation()) return;
    const list = byId("messages");
    for (const message of page.messages) {
        const item = document.createElement("li");
        const body = document.createElement("div"); body.textContent = message.body;
        const meta = document.createElement("small"); meta.textContent = `${message.sender} · message ${message.id}`;
        item.append(body, meta); list.append(item);
    }
    if (page.next) historyCursor = page.next;
    historyDone = page.messages.length < 100;
    byId("more-history").disabled = historyDone;
    list.scrollTop = list.scrollHeight;
}
async function loadDeliveries() {
    const page = await needClient().contextDeliveries(needConversation(), deliveryCursor, 25);
    for (const receipt of page.deliveries) {
        const row = document.createElement("li");
        row.textContent = `${receipt.state} · ${receipt.provider} · ${receipt.session_id || "chat"} · watermark ${receipt.watermark} · ${receipt.id}`;
        byId("deliveries").append(row);
    }
    deliveryCursor = page.next;
    byId("more-deliveries").disabled = !deliveryCursor;
}
byId("more-history").onclick = () => act("History loaded", loadHistory);
byId("more-deliveries").onclick = () => act("Delivery receipts loaded", loadDeliveries);
byId("retry").onclick = () => act("Saved action recovered", async () => {
    const retried = await journal().retry(needClient());
    if (retried?.result.kind === "conversation") byId("conversation").value = retried.result.data.id;
    if (retried && !retried.slot.startsWith("upload:")) journal().forget(retried.slot);
    showPending();
    if (value("conversation")) await refresh();
});

byId("message-form").onsubmit = (event) => {
    event.preventDefault();
    act("Message sent", async () => {
        await mutate(`send:${needConversation()}`, {op: "send", conversation_id: needConversation(), body: value("message")});
        byId("message").value = "";
        await refresh();
    });
};
for (const button of document.querySelectorAll("[data-availability]"))
    button.onclick = () =>
        act("Availability updated", async () => {
            const state = button.dataset.availability;
            const result = await mutate("availability", {op:"availability_set", state, preferred_route:state === "available" ? "webrtc" : null, expires_at:Math.floor(Date.now()/1000)+3600});
            byId("availability").textContent =
                `${result.state} until ${new Date(result.expires_at * 1000).toLocaleTimeString()}`;
        });

byId("upload").onclick = () => act("File uploaded", async () => {
    const file = byId("file").files[0];
    if (!file) throw new Error("Select the original file to start or resume.");
    if (file.size > 16 * 1024 * 1024) throw new Error("File exceeds the 16 MiB local profile.");
    const cid = needConversation();
    const bytes = new Uint8Array(await file.arrayBuffer());
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))).map(b=>b.toString(16).padStart(2,"0")).join("");
    const prefix = `upload:${cid}`;
    const artifact = await mutate(`${prefix}:begin`, {op:"artifact_begin", conversation_id:cid, name:file.name, media_type:file.type || "application/octet-stream", size:bytes.length, sha256:digest}, true);
    for (let offset=journal().progress(prefix); offset<bytes.length; offset+=32768) {
        const chunk = bytes.slice(offset, offset+32768);
        const data = btoa(String.fromCharCode(...chunk));
        // Save one chunk intent; advance local progress only after its receipt is saved.
        await mutate(`${prefix}:${offset}`, {op:"artifact_append", conversation_id:cid, artifact_id:artifact.id, offset, data}, true);
        journal().saveProgress(prefix, offset+chunk.length);
        journal().forget(`${prefix}:${offset}`);
        byId("file-state").textContent = `${Math.min(offset+chunk.length,bytes.length)} / ${bytes.length} bytes`;
    }
    await mutate(`${prefix}:finish`, {op:"artifact_finish", conversation_id:cid, artifact_id:artifact.id}, true);
    for (const slot of Object.keys(journal().read()).filter(k=>k.startsWith(prefix+":"))) journal().forget(slot);
    journal().saveProgress(prefix, 0);
    byId("file-state").textContent = `Ready · ${artifact.id}`;
    await refresh();
});

byId("invite").onclick = () =>
    act("Audio invited", async () => {
        const session = await mutate(`invite:${needConversation()}`, {op:"session_invite", conversation_id:needConversation(), participant_id:value("participant"), media:["audio"]});
        byId("session").value = session.id;
        await refresh();
    });
byId("accept").onclick = () =>
    act("Session accepted", async () => {
        await mutate(`accept:${value("session")}`, {op:"session_accept", conversation_id:needConversation(), session_id:value("session")});
        await refresh();
    });
byId("end").onclick = () =>
    act("Session ended", async () => {
        await mutate(`end:${value("session")}`, {op:"session_end", conversation_id:needConversation(), session_id:value("session"), reason:"human-test-complete"});
        await refresh();
    });

byId("join").onclick = () =>
    act("Voice connected", async () => {
        const binding = await configuredVoiceBinding(needClient(), needConversation(), value("participant"));
        byId("voice-binding").textContent = `Voice conversation: ${binding.conversation_id} · participant: ${binding.participant_id}`;
        byId("webrtc").value = binding.signaling_url;
        voice = await BrowserVoiceSession.open(
            binding.signaling_url,
            value("credential"),
        );
        const activeVoice = voice;
        void activeVoice.ended.then(() => {
            if (voice !== activeVoice) return;
            voice = undefined;
            byId("remote").pause();
            byId("remote").srcObject = null;
            byId("join").disabled = false;
            byId("hangup").disabled = true;
            report("Voice ended", "This voice leg ended; the conversation remains available.");
        });
        try {
            await activeVoice.attach(byId("remote"));
            if (voice !== activeVoice)
                throw new ConnectError("webrtc-failed", "Voice ended during playback startup");
        } catch (error) {
            await activeVoice.close();
            if (voice === activeVoice) voice = undefined;
            throw error;
        }
        byId("join").disabled = true;
        byId("hangup").disabled = false;
    });
byId("voice-diagnostics").onclick = () =>
    act("Voice path inspected", async () => {
        if (!voice) throw new Error("Join voice first.");
        byId("voice-proof").textContent = JSON.stringify(await voice.diagnostics(), null, 2);
    });
byId("hangup").onclick = () =>
    act("Voice ended", async () => {
        await voice?.close();
        voice = undefined;
        byId("join").disabled = false;
        byId("hangup").disabled = true;
        await refresh();
    });
window.addEventListener("beforeunload", () => {
    void voice?.close();
});
