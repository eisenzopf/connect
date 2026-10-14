import { BrowserVideoSession } from "../../sdk/typescript/dist/browser.js";
import {
    act,
    mutate,
    needClient,
    needConversation,
    value,
    refresh,
} from "./app.js";
import { configuredVoiceBinding } from "./workflow.js";
const byId = (id) => document.getElementById(id);
let session,
    camera = false,
    microphone = false;
function reset() {
    for (const id of ["local-video", "remote-video"]) {
        byId(id).pause();
        byId(id).srcObject = null;
    }
    for (const id of ["end-video", "mute-camera", "mute-microphone"])
        byId(id).disabled = true;
    for (const id of ["audio-device", "video-device", "refresh-devices"])
        byId(id).disabled = false;
    byId("mute-camera").textContent = "Mute camera";
    byId("mute-microphone").textContent = "Mute microphone";
    byId("join-video").disabled = false;
    session = undefined;
    camera = false;
    microphone = false;
}
byId("invite-video").onclick = () =>
    act("Video invited", async () => {
        const invited = await mutate(`invite:${needConversation()}`, {
            op: "session_invite",
            conversation_id: needConversation(),
            participant_id: value("participant"),
            media: ["audio", "video"],
        });
        byId("session").value = invited.id;
        await refresh();
    });
byId("join-video").onclick = () =>
    act("Video connected", async () => {
        const binding = await configuredVoiceBinding(
            needClient(),
            needConversation(),
            value("participant"),
            value("session"),
        );
        if (binding.session_id !== value("session"))
            throw new Error(
                "Select the configured accepted video Session before capture.",
            );
        const state = await needClient().sessionGet(
            needConversation(),
            binding.session_id,
        );
        if (
            !["accepted", "ready"].includes(state.state) ||
            !state.media.includes("video")
        )
            throw new Error("Accept an audio/video Session first.");
        session = await BrowserVideoSession.open(
            binding.signaling_url,
            value("credential"),
            15000,
            {
                conversationId: binding.conversation_id,
                sessionId: binding.session_id,
                audioDeviceId: byId("audio-device").value || undefined,
                videoDeviceId: byId("video-device").value || undefined,
            },
        );
        const current = session;
        current.ended.then(() => {
            if (session === current) reset();
        });
        byId("local-video").srcObject = current.localPreview;
        try {
            await current.attach(byId("remote-video"));
            if (session !== current)
                throw new Error("Video ended while playback was starting. Join again to reconnect.");
        } catch (error) {
            await current.close();
            if (session === current) reset();
            throw error;
        }
        byId("join-video").disabled = true;
        for (const id of ["audio-device", "video-device", "refresh-devices"])
            byId(id).disabled = true;
        for (const id of ["end-video", "mute-camera", "mute-microphone"])
            byId(id).disabled = false;
    });
byId("end-video").onclick = () =>
    act("Video left", async () => {
        await session?.close();
        reset();
        await refresh();
    });
byId("mute-camera").onclick = () => {
    camera = !camera;
    session?.setCameraMuted(camera);
    byId("mute-camera").textContent = camera ? "Unmute camera" : "Mute camera";
};
byId("mute-microphone").onclick = () => {
    microphone = !microphone;
    session?.setMicrophoneMuted(microphone);
    byId("mute-microphone").textContent = microphone
        ? "Unmute microphone"
        : "Mute microphone";
};
byId("video-diagnostics").onclick = () =>
    act("Video inspected", async () => {
        if (!session) throw new Error("Join video first.");
        byId("video-proof").textContent = JSON.stringify(
            await session.diagnostics(),
            null,
            2,
        );
    });
window.addEventListener("beforeunload", () => {
    void session?.close();
});

byId("refresh-devices").onclick = () =>
    act("Devices listed", async () => {
        if (session) throw new Error("Leave video before choosing devices.");
        const devices = await navigator.mediaDevices.enumerateDevices();
        if (devices.length > 128)
            throw new Error("Device list exceeds the supported limit.");
        for (const [id, kind] of [
            ["audio-device", "audioinput"],
            ["video-device", "videoinput"],
        ]) {
            const select = byId(id),
                previous = select.value;
            const defaultOption = document.createElement("option");
            defaultOption.value = "";
            defaultOption.textContent = "System default";
            select.replaceChildren(defaultOption);
            const seen = new Set();
            for (const device of devices.filter((item) => item.kind === kind)) {
                if (
                    !device.deviceId ||
                    device.deviceId.length > 512 ||
                    seen.has(device.deviceId)
                )
                    continue;
                seen.add(device.deviceId);
                const option = document.createElement("option");
                option.value = device.deviceId;
                option.textContent =
                    device.label.slice(0, 128) ||
                    `${kind === "audioinput" ? "Microphone" : "Camera"} ${seen.size}`;
                select.append(option);
            }
            select.value = seen.has(previous) ? previous : "";
        }
    });
