import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import { randomUUID } from "node:crypto";
import { MediaTimeline, CaptureClock, rtpTimestamp, playoutTime } from "./ui/media-clock.mjs";
import { VideoPlayout } from "./ui/media-playout.mjs";

const source = await readFile(new URL("./ui/media.mjs", import.meta.url), "utf8");
const body = source.slice(source.indexOf("const fail =")).replace("export class BrowserAppMedia", "class BrowserAppMedia");
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
function fixture({ denied = false } = {}) {
    const camera = deferred(), cameraEntered = deferred();
    const counts = { receive: 0, activeReceive: 0, cancelledReceive: 0, leaseClose: 0, contextClose: 0, codecClose: 0, cameraCallbacks: 0 };
    const tracks = ["audio", "video"].map(kind => ({ kind, readyState: "live", addEventListener() {}, stop() { this.readyState = "ended"; } }));
    class Stream {
        constructor(selected = tracks) { this.tracks = selected; }
        getTracks() { return this.tracks; }
        getAudioTracks() { return this.tracks.filter(t => t.kind === "audio"); }
        getVideoTracks() { return this.tracks.filter(t => t.kind === "video"); }
    }
    const node = () => ({ connect() {}, disconnect() {} });
    class Context {
        state = "running"; sampleRate = 48000; currentTime = 0; destination = {};
        audioWorklet = { addModule: async () => {} };
        resume() { return Promise.resolve(); }
        createMediaStreamSource() { return node(); }
        createGain() { return { ...node(), gain: { value: 0 } }; }
        async close() { counts.contextClose++; this.state = "closed"; }
    }
    class Worklet { port = { postMessage() {}, close() {} }; connect() {} disconnect() {} }
    class Codec { constructor(callbacks) { this.callbacks = callbacks; } close() { counts.codecClose++; } }
    class Assembler { clear() {} }
    const binding = { conversationId: "conversation", sessionId: "session", connectionId: "connection" };
    const streams = [{ id: "audio", medium: "audio", direction: "sendrecv", streamLocalId: 1,
        codec: { name: "opus", params: { payload_type: 111 } } },
    { id: "video", medium: "video", direction: "sendonly", streamLocalId: 2,
        codec: { name: "vp8", params: { payload_type: 96 } } }];
    const fetch = async (url, options) => {
        const action = url.slice(url.lastIndexOf("/") + 1);
        if (action === "open") return new Response(JSON.stringify({ leaseId: "lease", generation: 1,
            negotiation: { binding, sourceClock: true, streams } }));
        if (action === "close") { counts.leaseClose++; return new Response("{}"); }
        assert.equal(action, "receive");
        counts.receive++; counts.activeReceive++;
        await new Promise((_, reject) => {
            const abort = () => { counts.activeReceive--; counts.cancelledReceive++; reject(new DOMException("Stopped", "AbortError")); };
            if (options.signal.aborted) abort(); else options.signal.addEventListener("abort", abort, { once: true });
        });
    };
    const preview = { pause() {}, play() { cameraEntered.resolve(); return camera.promise; },
        requestVideoFrameCallback() { counts.cameraCallbacks++; return 1; }, cancelVideoFrameCallback() {} };
    const sandbox = { performance, TextDecoder, TextEncoder, crypto: { randomUUID }, AbortController, AbortSignal, setTimeout, clearTimeout, setInterval, clearInterval,
        isSecureContext: true, AudioContext: Context, AudioWorkletNode: Worklet, MediaStream: Stream,
        navigator: { permissions: { query: async () => ({ state: "granted" }) }, mediaDevices: { getUserMedia: async () => {
            if (denied) throw Object.assign(Error("Denied"), { name: "NotAllowedError" }); return new Stream();
        } } }, fetch, BrowserOpusCodec: Codec, BrowserVp8Codec: Codec, RtpSender: class {}, Vp8Assembler: Assembler,
        decodeRtpDatagram() { throw Error("No synthetic RTP supplied"); }, browserCodecSupport: async () => ({ audio: true, video: true }),
        MediaTimeline, CaptureClock, rtpTimestamp, playoutTime, VideoPlayout };
    vm.runInNewContext(body + "\nglobalThis.Media = BrowserAppMedia;", sandbox, { filename: "ui/media.mjs" });
    const media = new sandbox.Media();
    return { media, counts, camera, cameraEntered, tracks, binding, preview,
        start: () => media.start(binding, { video: true, preview, remote: { children: [], replaceChildren() {} } }) };
}

test("incoming audio reads do not wait for the local camera preview", async () => {
    const f = fixture(), starting = f.start(); void starting.catch(() => {});
    try {
        await Promise.race([f.cameraEntered.promise, starting.then(() => { throw Error("Startup completed before held camera"); })]);
        assert.equal(f.media.diagnostics.phase, "camera-capture");
        assert.equal(f.counts.receive, 1, "a held local camera must not build an incoming audio backlog");
        assert.equal(f.counts.activeReceive, 1);
    } finally {
        await f.media.stop(); await assert.rejects(starting, { code: "app-media-stopped" }); f.camera.resolve();
    }
});

test("denied device permission starts no incoming IO and closes the admitted lease", async () => {
    const f = fixture({ denied: true });
    await assert.rejects(f.start(), { code: "app-media-denied" });
    assert.equal(f.counts.receive, 0); assert.equal(f.counts.leaseClose, 1); assert.equal(f.counts.contextClose, 1);
    assert.equal(f.media.diagnostics.pendingRequests, 0); assert.equal(f.media.diagnostics.stopped, true);
});

test("stopping held camera startup joins the original incoming read and discards late preview readiness", async () => {
    const f = fixture(), starting = f.start(); void starting.catch(() => {});
    await Promise.race([f.cameraEntered.promise, starting.then(() => { throw Error("Startup completed before held camera"); })]);
    try { assert.equal(f.counts.activeReceive, 1); }
    finally { await f.media.stop(); await assert.rejects(starting, { code: "app-media-stopped" }); f.camera.resolve(); }
    await Promise.resolve();
    assert.equal(f.counts.activeReceive, 0); assert.equal(f.counts.cancelledReceive, 1); assert.equal(f.counts.leaseClose, 1);
    assert.equal(f.counts.contextClose, 1); assert.equal(f.counts.codecClose, 3); assert.equal(f.counts.cameraCallbacks, 0);
    assert.ok(f.tracks.every(t => t.readyState === "ended"));
    assert.equal(f.media.diagnostics.pendingRequests, 0); assert.equal(f.media.diagnostics.queuedAudioSources, 0);
});
