import { BrowserOpusCodec, BrowserVp8Codec, RtpSender, Vp8Assembler,
    decodeRtpDatagram, browserCodecSupport } from "/sdk/browser.js";
import { MediaTimeline, CaptureClock, playoutTime } from "./media-clock.mjs";
import { VideoPlayout } from "./media-playout.mjs";
import { takeMediaSendBatch } from "./media-send.mjs";
const fail = code => { throw Object.assign(Error(code), { code }); };
const base64 = bytes => btoa(String.fromCharCode(...bytes));
function unbase64(value) {
    if (typeof value !== "string" || value.length > 1576) fail("app-media-packet");
    return Uint8Array.from(atob(value), c => c.charCodeAt(0));
}
export class BrowserAppMedia {
    #stopped = false; #started = false; #abort = new AbortController(); #closing;
    #calls = new Set(); #loops = new Set(); #routes = new Map(); #sources = new Set();
    #startupAt = 0; #context; #input; #worklet; #silent; #stream; #lease; #binding; #video; #videoCallback;
    #preview; #remote; #timer; #audioOrigin; #timeline = new MediaTimeline();
    #phase = "idle"; #stoppedAt = null; #stopReason = null;
    #cname = "parley-" + crypto.randomUUID(); #onState;
    #counts = { capturedAudioFrames: 0, capturedVideoFrames: 0, encodedFrames: 0,
        sentRtp: 0, receivedRtp: 0, clockReportsSent: 0, clockReportsReceived: 0,
        videoClockReportsSent: 0, videoClockReportsReceived: 0,
        decodedAudioFrames: 0, playedAudioFrames: 0, decodedVideoFrames: 0, paintedVideoFrames: 0,
        droppedFrames: 0, synchronizedPackets: 0, unsynchronizedPackets: 0,
        audioLateDrops: 0, audioHorizonDrops: 0, audioCapacityDrops: 0,
        videoLateDrops: 0, videoHorizonDrops: 0, videoTimerDrops: 0, videoCapacityDrops: 0, droppedClockReports: 0,
        lastAudioOffsetMs: null, lastVideoOffsetMs: null, receiveClockCorrections: 0,
        lastAudioCaptureOffsetMs: null, lastVideoCaptureOffsetMs: null,
        receiveStartedMs: null, deviceSetupMs: null, captureGraphMs: null, cameraSetupMs: null };
    constructor({ onState = () => {} } = {}) { this.#onState = onState; }
    get binding() { return this.#binding ? { ...this.#binding } : null; }
    get closed() { return this.#stopped; }
    get diagnostics() {
        return { ...this.#counts, phase: this.#phase, stoppedAt: this.#stoppedAt, stopReason: this.#stopReason, activeTracks: this.#stream?.getTracks().filter(t => t.readyState === "live").length ?? 0,
            activeContexts: this.#context && this.#context.state !== "closed" ? 1 : 0,
            queuedAudioSources: this.#sources.size,
            queuedSendFrames: [...this.#routes.values()].reduce((n, r) => n + r.queue.length + r.pendingFrames, 0),
            queuedVideoFrames: [...this.#routes.values()].reduce((n, r) => n + (r.videoPlayout?.size ?? 0), 0),
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
            if (!response.ok) fail(value.error?.code === "offline-or-uncertain" ? "app-media-network" : value.error?.code ?? "app-media-network");
            return value;
        })();
        if (!cleanup) { this.#calls.add(work); void work.finally(() => this.#calls.delete(work)).catch(() => {}); }
        return work;
    }
    #loop(work) {
        const owned = work.catch(error => {
            if (!this.#stopped) void this.stop(typeof error.code === "string" ? error.code : "app-media-network");
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
        this.#started = true; this.#startupAt = performance.now(); this.#binding = { ...binding }; this.#preview = preview; this.#remote = remote;
        this.#publish("starting");
        try {
            this.#check();
            if (!isSecureContext || typeof AudioContext !== "function" || typeof AudioWorkletNode !== "function")
                fail("unsupported-app-action");
            // Start playback within the explicit user's gesture, before awaiting
            // permissions, negotiation or codec support.
            this.#phase = "playback-start";
            this.#context = new AudioContext({ sampleRate: 48000, latencyHint: "interactive" });
            const resumed = this.#context.resume();
            if (this.#context.sampleRate !== 48000) fail("unsupported-app-action");
            await this.#bound(resumed);
            this.#check(); if (this.#context.state !== "running") fail("app-media-playback");
            this.#context.onstatechange = () => {
                if (!this.#stopped && this.#context.state !== "running") void this.stop("app-media-playback");
            };
            this.#audioOrigin = performance.now() / 1000 - this.#context.currentTime;
            this.#phase = "codec-support";
            const support = await this.#bound(browserCodecSupport());
            if (!support.audio || (video && !support.video)) fail("unsupported-app-action");
            this.#phase = "native-media-open";
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
            this.#phase = "device-permission";
            this.#check(); this.#publish("permission");
            const deviceStarted = performance.now();
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
            this.#counts.deviceSetupMs = Math.round(performance.now() - deviceStarted);
            this.#check();
            if (this.#stream.getAudioTracks().length !== 1 || this.#stream.getVideoTracks().length !== (video ? 1 : 0) ||
                this.#stream.getTracks().some(t => t.readyState !== "live")) fail("unsupported-app-action");
            for (const track of this.#stream.getTracks())
                track.addEventListener("ended", () => { void this.stop("app-media-device-ended"); }, { once: true });
            // Incoming media is independent of local capture initialization.
            // After permission/track validation, drain its original current
            // packets while the worklet and camera preview become ready.
            for (const route of this.#routes.values())
                if (["recvonly", "sendrecv"].includes(route.binding.direction)) this.#loop(this.#receive(route));
            this.#counts.receiveStartedMs = Math.round(performance.now() - this.#startupAt);
            const graphStarted = performance.now();
            this.#phase = "capture-graph";
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
                    const timestamp = Math.round((this.#audioOrigin + data.firstFrame / 48000) * 1000000);
                    this.#counts.lastAudioCaptureOffsetMs = Math.round(timestamp / 1000 - performance.now());
                    if (audioRoute.encoder.encode(data.samples, timestamp)) audioRoute.captureClock.observe(timestamp);
                    else this.#counts.droppedFrames++;
                    this.#worklet.port.postMessage("credit");
                } catch (error) { void this.stop(error.code ?? "app-media-capture"); }
            };
            this.#counts.captureGraphMs = Math.round(performance.now() - graphStarted);
            if (video) {
                this.#phase = "camera-capture"; const cameraStarted = performance.now();
                await this.#camera(videoRoute);
                this.#counts.cameraSetupMs = Math.round(performance.now() - cameraStarted);
            }
            this.#check();
            this.#phase = "active";
            this.#timer = setInterval(() => this.#publish(), 500);
            this.#publish();
            return this;
        } catch (error) {
            const code = error?.name === "NotAllowedError" ? "app-media-denied" :
                typeof error.code === "string" ? error.code : "app-media-capture";
            await this.stop(code); throw Object.assign(Error(), { code });
        }
    }
    #configure(lease) {
        if (!lease.negotiation?.sourceClock || !Number.isSafeInteger(lease.generation) ||
            lease.negotiation.streams.length > 8) fail("unsupported-app-action");
        for (const stream of lease.negotiation.streams) {
            if (!["audio", "video"].includes(stream.medium) || stream.direction === "inactive") continue;
            if (!Number.isInteger(stream.streamLocalId) || stream.streamLocalId < 1 || stream.streamLocalId > 65535) continue;
            if (!Number.isInteger(stream.codec?.params?.payload_type) || stream.codec.params.payload_type < 0 ||
                stream.codec.params.payload_type > 127) fail("unsupported-app-action");
            const codec = stream.medium === "audio" ? "opus" : "vp8", rate = codec === "opus" ? 48000 : 90000;
            if (stream.codec?.name !== codec) fail("unsupported-app-action");
            const binding = { handle: stream.streamLocalId, streamId: stream.id, connectionId: this.#binding.connectionId,
                sessionId: this.#binding.sessionId, generation: lease.generation, kind: stream.medium, codec,
                payloadType: stream.codec.params?.payload_type, direction: stream.direction };
            const route = { binding, rate, queue: [], queuedBytes: 0, pendingFrames: 0, sending: false, lastReport: -Infinity, captureClock: new CaptureClock(),
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
                    route.videoPlayout = new VideoPlayout({ now: () => this.#context.currentTime,
                        paint: frame => {
                            if (this.#stopped) return;
                            try { route.canvas.drawImage(frame, 0, 0, 640, 360); this.#counts.paintedVideoFrames++; }
                            catch { void this.stop("app-media-playback"); }
                        }, drop: reason => {
                            this.#counts.droppedFrames++;
                            if (reason === "capacity") this.#counts.videoCapacityDrops++;
                            else this.#counts.videoTimerDrops++;
                        } });
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
        if (route.queue.length + route.pendingFrames >= capacity || route.queuedBytes + frame.bytes.length > 76032) {
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
                // Amortize HTTP round trips over frames that are already queued.
                // No fill timer, new source origin or additional frame queue.
                const batch = takeMediaSendBatch(route, () => performance.now(), () => {
                    this.#counts.droppedFrames++; route.encoder.requestKeyFrame?.();
                });
                const datagrams = batch.packets.map(v => v.datagram), packets = batch.packets.map(({ datagram, remainingMs }) => ({
                    sequence: decodeRtpDatagram(datagram).datagramSequence, remainingMs,
                    packet: { kind: "rtp", base64: base64(datagram.subarray(8)) },
                }));
                if (!packets.length) continue;
                let result;
                try { result = await this.#call("send", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId, packets }); }
                finally { route.pendingFrames = 0; }
                if (!Array.isArray(result.submissions) || result.submissions.length !== packets.length) fail("app-media-packet");
                let stats;
                for (let i = 0; i < packets.length; i++) {
                    const receipt = result.submissions[i];
                    if (receipt.datagramSequence !== packets[i].sequence) fail("app-media-packet");
                    if (receipt.boundary === "expired") {
                        this.#counts.droppedFrames++; route.encoder.requestKeyFrame?.(); continue;
                    }
                    if (receipt.boundary !== "local-media-queue") fail("app-media-packet");
                    this.#counts.sentRtp++; stats = route.sender.submitted(datagrams[i]);
                }
                // Allocate the SR sequence after the current frame's fragments so
                // both RTP and clock writes preserve their original order.
                if (stats && batch.frameComplete && performance.now() - route.lastReport >= 1000) {
                    const sequence = route.sender.nextDatagramSequence();
                    const report = route.captureClock.sample(route.rate, stats, this.#cname);
                    if (!report) fail("app-media-clock");
                    const clock = await this.#call("send", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId,
                        packets: [{ sequence, remainingMs: 200, packet: { kind: "clock", report } }] });
                    if (clock.submissions?.length !== 1 || clock.submissions[0].datagramSequence !== sequence)
                        fail("app-media-packet");
                    if (clock.submissions[0].boundary === "expired") this.#counts.droppedClockReports++;
                    else {
                        if (clock.submissions[0].boundary !== "local-media-queue") fail("app-media-packet");
                        this.#counts.clockReportsSent++;
                        if (route.binding.kind === "video") this.#counts.videoClockReportsSent++;
                        route.lastReport = performance.now();
                    }
                }
            }
        } finally { route.sending = false; }
    }
    async #receive(route) {
        while (!this.#stopped) {
            const batch = await this.#call("receive", { leaseId: this.#lease.leaseId, streamId: route.binding.streamId });
            this.#check();
            if (!Array.isArray(batch.packets) || batch.packets.length > 8) fail("app-media-packet");
            for (const packet of batch.packets) this.#receivePacket(route, packet);
        }
    }
    #receivePacket(route, packet) {
        this.#check();
        if (packet.kind === "gap") { route.assembler.clear(); this.#counts.droppedFrames++; return; }
        if (packet.kind === "clock") {
            if (this.#timeline.report(route.binding.streamId, packet.report)) {
                this.#counts.clockReportsReceived++;
                if (route.binding.kind === "video") this.#counts.videoClockReportsReceived++;
            }
            return;
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
        if (timing.changed || timing.clockChanged) {
            // Pending decoder output belongs to the old clock epoch too.
            this.#decoder(route); route.assembler.clear();
            if (timing.clockChanged) this.#counts.receiveClockCorrections++;
            route.lastAudio = -Infinity;
            for (const source of route.sources) { try { source.stop(); } catch {} source.disconnect(); this.#sources.delete(source); }
            route.sources.clear(); route.videoPlayout?.clear();
        }
        if (route.binding.kind === "audio") {
            if (timing.timestampUs <= route.lastAudio) { this.#counts.droppedFrames++; return; }
            route.lastAudio = timing.timestampUs;
            if (!route.decoder.decode(parsed.payload, timing.timestampUs)) this.#counts.droppedFrames++;
        } else {
            const frame = route.assembler.push(parsed);
            if (frame && !route.decoder.decode(frame, timing.timestampUs)) this.#counts.droppedFrames++;
        }
    }
    #desired(timestampUs) {
        return playoutTime(timestampUs, Math.round(performance.now() * 1000), this.#context.currentTime);
    }
    #playAudio(route, frame) {
        this.#counts.decodedAudioFrames++;
        const now = this.#context.currentTime, desired = this.#desired(frame.timestamp);
        this.#counts.lastAudioOffsetMs = Math.round((desired - now) * 1000000) / 1000;
        if (this.#context.state !== "running" || this.#sources.size >= 10 || desired < now - 1 / 48000 || desired + 0.02 > now + 0.2) {
            if (this.#sources.size >= 10) this.#counts.audioCapacityDrops++;
            if (desired < now - 1 / 48000) this.#counts.audioLateDrops++;
            if (desired + 0.02 > now + 0.2) this.#counts.audioHorizonDrops++;
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
        this.#counts.lastVideoOffsetMs = Math.round((desired - now) * 1000000) / 1000;
        if (desired < now - 0.02 || desired > now + 0.2) {
            if (desired < now - 0.02) this.#counts.videoLateDrops++;
            if (desired > now + 0.2) this.#counts.videoHorizonDrops++;
            this.#counts.droppedFrames++; return;
        }
        route.videoPlayout.enqueue(frame, desired);
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
                this.#counts.lastVideoCaptureOffsetMs = Math.round(timestamp / 1000 - performance.now());
                out.drawImage(this.#video, 0, 0, 640, 360);
                const image = new VideoFrame(canvas, { timestamp });
                try {
                    this.#counts.capturedVideoFrames++;
                    if (route.encoder.encode(image)) route.captureClock.observe(timestamp);
                    else this.#counts.droppedFrames++;
                }
                finally { image.close(); }
            } catch { void this.stop("app-media-capture"); return; }
            this.#videoCallback = this.#video.requestVideoFrameCallback(frame);
        };
        this.#videoCallback = this.#video.requestVideoFrameCallback(frame);
    }
    stop(reason = "stopped") {
        if (this.#closing) return this.#closing;
        this.#stoppedAt = this.#phase; this.#phase = "stopped"; this.#stopReason = reason;
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
            route.queue.length = 0; route.queuedBytes = 0; route.pendingFrames = 0; route.videoPlayout?.close();
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
