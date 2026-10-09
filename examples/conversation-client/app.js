import {
    BrowserHttpClient,
    BrowserVoiceSession,
    ConnectError,
} from "../../sdk/typescript/dist/browser.js";

const byId = (id) => document.getElementById(id);
let client;
let voice;
const value = (id) => byId(id).value.trim();
const requestId = (prefix) => `${prefix}-${crypto.randomUUID()}`;

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
        throw error;
    }
}

byId("connect").onclick = () =>
    act("Connected", async () => {
        client = new BrowserHttpClient(value("http"), value("credential"));
        const health = await client.health();
        report("Connected", `Schema ${health.schema} · ${health.profile}`);
    });
byId("create").onclick = () =>
    act("Conversation created", async () => {
        const conversation = await needClient().create(requestId("create"), [
            value("participant"),
        ]);
        byId("conversation").value = conversation.id;
        await refresh();
    });
byId("refresh").onclick = () => act("Timeline refreshed", refresh);

async function refresh() {
    const id = needConversation();
    const [history, context] = await Promise.all([
        needClient().history(id, null, 100),
        needClient().contextView(id, 100),
    ]);
    const list = byId("messages");
    list.replaceChildren();
    const displayed = new Set(history.messages.map((message) => message.id));
    for (const message of history.messages) {
        const item = document.createElement("li");
        const body = document.createElement("div");
        body.textContent = message.body;
        const meta = document.createElement("small");
        meta.textContent = `${message.sender} · message ${message.id}`;
        item.append(body, meta);
        list.append(item);
    }
    for (const item of context.items.filter(
        (item) => item.kind !== "message" && !displayed.has(item.source_id),
    )) {
        const row = document.createElement("li");
        const body = document.createElement("div");
        body.textContent = item.body;
        const meta = document.createElement("small");
        meta.textContent = `${item.kind} · sequence ${item.sequence}`;
        row.append(body, meta);
        list.append(row);
    }
    byId("watermark").textContent = `watermark ${context.watermark}`;
    list.scrollTop = list.scrollHeight;
}

byId("message-form").onsubmit = (event) => {
    event.preventDefault();
    act("Message sent", async () => {
        await needClient().send(
            requestId("send"),
            needConversation(),
            value("message"),
        );
        byId("message").value = "";
        await refresh();
    });
};
for (const button of document.querySelectorAll("[data-availability]"))
    button.onclick = () =>
        act("Availability updated", async () => {
            const state = button.dataset.availability;
            const result = await needClient().availabilitySet(
                requestId("availability"),
                state,
                Math.floor(Date.now() / 1000) + 3600,
                state === "available" ? "webrtc" : null,
            );
            byId("availability").textContent =
                `${result.state} until ${new Date(result.expires_at * 1000).toLocaleTimeString()}`;
        });

byId("upload").onclick = () =>
    act("File uploaded", async () => {
        const file = byId("file").files[0];
        if (!file) throw new Error("Choose a file first.");
        const bytes = new Uint8Array(await file.arrayBuffer());
        const digest = Array.from(
            new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
            (byte) => byte.toString(16).padStart(2, "0"),
        ).join("");
        const artifact = await needClient().artifactBegin(
            requestId("file-begin"),
            needConversation(),
            file.name,
            file.type || "application/octet-stream",
            bytes.length,
            digest,
        );
        let offset = 0;
        while (offset < bytes.length) {
            const chunk = bytes.slice(
                offset,
                Math.min(offset + 32768, bytes.length),
            );
            await client.uploadArtifactChunk(
                needConversation(),
                artifact.id,
                requestId("file-chunk"),
                offset,
                chunk,
            );
            offset += chunk.length;
            byId("file-state").textContent =
                `${offset} / ${bytes.length} bytes`;
        }
        await client.artifactFinish(
            requestId("file-finish"),
            needConversation(),
            artifact.id,
        );
        byId("file-state").textContent = `Ready · ${artifact.id}`;
        await refresh();
    });

byId("invite").onclick = () =>
    act("Audio invited", async () => {
        const session = await needClient().sessionInvite(
            requestId("invite"),
            needConversation(),
            value("participant"),
            ["audio"],
        );
        byId("session").value = session.id;
        await refresh();
    });
byId("accept").onclick = () =>
    act("Session accepted", async () => {
        await needClient().sessionAccept(
            requestId("accept"),
            needConversation(),
            value("session"),
        );
        await refresh();
    });
byId("end").onclick = () =>
    act("Session ended", async () => {
        await needClient().sessionEnd(
            requestId("end"),
            needConversation(),
            value("session"),
            "human-test-complete",
        );
        await refresh();
    });

byId("join").onclick = () =>
    act("Voice connected", async () => {
        voice = await BrowserVoiceSession.open(
            value("webrtc"),
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
