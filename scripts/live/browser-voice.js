/** Explicit live browser rehearsal; never imported by the offline runner. */
import {
    BrowserVoiceSession,
    BrowserHttpClient,
} from "../../sdk/typescript/dist/browser.js";
const byId = (id) => document.getElementById(id);
let current;
let captured, uploadIntent;
const wait = (milliseconds) =>
    new Promise((resolve) => setTimeout(resolve, milliseconds));

function wav(chunks, sampleRate) {
    const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const buffer = new ArrayBuffer(44 + length * 2);
    const view = new DataView(buffer);
    const text = (offset, value) =>
        [...value].forEach((c, i) =>
            view.setUint8(offset + i, c.charCodeAt(0)),
        );
    text(0, "RIFF");
    view.setUint32(4, 36 + length * 2, true);
    text(8, "WAVE");
    text(12, "fmt ");
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 1, true);
    view.setUint32(24, sampleRate, true);
    view.setUint32(28, sampleRate * 2, true);
    view.setUint16(32, 2, true);
    view.setUint16(34, 16, true);
    text(36, "data");
    view.setUint32(40, length * 2, true);
    let offset = 44;
    for (const chunk of chunks)
        for (const value of chunk) {
            view.setInt16(
                offset,
                Math.max(-32768, Math.min(32767, Math.round(value * 32767))),
                true,
            );
            offset += 2;
        }
    return new Blob([buffer], { type: "audio/wav" });
}

function download(blob, name) {
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = name;
    link.textContent = `Download ${name}`;
    byId("downloads").append(link, document.createElement("br"));
}

async function remoteAudio(stream) {
    if (stream.getAudioTracks().length) return;
    await new Promise((resolve, reject) => {
        const finish = () => {
            clearTimeout(timer);
            stream.removeEventListener("addtrack", added);
            resolve();
        };
        const added = () => {
            if (stream.getAudioTracks().length) finish();
        };
        const timer = setTimeout(() => {
            stream.removeEventListener("addtrack", added);
            reject(new Error("Remote audio track deadline exceeded"));
        }, 10000);
        stream.addEventListener("addtrack", added);
    });
}

byId("stop").onclick = () => {
    if (current) {
        current.stopped = true;
        void current.voice?.close();
    }
};
byId("upload").onclick = async () => {
    if (!captured || !byId("conversation").value) return;
    byId("upload").disabled = true;
    try {
        const client = new BrowserHttpClient(
            byId("http").value,
            byId("credential").value,
        );
        const bytes = new Uint8Array(await captured.arrayBuffer());
        const digest = new Uint8Array(
            await crypto.subtle.digest("SHA-256", bytes),
        );
        const sha256 = [...digest]
            .map((x) => x.toString(16).padStart(2, "0"))
            .join("");
        const id = (uploadIntent ??= crypto.randomUUID());
        const cid = byId("conversation").value;
        const artifact = await client.artifactBegin(
            id + "-begin",
            cid,
            "browser-received.wav",
            "audio/wav",
            bytes.length,
            sha256,
        );
        for (let offset = 0; offset < bytes.length; offset += 32768)
            await client.uploadArtifactChunk(
                cid,
                artifact.id,
                id + "-chunk-" + offset + "-32768",
                offset,
                bytes.slice(offset, offset + 32768),
            );
        await client.artifactFinish(id + "-finish", cid, artifact.id);
        byId("result").textContent +=
            "\n" +
            JSON.stringify({ recordingArtifact: artifact.id, sha256 }, null, 2);
        byId("status").textContent =
            "Recording stored · independent decoding still required.";
    } catch (error) {
        byId("status").textContent =
            `Recording upload failed: ${error.code ?? "failed"}. Same intent retained.`;
        byId("upload").disabled = false;
    }
};
byId("run").onclick = async () => {
    byId("run").disabled = true;
    byId("stop").disabled = false;
    byId("downloads").replaceChildren();
    byId("result").textContent = "";
    const context = new AudioContext();
    await context.resume();
    const state = (current = { stopped: false, voice: undefined });
    captured = undefined;
    uploadIntent = undefined;
    byId("upload").disabled = true;
    const chunks = [],
        report = {
            result: "running",
            browser: navigator.userAgent,
            turns: 0,
            turnStartSeconds: [],
            inputPeak: 0,
            inputSamples: 0,
        };
    const sources = [];
    let replyAudio = false;
    let recordingStarted;
    let recorder,
        monitor,
        remoteSource,
        capture = 0,
        deadline,
        inputStream;
    try {
        const scenarioUrl = new URL(byId("scenario").value, location.href);
        if (scenarioUrl.origin !== location.origin)
            throw new Error("Use same-origin speech fixtures");
        const scenario = await (await fetch(scenarioUrl)).json();
        if (
            !Array.isArray(scenario.turns) ||
            scenario.turns.length > 6 ||
            !scenario.turns.length
        )
            throw new Error("One to six turns required");
        const prompts = [];
        let total = 2;
        for (const turn of scenario.turns) {
            const url = new URL(turn.wav, scenarioUrl);
            if (url.origin !== location.origin)
                throw new Error("Use same-origin speech fixtures");
            const response = await fetch(url);
            if (!response.ok) throw new Error("Speech fixture fetch failed");
            const bytes = await response.arrayBuffer();
            if (bytes.byteLength > 2 * 1024 * 1024)
                throw new Error("Speech fixture exceeds bounds");
            const buffer = await context.decodeAudioData(bytes);
            const delay = turn.wait_seconds ?? 10;
            if (!Number.isFinite(delay) || delay < 0 || delay > 20)
                throw new Error(
                    "Wait per turn must be between zero and twenty seconds",
                );
            const waitForReply = turn.wait_for_reply ?? false;
            if (typeof waitForReply !== "boolean")
                throw new Error("wait_for_reply must be boolean");
            total += buffer.duration + delay + (waitForReply ? 12 : 0);
            prompts.push({ buffer, delay, waitForReply });
        }
        if (total > 90)
            throw new Error("Browser rehearsal is bounded by ninety seconds");
        await context.audioWorklet.addModule("capture-worklet.js");
        const destination = context.createMediaStreamDestination();
        destination.channelCount = 1;
        inputStream = destination.stream;
        monitor = new AudioWorkletNode(context, "connect-capture");
        const inputMute = context.createGain();
        inputMute.gain.value = 0;
        monitor.connect(inputMute).connect(context.destination);
        monitor.port.onmessage = (event) => {
            report.inputSamples += event.data.length;
            for (const value of event.data)
                report.inputPeak = Math.max(report.inputPeak, Math.abs(value));
        };
        state.voice = await BrowserVoiceSession.open(
            byId("signaling").value,
            byId("credential").value,
            15000,
            { inputStream },
        );
        state.voice.ended.then(() => {
            state.stopped = true;
        });
        await remoteAudio(state.voice.remoteStream);
        await state.voice.attach(byId("remote"));
        remoteSource = context.createMediaStreamSource(
            state.voice.remoteStream,
        );
        recorder = new AudioWorkletNode(context, "connect-capture");
        recorder.port.onmessage = (event) => {
            capture += event.data.length;
            if (event.data.some((value) => Math.abs(value) > 0.02))
                replyAudio = true;
            if (capture <= context.sampleRate * 92) chunks.push(event.data);
            else {
                state.stopped = true;
                void state.voice.close();
            }
        };
        const mute = context.createGain();
        mute.gain.value = 0;
        remoteSource
            .connect(recorder)
            .connect(mute)
            .connect(context.destination);
        deadline = setTimeout(() => {
            state.stopped = true;
            void state.voice.close();
        }, 95000);
        recordingStarted = performance.now();
        for (const { buffer, delay, waitForReply } of prompts) {
            if (state.stopped) throw new Error("Voice ended before all turns");
            byId("status").textContent =
                `Connected · speaking turn ${report.turns + 1}`;
            const source = context.createBufferSource();
            sources.push(source);
            source.buffer = buffer;
            source.connect(destination);
            source.connect(monitor);
            report.turnStartSeconds.push(
                (performance.now() - recordingStarted) / 1000,
            );
            source.start();
            report.audioContextState = context.state;
            report.path = await state.voice.diagnostics();
            await wait(buffer.duration * 1000);
            if (waitForReply) {
                replyAudio = false;
                const replyDeadline = performance.now() + 12000;
                while (!replyAudio) {
                    if (state.stopped || performance.now() >= replyDeadline)
                        throw new Error("Reply audio deadline exceeded");
                    await wait(20);
                }
            }
            await wait(delay * 1000);
            report.turns++;
        }
        report.path = await state.voice.diagnostics();
        report.result =
            chunks.length && report.turns === prompts.length
                ? "captured"
                : "failed";
    } catch (error) {
        report.result = "failed";
        report.error = error.code ?? error.message;
    } finally {
        clearTimeout(deadline);
        for (const source of sources) {
            try {
                source.stop();
            } catch {}
            source.disconnect();
        }
        recorder?.disconnect();
        monitor?.disconnect();
        remoteSource?.disconnect();
        await state.voice?.close();
        report.inputTracksAfterHangup = inputStream
            ?.getTracks()
            .map((track) => track.readyState);
        const element = byId("remote");
        element.pause();
        element.srcObject = null;
        await context.close();
        current = undefined;
        report.receivedSamples = capture;
        report.sampleRate = context.sampleRate;
        byId("result").textContent = JSON.stringify(report, null, 2);
        byId("status").textContent =
            report.result === "captured"
                ? "Captured · speech recognition still required."
                : "Failed · retained for review.";
        captured = wav(chunks, context.sampleRate);
        download(captured, "browser-received.wav");
        download(
            new Blob([JSON.stringify(report, null, 2)], {
                type: "application/json",
            }),
            "browser-result.json",
        );
        byId("run").disabled = false;
        byId("stop").disabled = true;
        byId("upload").disabled = !chunks.length;
    }
};
