import { readCanonicalMedia } from "./canonical-media.js";
import { renderArtifactHistory } from "./files.js";
import {
    IntentJournal,
    historyPage,
} from "./workflow.js";
import {
    BrowserHttpClient,
    BrowserVoiceSession,
    ConnectError,
} from "../../sdk/typescript/dist/browser.js";

const byId = (id) => document.getElementById(id);
let client;
let actionRunning = false;
let voice;
let voiceMedia;
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
    byId("pending").textContent =
        `${entries.filter((x) => !x.result).length} unresolved actions; saved inputs stay in this tab across refresh. Re-select the same file to resume an upload.`;
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
    } finally {
        actionRunning = false;
    }
}

byId("connect").onclick = () =>
    act("Connected", async () => {
        client = new BrowserHttpClient(value("http"), value("credential"));
        const identity = value("identity");
        if (!identity)
            throw new Error(
                "Enter your identity ID so saved intent stays scoped to this identity.",
            );
        intents = new IntentJournal(
            sessionStorage,
            `${value("http")}:${identity}`,
        );
        const health = await client.health();
        showPending();
        report("Connected", `Schema ${health.schema} · ${health.profile}`);
    });
byId("create").onclick = () =>
    act("Conversation created", async () => {
        const conversation = await mutate("create", {
            op: "create",
            participants: [value("participant")],
        });
        byId("conversation").value = conversation.id;
        await refresh();
    });
byId("refresh").onclick = () => act("Timeline refreshed", refresh);

async function refresh() {
    const id = needConversation();
    if (historyConversation !== id) {
        historyConversation = id;
        historyCursor = null;
        historyDone = false;
        byId("messages").replaceChildren();
    }
    // Continue from the durable cursor; refresh never resets to the oldest page.
    await loadHistory();
    const context = await needClient().contextView(id, 100);
    if (id !== needConversation()) return;
    byId("watermark").textContent = `watermark ${context.watermark}`;
    byId("context-state").textContent =
        `${context.omitted} source items omitted from the bounded model snapshot; ${context.deliveries_omitted || 0} older delivery receipts. History remains retained.`;
    const events = byId("context-events");
    events.replaceChildren();
    for (const item of context.items.filter(
        (item) => item.kind !== "message",
    )) {
        const row = document.createElement("li");
        row.textContent = `${item.kind} · sequence ${item.sequence} · ${item.body}`;
        events.append(row);
    }
    deliveryCursor = null;
    byId("deliveries").replaceChildren();
    await loadDeliveries();
    renderArtifactHistory(context.items);
    showPending();
}
async function loadHistory() {
    const id = needConversation();
    const page = await historyPage(needClient(), id, historyCursor);
    if (id !== needConversation()) return;
    const list = byId("messages");
    for (const message of page.messages) {
        const item = document.createElement("li");
        const body = document.createElement("div");
        body.textContent = message.body;
        const meta = document.createElement("small");
        meta.textContent = `${message.sender} · message ${message.id}`;
        item.append(body, meta);
        list.append(item);
    }
    if (page.next) historyCursor = page.next;
    // The server can end a page early at its byte budget. Only an empty page
    // proves exhaustion; a short page can still have a continuation cursor.
    historyDone = page.messages.length === 0;
    byId("more-history").disabled = historyDone;
    list.scrollTop = list.scrollHeight;
}
async function loadDeliveries() {
    const page = await needClient().contextDeliveries(
        needConversation(),
        deliveryCursor,
        25,
    );
    for (const receipt of page.deliveries) {
        const row = document.createElement("li");
        row.textContent = `${receipt.state} · ${receipt.provider} · ${receipt.session_id || "chat"} · watermark ${receipt.watermark} · ${receipt.id}`;
        byId("deliveries").append(row);
    }
    deliveryCursor = page.next;
    byId("more-deliveries").disabled = !deliveryCursor;
}
byId("more-history").onclick = () => act("History loaded", loadHistory);
byId("more-deliveries").onclick = () =>
    act("Delivery receipts loaded", loadDeliveries);
byId("retry").onclick = () =>
    act("Saved action recovered", async () => {
        const retried = await journal().retry(needClient());
        if (retried?.result.kind === "conversation")
            byId("conversation").value = retried.result.data.id;
        if (retried && !retried.slot.startsWith("upload:"))
            journal().forget(retried.slot);
        showPending();
        if (value("conversation")) await refresh();
    });

byId("message-form").onsubmit = (event) => {
    event.preventDefault();
    act("Message sent", async () => {
        await mutate(`send:${needConversation()}`, {
            op: "send",
            conversation_id: needConversation(),
            message: {
                to: "all",
                content_type: "text/plain",
                body: value("message"),
                body_encoding: "utf8",
                label: "connect-messages",
                attachments: [],
                in_reply_to_msg: null,
                msg_id: null,
            },
        });
        byId("message").value = "";
        await refresh();
    });
};
for (const button of document.querySelectorAll("[data-availability]"))
    button.onclick = () =>
        act("Availability updated", async () => {
            const state = button.dataset.availability;
            const result = await mutate("availability", {
                op: "availability_set",
                state,
                preferred_route: state === "available" ? "webrtc" : null,
                expires_at: Math.floor(Date.now() / 1000) + 3600,
            });
            byId("availability").textContent =
                `${result.state} until ${new Date(result.expires_at * 1000).toLocaleTimeString()}`;
        });

byId("invite").onclick = () =>
    act("Audio invited", async () => {
        const session = await mutate(`invite:${needConversation()}`, {
            op: "session_invite",
            conversation_id: needConversation(),
            participant_id: value("participant"),
            media: ["audio"],
        });
        byId("session").value = session.id;
        await refresh();
    });
byId("accept").onclick = () =>
    act("Session accepted", async () => {
        await mutate(`accept:${value("session")}`, {
            op: "session_accept",
            conversation_id: needConversation(),
            session_id: value("session"),
        });
        await refresh();
    });
byId("reject").onclick = () =>
    act("Session rejected", async () => {
        await mutate(`reject:${value("session")}`, {
            op: "session_reject",
            conversation_id: needConversation(),
            session_id: value("session"),
            reason: "human-declined",
        });
        await refresh();
    });
byId("end").onclick = () =>
    act("Session ended", async () => {
        await mutate(`end:${value("session")}`, {
            op: "session_end",
            conversation_id: needConversation(),
            session_id: value("session"),
            reason: "human-test-complete",
        });
        await refresh();
    });

byId("join").onclick = () =>
    act("Voice connected", async () => {
        voiceMedia?.client.close();
        voiceMedia = await needMedia();
        byId("voice-binding").textContent = `Voice Conversation: ${voiceMedia.options.conversationId} · Session: ${voiceMedia.options.sessionId}`;
        try { voice = await BrowserVoiceSession.open(voiceMedia.client, voiceMedia.options); }
        catch (error) { voiceMedia.client.close(); throw error; }
        const activeVoice = voice;
        void activeVoice.ended.then(() => {
            if (voice !== activeVoice) return;
            voice = undefined;
            byId("remote").pause();
            byId("remote").srcObject = null;
            byId("join").disabled = false;
            byId("hangup").disabled = true;
            byId("receive-voice").disabled = true;
            report(
                "Voice ended",
                "This voice leg ended; the conversation remains available.",
            );
        });
        try {
            await activeVoice.attach(byId("remote"));
            if (voice !== activeVoice)
                throw new ConnectError(
                    "webrtc-failed",
                    "Voice ended during playback startup",
                );
        } catch (error) {
            await activeVoice.close();
            if (voice === activeVoice) voice = undefined;
            throw error;
        }
        byId("join").disabled = true;
        byId("hangup").disabled = false;
        byId("receive-voice").disabled = false;
    });
byId("receive-voice").onclick = () => act("Receive selection confirmed", async()=>{
    if (!voice || !voiceMedia) throw Error("Join voice first.");
    await voiceMedia.receive(); byId("receive-voice").disabled = true;
});
byId("voice-diagnostics").onclick = () =>
    act("Voice path inspected", async () => {
        if (!voice) throw new Error("Join voice first.");
        byId("voice-proof").textContent = JSON.stringify(
            await voice.diagnostics(),
            null,
            2,
        );
    });
byId("hangup").onclick = () =>
    act("Voice ended", async () => {
        const current = voice, media = voiceMedia;
        try { if (current) await current.end(media.end.operationId, media.end.expectedRevision); }
        finally { media?.client.close(); }
        voice = undefined;
        byId("join").disabled = false;
        byId("hangup").disabled = true;
            byId("receive-voice").disabled = true;
        await refresh();
    });
window.addEventListener("beforeunload", () => {
    void voice?.close();
    voiceMedia?.client.close();
});

async function needMedia() {
    const media = await readCanonicalMedia(byId("media-config").files?.[0], {
        conversationId: needConversation(), sessionId: value("session"), identityId: value("identity"),
    });
    byId("webrtc").value = media.url;
    return media;
}

// Narrow reference-client hook: modules use the same public client and saved intent.
export {
    act,
    mutate,
    needClient,
    needMedia,
    needConversation,
    value,
    refresh,
    journal,
    showPending,
};
