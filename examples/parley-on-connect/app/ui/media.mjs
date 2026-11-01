import { BrowserOpusCodec, BrowserVp8Codec, RtpSender, Vp8Assembler,
    decodeRtpDatagram, browserCodecSupport } from "/sdk/browser.js";
import { MediaTimeline, rtpTimestamp, senderReport } from "./media-clock.mjs";
const fail = code => { throw Object.assign(Error(code), { code }); };
const base64 = bytes => btoa(String.fromCharCode(...bytes));
function unbase64(value) {
    if (typeof value !== "string" || value.length > 21864) fail("app-media-packet");
    return Uint8Array.from(atob(value), c => c.charCodeAt(0));
}
export class BrowserAppMedia {
    #stopped = false; #started = false; #abort = new AbortController(); #closing;
    #calls = new Set(); #loops = new Set(); #routes = new Map(); #sources = new Set();
    #context; #input; #worklet; #silent; #stream; #lease; #binding; #video; #videoCallback;
    #preview; #remote; #timer; #presentation; #audioOrigin; #timeline = new MediaTimeline();
    #cname = "parley-" + crypto.randomUUID(); #onState;
    #counts = { capturedAudioFrames: 0, capturedVideoFrames: 0, encodedFrames: 0,
        sentRtp: 0, receivedRtp: 0, clockReportsSent: 0, clockReportsReceived: 0,
        decodedAudioFrames: 0, playedAudioFrames: 0, decodedVideoFrames: 0, paintedVideoFrames: 0,
        droppedFrames: 0, synchronizedPackets: 0, unsynchronizedPackets: 0 };
    constructor({ onState = () => {} } = {}) { this.#onState = onState; }
    get binding() { return this.#binding ? { ...this.#binding } : null; }
    get closed() { return this.#stopped; }
    get diagnostics() {
        return { ...this.#counts, activeTracks: this.#stream?.getTracks().filter(t => t.readyState === "live").length ?? 0,
            activeContexts: this.#context && this.#context.state !== "closed" ? 1 : 0,
            queuedAudioSources: this.#sources.size,
            queuedSendFrames: [...this.#routes.values()].reduce((n, r) => n + r.queue.length, 0),
            queuedVideoFrames: [...this.#routes.values()].filter(r => r.pendingVideo).length,
            pendingRequests: this.#calls.size, stopped: this.#stopped };
    }
    #publish(state = this.#stopped ? "stopped" : "active") { this.#onState(state, this.diagnostics); }
    #check() { if (this.#stopped) fail("app-media-stopped"); }
    #call(path, input, cleanup = false) {
        const work = (async () => {
            if (!cleanup) this.#check();
            const signal = cleanup ? AbortSignal.timeout(2500) : AbortSignal.any([this.#abort.signal, AbortSignal.timeout(2500)]);
            const response = await fetch("/api/media/" + path, { method: "POST", credentials: "same-origin",
                redirect: "error", cache: "no-store", referrerPolicy: "no-referrer",
                headers: { "Content-Type": "application/json", "X-Parley-Request": "1" },
                body: JSON.stringify(input), signal, keepalive: cleanup });
            if (!response.body) fail("app-media-network");
            const reader = response.body.getReader(), blocks = []; let size = 0;
            try {
                while (true) {
                    const next = await reader.read(); if (next.done) break;
                    size += next.value.length;
                    if (size > 65536) { await reader.cancel(); fail("app-media-capacity"); }
                    blocks.push(next.value);
                }
            } finally { reader.releaseLock(); }
            const bytes = new Uint8Array(size); let offset = 0;
            for (const block of blocks) { bytes.set(block, offset); offset += block.length; }
            const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
            if (!cleanup) this.#check();
            if (!response.ok) fail(value.error?.code ?? "app-media-network");
            return value;
        })();
        if (!cleanup) { this.#calls.add(work); void work.finally(() => this.#calls.delete(work)).catch(() => {}); }
        return work;
    }
    #loop(work) {
        const owned = work.catch(error => {
            if (!this.#stopped) void this.stop(error.code ?? "app-media-network");
        }).finally(() => this.#loops.delete(owned));
        this.#loops.add(owned);
    }
    async #bound(promise, milliseconds = 8000) {
        let timer, cancelled;
        const interrupted = new Promise((_, reject) => {
            timer = setTimeout(() => reject(Object.assign(Error(), { code: "app-media-permission-timeout" })), milliseconds);
            cancelled = () => reject(Object.assign(Error(), { code: "app-media-stopped" }));
            this.#abort.signal.addEventListener("abort", cancelled, { once: true });
            if (this.#abort.signal.aborted) cancelled();
        });
        try { return await Promise.race([promise, interrupted]); }
        finally { clearTimeout(timer); this.#abort.signal.removeEventListener("abort", cancelled); }
    }
    async start(binding, { video = false, preview, remote } = {}) {
        if (this.#started) fail("app-media-stopped");
        this.#started = true; this.#binding = { ...binding }; this.#preview = preview; this.#remote = remote;
        this.#publish("starting");
        try {
            this.#check();
            if (!isSecureContext || typeof AudioContext !== "function" || typeof AudioWorkletNode !== "function")
                fail("unsupported-app-action");
            // Start playback within the explicit user's gesture, before awaiting
            // permissions, negotiation or codec support.
            this.#context = new AudioContext({ sampleRate: 48000, latencyHint: "interactive" });
            const resumed = this.#context.resume();
            if (this.#context.sampleRate !== 48000) fail("unsupported-app-action");
            await this.#bound(resumed);
            this.#check(); if (this.#context.state !== "running") fail("app-media-playback");
            this.#context.onstatechange = () => {
                if (!this.#stopped && this.#context.state !== "running") void this.stop("app-media-playback");
            };
            this.#audioOrigin = performance.now() / 1000 - this.#context.currentTime;
            const support = await this.#bound(browserCodecSupport());
            if (!support.audio || (video && !support.video)) fail("unsupported-app-action");
            this.#lease = await this.#call("open", this.#binding);
            this.#check(); this.#configure(this.#lease);
            const audioRoute = [...this.#routes.values()].find(r => r.binding.kind === "audio" && r.sender);
            const videoRoute = [...this.#routes.values()].find(r => r.binding.kind === "video" && r.sender);
            if (!audioRoute || (video && !videoRoute)) fail("unsupported-app-action");
            for (const name of ["microphone", ...(video ? ["camera"] : [])]) {
                let denied = false;
                try { denied = (await navigator.permissions.query({ name })).state === "denied"; } catch {}
                if (denied) fail("app-media-denied");
            }
            this.#check(); this.#publish("permission");
            const requested = navigator.mediaDevices.getUserMedia({
                audio: { sampleRate: { ideal: 48000 }, channelCount: { ideal: 1, max: 2 } },
                video: video ? { width: { ideal: 640, max: 640 }, height: { ideal: 360, max: 360 },
                    frameRate: { ideal: 30, max: 30 } } : false,
            });
            // A browser permission prompt is not cancellable. A late grant
            // remains owned here and is stopped before it can enter the graph.
            void requested.then(stream => {
                if (this.#stopped) for (const track of stream.getTracks()) track.stop();
            }, () => {});
            this.#stream = await this.#bound(requested);
            this.#check();
            if (this.#stream.getAudioTracks().length !== 1 || this.#stream.getVideoTracks().length !== (video ? 1 : 0) ||
                this.#stream.getTracks().some(t => t.readyState !== "live")) fail("unsupported-app-action");
            for (const track of this.#stream.getTracks())
                track.addEventListener("ended", () => { void this.stop("app-media-device-ended"); }, { once: true });
            await this.#bound(this.#context.audioWorklet.addModule("/audio-worklet.mjs"));
            this.#check();
            this.#input = this.#context.createMediaStreamSource(new MediaStream(this.#stream.getAudioTracks()));
            this.#worklet = new AudioWorkletNode(this.#context, "parley-capture", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1] });
            this.#silent = this.#context.createGain(); this.#silent.gain.value = 0;
            this.#input.connect(this.#worklet); this.#worklet.connect(this.#silent); this.#silent.connect(this.#context.destination);
            this.#worklet.onprocessorerror = () => { void this.stop("app-media-capture"); };
            this.#worklet.port.onmessage = ({ data }) => {
                if (this.#stopped) return;
                try {
                    if (!(data.samples instanceof Float32Array) || data.samples.length !== 960 ||
                        !Number.isSafeInteger(data.firstFrame) || data.firstFrame < 0) fail("app-media-capture");
                    this.#counts.capturedAudioFrames++;
                    if (!audioRoute.encoder.encode(data.samples, Math.round((this.#audioOrigin + data.firstFrame / 48000) * 1000000)))
                        this.#counts.droppedFrames++;
                    this.#worklet.port.postMessage("credit");
                } catch (error) { void this.stop(error.code ?? "app-media-capture"); }
            };
            if (video) await this.#camera(videoRoute);
            this.#check();
            for (const route of this.#routes.values())
                if (["recvonly", "sendrecv"].includes(route.binding.direction)) this.#loop(this.#receive(route));
            this.#timer = setInterval(() => this.#publish(), 500);
            this.#publish();
            return this;
        } catch (error) {
            const code = error?.name === "NotAllowedError" ? "app-media-denied" : error.code ?? "app-media-capture";
            await this.stop(code); throw Object.assign(Error(), { code });
        }
    }
    #configure(lease) {
        if (!lease.negotiation?.sourceClock || !Number.isSafeInteger(lease.generation) ||
            lease.negotiation.streams.length > 8) fail("unsupported-app-action");
        for (const stream of lease.negotiation.streams) {
            if (!["audio", "video"].includes(stream.medium) || stream.direction === "inactive") continue;
            const codec = stream.medium === "audio" ? "opus" : "vp8", rate = codec === "opus" ? 48000 : 90000;
            if (stream.codec?.name !== codec) fail("unsupported-app-action");
            const binding = { handle: stream.streamLocalId, streamId: stream.id, connectionId: this.#binding.connectionId,
                sessionId: this.#binding.sessionId, generation: lease.generation, kind: stream.medium, codec,
                payloadType: stream.codec.params?.payload_type, direction: stream.direction };
            const route = { binding, rate, queue: [], queuedBytes: 0, sending: false, lastReport: -Infinity,
                lastAudio: -Infinity, sources: new Set(), assembler: new Vp8Assembler() };
            this.#routes.set(binding.streamId, route);
            if (["sendonly", "sendrecv"].includes(binding.direction)) {
                route.sender = new RtpSender(binding);
                const callbacks = { encoded: frame => this.#enqueue(route, frame), decoded: () => {},
                    error: () => { void this.stop("app-media-codec"); } };
                route.encoder = codec === "opus" ? new BrowserOpusCodec(callbacks) : new BrowserVp8Codec(callbacks);
            }
            if (["recvonly", "sendrecv"].includes(binding.direction)) {
                this.#timeline.register(binding.streamId, rate);
                if (codec === "vp8") {
                    const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 360;
                    canvas.setAttribute("aria-label", "Received video " + (this.#remote.children.length + 1));
                    canvas.dataset.stream = binding.streamId;
                    this.#remote.append(canvas); route.canvas = canvas.getContext("2d");
                    if (!route.canvas) fail("unsupported-app-action");
                }
                this.#decoder(route);
            }
        }
    }
    #decoder(route) {
        route.decoder?.close();
        const callbacks = { encoded: () => {}, decoded: frame => {
            if (this.#stopped) return;
            try {
                if (route.binding.kind === "audio") this.#playAudio(route, frame);
                else this.#paintVideo(route, frame);
            } catch { void this.stop("app-media-playback"); }
        }, error: () => { void this.stop("app-media-codec"); } };
        route.decoder = route.binding.kind === "audio" ? new BrowserOpusCodec(callbacks) : new BrowserVp8Codec(callbacks);
    }
    #enqueue(route, frame) {
        if (this.#stopped) return;
        const capacity = route.binding.kind === "audio" ? 10 : 2;
        if (route.queue.length >= capacity || route.queuedBytes + frame.bytes.length > 76032) {
            this.#counts.droppedFrames++; route.encoder.requestKeyFrame?.(); return;
        }
        const captured = { bytes: frame.bytes.slice(), timestampUs: frame.timestampUs, queuedAt: performance.now() };
        route.queue.push(captured); route.queuedBytes += captured.bytes.length; this.#counts.encodedFrames++;
        if (!route.sending) this.#loop(this.#send(route));
    }
    async #send(route) {
        route.sending = true;
        try {
            while (!this.#stopped && route.queue.length) {
                const frame = route.queue.shift(); route.queuedBytes -= frame.bytes.length;
                if (performance.now() - frame.queuedAt >= 200) { this.#counts.droppedFrames++; route.encoder.requestKeyFrame?.(); continue; }
                const packets = route.sender.packetize(frame.bytes, rtpTimestamp(frame.timestampUs, route.rate));
                let stats;
                for (const datagram of packets) {
                    this.#check();
                    if (performance.now() - frame.queuedAt >= 200) {
                        this.#counts.droppedFrames++; route.encoder.requestKeyFrame?.(); break;
                    }
                    const parsed = decodeRtpDatagram(datagram);
                    const result = await this.#call("send", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId,
                        sequence: parsed.datagramSequence, packet: { kind: "rtp", base64: base64(datagram.subarray(8)) } });
                    if (result.boundary !== "local-media-queue" || result.datagramSequence !== parsed.datagramSequence) fail("app-media-packet");
                    this.#counts.sentRtp++; stats = route.sender.submitted(datagram);
                }
                // Allocate the SR sequence after this frame's RTP fragments so
                // both RTP and clock writes preserve their original order.
                if (stats && performance.now() - route.lastReport >= 1000) {
                    const sequence = route.sender.nextDatagramSequence();
                    const clock = await this.#call("send", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId, sequence,
                        packet: { kind: "clock", report: senderReport(frame.timestampUs, route.rate, stats, this.#cname) } });
                    if (clock.boundary !== "local-media-queue" || clock.datagramSequence !== sequence) fail("app-media-packet");
                    this.#counts.clockReportsSent++; route.lastReport = performance.now();
                }
            }
        } finally { route.sending = false; }
    }
    async #receive(route) {
        while (!this.#stopped) {
            const packet = await this.#call("receive", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId });
            this.#check();
            if (packet.kind === "idle") continue;
            if (packet.kind === "gap") { route.assembler.clear(); this.#counts.droppedFrames++; continue; }
            if (packet.kind === "clock") {
                if (this.#timeline.report(route.binding.streamId, packet.report)) this.#counts.clockReportsReceived++;
                continue;
            }
            if (packet.kind !== "rtp") fail("app-media-packet");
            if (!Number.isInteger(packet.sequence) || packet.sequence < 0 || packet.sequence > 0xffffffff) fail("app-media-packet");
            const raw = unbase64(packet.base64), wire = new Uint8Array(raw.length + 8), header = new DataView(wire.buffer);
            wire[0] = 1; header.setUint16(2, route.binding.handle); header.setUint32(4, packet.sequence); wire.set(raw, 8);
            const parsed = decodeRtpDatagram(wire);
            if (parsed.payloadType !== route.binding.payloadType) fail("app-media-packet");
            this.#counts.receivedRtp++;
            const timing = this.#timeline.timestamp(route.binding.streamId, parsed.ssrc, parsed.timestamp);
            if (timing.synchronized) this.#counts.synchronizedPackets++; else this.#counts.unsynchronizedPackets++;
            if (timing.changed) {
                this.#decoder(route); route.assembler.clear(); route.lastAudio = -Infinity;
                for (const source of route.sources) { try { source.stop(); } catch {} source.disconnect(); this.#sources.delete(source); }
                route.sources.clear(); route.pendingVideo?.frame.close(); route.pendingVideo = null; clearTimeout(route.paintTimer);
            }
            if (route.binding.kind === "audio") {
                if (timing.timestampUs <= route.lastAudio) { this.#counts.droppedFrames++; continue; }
                route.lastAudio = timing.timestampUs;
                if (!route.decoder.decode(parsed.payload, timing.timestampUs)) this.#counts.droppedFrames++;
            } else {
                const frame = route.assembler.push(parsed);
                if (frame && !route.decoder.decode(frame, timing.timestampUs)) this.#counts.droppedFrames++;
            }
        }
    }
    #desired(timestampUs) {
        this.#presentation ??= { source: timestampUs, local: this.#context.currentTime + 0.08 };
        return this.#presentation.local + (timestampUs - this.#presentation.source) / 1000000;
    }
    #playAudio(route, frame) {
        this.#counts.decodedAudioFrames++;
        const now = this.#context.currentTime, desired = this.#desired(frame.timestamp);
        if (this.#context.state !== "running" || this.#sources.size >= 10 || desired < now - 1 / 48000 || desired + 0.02 > now + 0.2) {
            this.#counts.droppedFrames++; return;
        }
        const samples = new Float32Array(960);
        if (frame.numberOfFrames !== 960 || frame.sampleRate !== 48000 || frame.numberOfChannels !== 1) fail("app-media-codec");
        frame.copyTo(samples, { planeIndex: 0, format: "f32-planar" });
        const buffer = this.#context.createBuffer(1, 960, 48000); buffer.copyToChannel(samples, 0);
        const source = this.#context.createBufferSource(); source.buffer = buffer; source.connect(this.#context.destination);
        this.#sources.add(source); route.sources.add(source);
        source.onended = () => { source.onended = null; source.disconnect(); this.#sources.delete(source); route.sources.delete(source); };
        try { source.start(Math.max(now, desired)); this.#counts.playedAudioFrames++; }
        catch (error) { source.onended = null; source.disconnect(); this.#sources.delete(source); route.sources.delete(source); throw error; }
    }
    #paintVideo(route, frame) {
        this.#counts.decodedVideoFrames++;
        const now = this.#context.currentTime, desired = this.#desired(frame.timestamp);
        if (desired < now - 0.02 || desired > now + 0.2) { this.#counts.droppedFrames++; return; }
        if (route.pendingVideo) { route.pendingVideo.frame.close(); this.#counts.droppedFrames++; }
        clearTimeout(route.paintTimer); const owned = { frame: frame.clone(), desired }; route.pendingVideo = owned;
        route.paintTimer = setTimeout(() => {
            if (route.pendingVideo !== owned) return;
            route.pendingVideo = null;
            try {
                if (!this.#stopped && this.#context.currentTime - owned.desired < 0.05) {
                    route.canvas.drawImage(owned.frame, 0, 0, 640, 360); this.#counts.paintedVideoFrames++;
                } else this.#counts.droppedFrames++;
            } finally { owned.frame.close(); }
        }, Math.max(0, (desired - now) * 1000));
    }
    async #camera(route) {
        this.#video = this.#preview; this.#video.muted = true; this.#video.playsInline = true;
        this.#video.srcObject = new MediaStream(this.#stream.getVideoTracks());
        await this.#bound(this.#video.play());
        this.#check(); const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 360;
        const out = canvas.getContext("2d"); if (!out || typeof this.#video.requestVideoFrameCallback !== "function") fail("unsupported-app-action");
        let origin;
        const frame = (_now, metadata) => {
            if (this.#stopped) return;
            try {
                const mediaUs = Math.round(metadata.mediaTime * 1000000), presentationUs = Math.round(metadata.presentationTime * 1000);
                if (!Number.isSafeInteger(mediaUs) || !Number.isSafeInteger(presentationUs) || mediaUs < 0 || presentationUs < 0)
                    fail("app-media-clock");
                origin ??= presentationUs - mediaUs;
                const timestamp = metadata.captureTime === undefined ? origin + mediaUs : Math.round(metadata.captureTime * 1000);
                out.drawImage(this.#video, 0, 0, 640, 360);
                const image = new VideoFrame(canvas, { timestamp });
                try { this.#counts.capturedVideoFrames++; if (!route.encoder.encode(image)) this.#counts.droppedFrames++; }
                finally { image.close(); }
            } catch { void this.stop("app-media-capture"); return; }
            this.#videoCallback = this.#video.requestVideoFrameCallback(frame);
        };
        this.#videoCallback = this.#video.requestVideoFrameCallback(frame);
    }
    stop(reason = "stopped") {
        if (this.#closing) return this.#closing;
        this.#stopped = true; this.#abort.abort(); clearInterval(this.#timer);
        if (this.#videoCallback !== undefined) this.#video?.cancelVideoFrameCallback(this.#videoCallback);
        this.#video?.pause(); if (this.#preview) this.#preview.srcObject = null;
        if (this.#stream) for (const track of this.#stream.getTracks()) track.stop();
        if (this.#worklet) { this.#worklet.port.postMessage("stop"); this.#worklet.port.onmessage = null; this.#worklet.port.close(); this.#worklet.disconnect(); }
        this.#input?.disconnect(); this.#silent?.disconnect();
        for (const source of this.#sources) { source.onended = null; try { source.stop(); } catch {} source.disconnect(); }
        this.#sources.clear();
        for (const route of this.#routes.values()) {
            route.encoder?.close(); route.decoder?.close(); route.assembler.clear(); route.sources.clear();
            route.queue.length = 0; route.queuedBytes = 0; clearTimeout(route.paintTimer);
            route.pendingVideo?.frame.close(); route.pendingVideo = null;
            if (route.canvas) { route.canvas.canvas.width = 0; route.canvas.canvas.height = 0; }
        }
        this.#routes.clear(); this.#timeline.clear(); this.#remote?.replaceChildren();
        this.#closing = (async () => {
            const retired = this.#lease ? this.#call("close", { leaseId: this.#lease.leaseId }, true) : Promise.resolve();
            await Promise.allSettled([...this.#calls, ...this.#loops, retired,
                this.#context?.state !== "closed" ? this.#context?.close() : Promise.resolve()]);
            this.#publish(reason);
        })();
        return this.#closing;
    }
}
