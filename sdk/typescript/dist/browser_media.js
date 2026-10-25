import { BrowserOpusCodec, BrowserVp8Codec, browserCodecSupport, RtpClock, } from "./browser_codecs.js";
import { MediaBindings, MediaSendQueue, RtpSender, Vp8Assembler, decodeRtpDatagram, MEDIA_LIMITS, } from "./browser_rtp.js";
import { ConnectError } from "./errors.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
const fail = (message) => {
    throw new ConnectError("protocol", message);
};
const invert = (value) => {
    if (value === "sendonly")
        return "recvonly";
    if (value === "recvonly")
        return "sendonly";
    if (value === "sendrecv")
        return "sendrecv";
    return fail("Invalid negotiated media direction");
};
/** One datagram owner per authenticated physical client; protocol/media state has no UI. */
export class BrowserMediaPeer {
    client;
    callbacks;
    reader;
    writer;
    registry = new MediaBindings();
    routes = new Map();
    connections = new Map();
    transitions = new Map();
    queue = new MediaSendQueue();
    stopEvents;
    stopLifetime;
    timer;
    stopped = false;
    sending = false;
    mtu;
    counters = {
        sent: 0,
        received: 0,
        rejected: 0,
        decodedAudio: 0,
        decodedVideo: 0,
    };
    constructor(client, callbacks = {}) {
        this.client = client;
        this.callbacks = callbacks;
        const datagrams = client.datagrams;
        if (datagrams.readable.locked ||
            datagrams.writable.locked ||
            !(datagrams.maxDatagramSize >= 64))
            throw new ConnectError("unsupported-media-profile", "Media requires exclusively owned negotiated datagrams");
        this.mtu = Math.min(datagrams.maxDatagramSize, MEDIA_LIMITS.rtpBytes + 8);
        datagrams.incomingMaxAge = datagrams.outgoingMaxAge = 200;
        datagrams.incomingHighWaterMark = datagrams.outgoingHighWaterMark = 64;
        this.reader = datagrams.readable.getReader();
        this.writer = datagrams.writable.getWriter();
        this.stopEvents = client.onEvent((frame) => this.event(frame));
        this.stopLifetime = client.onClose(() => this.close());
        this.timer = setInterval(() => {
            for (const route of this.routes.values()) {
                const expired = route.assembler?.expire() ?? 0;
                route.observed.expired += expired;
                if (expired)
                    route.vp8?.lost();
            }
        }, 50);
        void this.receive().catch((error) => this.die(error));
    }
    get diagnostics() {
        return {
            ...this.counters,
            queuedPackets: this.queue.queuedPackets,
            queuedBytes: this.queue.queuedBytes,
            droppedPackets: this.queue.droppedPackets,
            activeBindings: this.routes.size,
        };
    }
    get routeDiagnostics() {
        return [...this.routes.values()].map((r) => ({
            streamId: r.binding.streamId,
            direction: r.binding.direction,
            kind: r.binding.kind,
            counters: { ...r.observed },
            incompleteFrames: r.assembler?.queuedFrames ?? 0,
            fragments: r.assembler?.queuedFragments ?? 0,
        }));
    }
    check() {
        if (this.stopped || this.client.closed)
            throw new ConnectError("closed", "Browser media peer closed");
    }
    die(value) {
        if (this.stopped)
            return;
        const error = value instanceof Error
            ? value
            : new ConnectError("media", "Browser media failed");
        this.close();
        this.callbacks.error?.(error);
    }
    event(frame) {
        if (frame.type === "connection.offer" &&
            typeof frame.operation_id === "string") {
            const pending = this.transitions.get(frame.operation_id);
            if (pending) {
                try {
                    for (const field of ["cid", "sid", "connid"])
                        if (frame[field] !== pending.scope[field])
                            fail("Media parameter scope mismatch");
                    const setup = frame.payload.substrate_setup;
                    if (frame.payload.substrate !== "webtransport" ||
                        !jsonObject(setup) ||
                        setup.kind !== "native-prepared/1" ||
                        setup.offer_id !== pending.requestId ||
                        setup.revision !== pending.revision + 1)
                        fail("Media parameters do not bind the current request/revision");
                    pending.accept(frame);
                }
                catch (error) {
                    pending.fail(error);
                }
            }
            else {
                const state = typeof frame.connid === "string"
                    ? this.connections.get(frame.connid)
                    : undefined;
                if (state &&
                    frame.cid === state.scope.cid &&
                    frame.sid === state.scope.sid)
                    this.serverOffer(state, frame);
            }
        }
        else if (frame.type === "message.send")
            this.dataReceived(frame);
        else if (frame.type === "connection.end" ||
            frame.type === "session.ended" ||
            frame.type === "conversation.closed") {
            for (const [id, c] of this.connections)
                if (frame.cid === c.scope.cid &&
                    (frame.type === "conversation.closed" ||
                        (frame.sid === c.scope.sid &&
                            (frame.type === "session.ended" ||
                                frame.connid === id))))
                    this.retire(id);
        }
    }
    serverOffer(state, frame) {
        if (state.reconciling ||
            [...this.transitions.values()].some((t) => t.scope.connid === state.scope.connid)) {
            if (state.pendingServer)
                return this.die(new ConnectError("backpressure", "Repeated server media negotiation"));
            state.pendingServer = frame;
            return;
        }
        const work = this.reconcile(state, frame);
        state.reconciling = work;
        void work
            .catch((error) => this.die(error))
            .finally(() => {
            state.reconciling = undefined;
            const next = state.pendingServer;
            state.pendingServer = undefined;
            if (next && !this.stopped)
                this.serverOffer(state, next);
        });
    }
    async reconcile(state, frame) {
        const setup = frame.payload.substrate_setup;
        if (!identifier(frame.operation_id) ||
            frame.payload.substrate !== "webtransport" ||
            !jsonObject(setup) ||
            setup.kind !== "native-prepared/1" ||
            !identifier(setup.offer_id) ||
            setup.revision !== state.revision + 1 ||
            !state.selectedKinds.size)
            return fail("Unbound server reconciliation parameters");
        const plan = this.plan(frame, state, state.selectedKinds);
        await this.answer(state, frame, frame.operation_id, {
            timeoutMs: 10000,
        });
        await this.completed(state.scope.cid, frame.operation_id, performance.now() + 10000);
        this.install(state, plan);
    }
    async stable(state) {
        while (state.reconciling)
            await state.reconciling;
        this.check();
    }
    answer(state, frame, operationId, options) {
        const setup = frame.payload.substrate_setup;
        return this.client.send("connection.answer", {
            by_participant: frame.payload.by_participant,
            substrate: "webtransport",
            capabilities: state.caps,
            streams_answered: setup.streams.map((s) => ({
                id: s.strm_id,
                kind: s.kind,
                direction: invert(s.direction),
                codec: s.codec,
            })),
            substrate_setup: setup,
        }, {
            ...state.scope,
            operation_id: operationId,
            in_reply_to: frame.id,
        }, options);
    }
    plan(frame, state, selectedKinds) {
        const setup = frame.payload.substrate_setup;
        if (!jsonObject(setup) ||
            !Array.isArray(setup.streams) ||
            !setup.streams.length ||
            setup.streams.length > 32 ||
            !Number.isSafeInteger(setup.revision))
            return fail("Invalid native media setup");
        const media = [], data = new Map(), received = new Set(), ids = new Set(), handles = new Set();
        for (const value of setup.streams) {
            if (!jsonObject(value) ||
                !identifier(value.strm_id) ||
                ids.has(value.strm_id) ||
                !jsonObject(value.codec) ||
                !jsonObject(value.codec.params) ||
                !Number.isInteger(value.stream_local_id) ||
                Number(value.stream_local_id) < 1 ||
                Number(value.stream_local_id) > 65535 ||
                handles.has(Number(value.stream_local_id)))
                return fail("Invalid or repeated negotiated stream");
            ids.add(value.strm_id);
            handles.add(Number(value.stream_local_id));
            const codec = value.codec, params = codec.params;
            const allowed = state.allowed.get(value.strm_id);
            const direction = invert(value.direction);
            if (allowed) {
                if (allowed.kind !== value.kind || allowed.codec !== codec.name)
                    return fail("Server changed the requested logical Stream profile");
            }
            else {
                // Explicit subscriptions create receiver ports named after source
                // Streams. They never authorize another local publisher.
                if (direction !== "recvonly" ||
                    !(state.received.has(value.strm_id) ||
                        selectedKinds.has(String(value.kind))))
                    return fail("Unrequested publisher or receiver Stream");
                received.add(value.strm_id);
            }
            if (value.kind === "data") {
                if (!["text", "json", "binary"].includes(String(codec.name)) ||
                    params.reliability !== "reliable_ordered" ||
                    !Array.isArray(state.caps.data_protocols) ||
                    !state.caps.data_protocols.includes(codec.name))
                    return fail("Unsupported reliable data format");
                data.set(value.strm_id, codec.name);
                continue;
            }
            if (!((value.kind === "audio" &&
                codec.name === "opus" &&
                params.sample_rate === 48000 &&
                params.channels === 1) ||
                ((value.kind === "video" || value.kind === "screen") &&
                    codec.name === "vp8" &&
                    params.sample_rate === 90000)) ||
                !Number.isInteger(params.payload_type) ||
                Number(params.payload_type) < 0 ||
                Number(params.payload_type) > 127)
                return fail("Server selected an unsupported media profile");
            const declared = value.kind === "audio"
                ? state.caps.audio_codecs
                : state.caps.video_codecs;
            if (!Array.isArray(declared) ||
                !declared.some((c) => jsonObject(c) && c.name === codec.name))
                return fail("Receiver codec was never declared");
            media.push({
                handle: Number(value.stream_local_id),
                streamId: value.strm_id,
                connectionId: state.scope.connid,
                sessionId: state.scope.sid,
                generation: 1,
                kind: value.kind,
                codec: codec.name,
                payloadType: Number(params.payload_type),
                direction,
            });
        }
        for (const id of state.allowed.keys())
            if (!ids.has(id))
                return fail("Server omitted a requested local Stream");
        return { media, data, revision: Number(setup.revision), received };
    }
    install(state, plan) {
        const retained = new Set(plan.media.map((b) => b.handle));
        const current = new Map();
        for (const binding of plan.media) {
            let route = this.routes.get(binding.handle);
            if (route) {
                if (canonicalJson(route.binding) !==
                    canonicalJson(binding))
                    fail("Physical handle changed its binding");
            }
            else {
                this.registry.register(binding);
                const clock = new RtpClock(binding.kind === "audio" ? 48000 : 90000);
                route = {
                    binding: Object.freeze({ ...binding }),
                    clock,
                    observed: {
                        encoded: 0,
                        keys: 0,
                        received: 0,
                        complete: 0,
                        submitted: 0,
                        skipped: 0,
                        decoded: 0,
                        expired: 0,
                    },
                };
                this.routes.set(binding.handle, route);
                if (binding.direction !== "recvonly")
                    route.sender = new RtpSender(binding, this.mtu);
                const encoded = (frame) => {
                    if (this.stopped ||
                        this.routes.get(binding.handle) !== route ||
                        !route.sender)
                        return;
                    this.registry.require(binding.handle, "send");
                    route.observed.encoded++;
                    if (frame.key)
                        route.observed.keys++;
                    const before = this.queue.droppedPackets;
                    const accepted = this.queue.push(route.sender.packetize(frame.bytes, Math.round((frame.timestampUs *
                        (binding.kind === "audio"
                            ? 48000
                            : 90000)) /
                        1000000) >>> 0), binding.kind === "audio");
                    if (!accepted || before !== this.queue.droppedPackets)
                        for (const r of this.routes.values())
                            r.vp8?.requestKeyFrame();
                    void this.drain().catch((e) => this.die(e));
                };
                if (binding.kind === "audio")
                    route.opus = new BrowserOpusCodec({
                        encoded,
                        decoded: (frame) => {
                            this.counters.decodedAudio++;
                            route.observed.decoded++;
                            this.callbacks.audio?.(route.binding, frame);
                        },
                        error: (e) => this.die(e),
                    });
                else {
                    route.assembler = new Vp8Assembler();
                    route.vp8 = new BrowserVp8Codec({
                        encoded,
                        decoded: (frame) => {
                            this.counters.decodedVideo++;
                            route.observed.decoded++;
                            this.callbacks.video?.(route.binding, frame);
                        },
                        error: (e) => this.die(e),
                    });
                }
            }
            current.set(binding.streamId, route);
        }
        for (const old of state.routes.values())
            if (!retained.has(old.binding.handle))
                this.remove(old);
        state.routes = current;
        state.data = plan.data;
        state.received = plan.received;
        state.revision = plan.revision;
    }
    async drain() {
        if (this.sending || this.stopped)
            return;
        this.sending = true;
        try {
            for (;;) {
                const bytes = this.queue.pop();
                if (!bytes || this.stopped)
                    return;
                try {
                    this.registry.require(new DataView(bytes.buffer, bytes.byteOffset).getUint16(2), "send");
                }
                catch {
                    continue;
                }
                let timer;
                try {
                    await Promise.race([
                        this.writer.write(bytes),
                        new Promise((_, reject) => {
                            timer = setTimeout(() => reject(new ConnectError("timeout", "Datagram writer stalled beyond media age")), 200);
                        }),
                    ]);
                    this.counters.sent++;
                }
                finally {
                    clearTimeout(timer);
                }
            }
        }
        finally {
            this.sending = false;
        }
    }
    async receive() {
        let burst = 0;
        while (!this.stopped) {
            const { value, done } = await this.reader.read();
            if (done) {
                if (!this.stopped)
                    throw new ConnectError("transport", "Media datagram EOF");
                return;
            }
            // A stream with queued chunks can resolve every read as a microtask.
            // Give WebCodecs output/dequeue tasks a turn within its eight-frame
            // decode bound, rather than treating task starvation as media loss.
            if (++burst === 8) {
                burst = 0;
                await new Promise((resolve) => setTimeout(resolve, 0));
                if (this.stopped)
                    return;
            }
            let packet, route;
            try {
                packet = decodeRtpDatagram(value);
                this.registry.require(packet.handle, "receive", packet.payloadType);
                route = this.routes.get(packet.handle);
            }
            catch {
                this.counters.rejected++;
                continue;
            }
            this.counters.received++;
            route.observed.received++;
            if (route.opus)
                route.opus.decode(packet.payload, route.clock.timestamp(packet.timestamp));
            else {
                let complete;
                try {
                    complete = route.assembler.push(packet);
                }
                catch {
                    this.counters.rejected++;
                    route.vp8.lost();
                    continue;
                }
                if (complete) {
                    route.observed.complete++;
                    if (route.vp8.decode(complete, route.clock.timestamp(packet.timestamp)))
                        route.observed.submitted++;
                    else
                        route.observed.skipped++;
                }
            }
        }
    }
    async offerConnection(input, options = {}) {
        this.check();
        const streams = [];
        for (const kind of ["audio", "video", "screen"])
            if (input[kind])
                streams.push({
                    id: input[kind],
                    kind,
                    direction: "sendrecv",
                    codec_preferences: [kind === "audio" ? "opus" : "vp8"],
                });
        for (const data of input.data ?? [])
            streams.push({
                id: data.id,
                kind: "data",
                direction: "sendrecv",
                codec_preferences: [data.protocol],
            });
        if (![
            input.conversationId,
            input.sessionId,
            input.connectionId,
            input.operationId,
        ].every(identifier) ||
            !streams.length ||
            streams.length > 8 ||
            streams.some((s) => !identifier(s.id)) ||
            new Set(streams.map((s) => s.id)).size !== streams.length ||
            this.connections.has(input.connectionId) ||
            this.connections.size >= 8)
            throw new ConnectError("invalid-request", "Use new scoped connection/operation IDs and 1–8 distinct streams");
        const support = await browserCodecSupport();
        if ((input.audio && !support.audio) ||
            ((input.video || input.screen) && !support.video))
            throw new ConnectError("unsupported-media-profile", support.missing.join(", "));
        const caps = {
            audio_codecs: input.audio
                ? [
                    {
                        name: "opus",
                        params: { sample_rate: 48000, channels: 1 },
                    },
                ]
                : [],
            video_codecs: input.video || input.screen
                ? [
                    {
                        name: "vp8",
                        params: { sample_rate: 90000, channels: 0 },
                    },
                ]
                : [],
            data_protocols: [
                ...new Set((input.data ?? []).map((s) => s.protocol)),
            ],
            // Retain local publishers plus independently named subscriber ports.
            max_streams_per_connection: 32,
            identity_assurance_offered: "identified",
        };
        const state = {
            scope: {
                cid: input.conversationId,
                sid: input.sessionId,
                connid: input.connectionId,
            },
            revision: 0,
            caps,
            routes: new Map(),
            data: new Map(),
            allowed: new Map(streams.map((s) => [
                String(s.id),
                { kind: s.kind, codec: s.codec_preferences[0] },
            ])),
            received: new Set(),
            selectedKinds: new Set(),
            closed: new Set(),
        };
        this.connections.set(input.connectionId, state);
        try {
            await this.transition(state, input.operationId, "connection.offer", {
                substrate: "webtransport",
                capabilities: caps,
                streams_offered: streams,
            }, options);
        }
        catch (error) {
            this.retire(input.connectionId);
            throw error;
        }
        return new BrowserMediaConnection(this, input.connectionId);
    }
    async transition(state, operationId, type, payload, options) {
        this.check();
        await this.stable(state);
        if (!identifier(operationId) ||
            this.transitions.has(operationId) ||
            [...this.transitions.values()].some((t) => t.scope.connid === state.scope.connid))
            throw new ConnectError("invalid-request", "Use a distinct stable logical operation ID");
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 60000)
            throw new ConnectError("invalid-request", "Invalid media deadline");
        const until = performance.now() + timeout;
        const remaining = () => ({
            timeoutMs: Math.max(1, until - performance.now()),
            signal: options.signal,
        });
        let accept, reject;
        const parameters = new Promise((yes, no) => {
            accept = yes;
            reject = no;
        });
        void parameters.catch(() => { });
        const scope = {
            ...state.scope,
            operation_id: operationId,
            expected_revision: state.revision,
        };
        const started = this.client.begin(type, payload, scope, {
            ...remaining(),
            terminalType: type === "connection.offer" ? "connection.answer" : undefined,
        });
        this.transitions.set(operationId, {
            scope,
            requestId: started.id,
            revision: state.revision,
            accept,
            fail: reject,
        });
        void started.response.catch(reject);
        const abort = () => {
            const error = new ConnectError("cancelled", "Media negotiation cancelled; retain logical operation ID");
            reject(error);
            this.client.close(error);
        };
        const timer = setTimeout(() => {
            const error = new ConnectError("timeout", "Media negotiation deadline; retain logical operation ID");
            reject(error);
            this.client.close(error);
        }, timeout);
        options.signal?.addEventListener("abort", abort, { once: true });
        if (options.signal?.aborted)
            abort();
        try {
            const first = await Promise.race([
                parameters.then((frame) => ({ frame })),
                started.response.then((frames) => ({ frames })),
            ]);
            let frame;
            if ("frame" in first)
                frame = first.frame;
            else if (first.frames.some((f) => f.type === "connection.answer"))
                frame = await parameters;
            else if (type === "connection.offer") {
                const status = await this.client.operationStatus(state.scope.cid, operationId, remaining());
                if (status.state === "completed")
                    throw new ConnectError("historical-operation", "Completed offer has no new physical media parameters; inspect its original operation status");
                const problem = jsonObject(status.problem)
                    ? status.problem
                    : {};
                throw new ConnectError("refused", "Media offer did not negotiate: " +
                    String(problem.code ?? status.state) +
                    (typeof problem.message === "string"
                        ? ": " + problem.message
                        : ""));
            }
            if (frame) {
                const selectedKinds = new Set();
                if (type === "stream.subscribe" &&
                    Array.isArray(payload.subscriptions))
                    for (const sub of payload.subscriptions)
                        if (jsonObject(sub) && Array.isArray(sub.kinds))
                            for (const kind of sub.kinds)
                                if (typeof kind === "string")
                                    selectedKinds.add(kind);
                const plan = this.plan(frame, state, selectedKinds);
                await this.answer(state, frame, operationId, remaining());
                await started.response;
                await this.completed(state.scope.cid, operationId, until, options.signal);
                try {
                    this.install(state, plan);
                    for (const kind of selectedKinds)
                        state.selectedKinds.add(kind);
                }
                catch (error) {
                    this.die(error);
                    throw error;
                }
            }
            else
                await this.completed(state.scope.cid, operationId, until, options.signal);
        }
        catch (error) {
            if (!(error instanceof ConnectError) ||
                !["refused", "historical-operation"].includes(error.code))
                this.client.close(error);
            throw error;
        }
        finally {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
            this.transitions.delete(operationId);
            const next = state.pendingServer;
            state.pendingServer = undefined;
            if (next && !this.stopped)
                this.serverOffer(state, next);
        }
    }
    async completed(cid, operation, until, signal) {
        for (;;) {
            const state = await this.client.operationStatus(cid, operation, {
                timeoutMs: Math.max(1, until - performance.now()),
                signal,
            });
            if (state.state === "completed")
                return;
            if (["failed", "cancelled"].includes(String(state.state))) {
                const problem = jsonObject(state.problem) ? state.problem : {};
                throw new ConnectError("refused", "Media operation " +
                    operation +
                    " did not complete: " +
                    String(problem.code ?? state.state) +
                    (typeof problem.message === "string"
                        ? ": " + problem.message
                        : ""));
            }
            if (performance.now() >= until)
                throw new ConnectError("timeout", "Media operation remains uncertain");
            await new Promise((r) => setTimeout(r, 20));
        }
    }
    state(id) {
        this.check();
        const state = this.connections.get(id);
        if (!state)
            throw new ConnectError("closed", "Media Connection retired");
        return state;
    }
    streams(id) {
        return [...this.state(id).routes.values()].map((r) => r.binding);
    }
    revision(id) {
        return this.state(id).revision;
    }
    onConnectionClose(id, callback) {
        const state = this.state(id);
        if (state.closed.size >= 16)
            throw new ConnectError("backpressure", "Too many Connection resource owners");
        state.closed.add(callback);
        return () => {
            state.closed.delete(callback);
        };
    }
    async subscribe(id, operationId, participantId, kinds, options) {
        if (!identifier(participantId) ||
            !kinds.length ||
            kinds.some((k) => !["audio", "video", "screen", "data"].includes(k)))
            throw new ConnectError("invalid-request", "Use explicit participant subscriptions");
        await this.transition(this.state(id), operationId, "stream.subscribe", { subscriptions: [{ from_participant: participantId, kinds }] }, options);
    }
    encodeAudio(id, streamId, samples, timestampUs) {
        const route = this.state(id).routes.get(streamId);
        if (!route?.opus || !route.sender)
            throw new ConnectError("unsupported-media-profile", "No negotiated audio sender for this Stream");
        return route.opus.encode(samples, timestampUs);
    }
    encodeVideo(id, streamId, frame) {
        let route;
        try {
            route = this.state(id).routes.get(streamId);
        }
        catch (error) {
            frame.close();
            throw error;
        }
        if (!route?.vp8 || !route.sender) {
            frame.close();
            throw new ConnectError("unsupported-media-profile", "No negotiated video sender for this Stream");
        }
        return route.vp8.encode(frame);
    }
    async sendData(id, operationId, messageId, streamId, body, options) {
        const state = this.state(id);
        await this.stable(state);
        const until = performance.now() + (options.timeoutMs ?? 10000);
        if (!identifier(operationId) ||
            !identifier(messageId) ||
            state.data.get(streamId) !== body.type ||
            !state.allowed.has(streamId))
            throw new ConnectError("invalid-request", "Use a negotiated data Stream and stable operation/message IDs");
        if (body.type === "text")
            canonicalJson(body.value);
        const bytes = body.type === "binary"
            ? new Uint8Array(body.value)
            : new TextEncoder().encode(body.type === "text"
                ? body.value
                : canonicalJson(body.value));
        if (bytes.length > 65536)
            throw new ConnectError("invalid-request", "Inline reliable data exceeds 64 KiB");
        let raw = "";
        if (body.type === "binary")
            for (const value of bytes)
                raw += String.fromCharCode(value);
        await this.client.request("message.send", {
            msg_id: messageId,
            to: "all",
            content_type: body.type === "text"
                ? "text/plain"
                : body.type === "json"
                    ? "application/json"
                    : "application/octet-stream",
            label: streamId,
            reliability: { mode: "reliable_ordered" },
            body: body.type === "binary"
                ? btoa(raw)
                : new TextDecoder("utf-8", { fatal: true }).decode(bytes),
            body_encoding: body.type === "binary" ? "base64" : "utf8",
        }, {
            ...state.scope,
            operation_id: operationId,
            expected_revision: state.revision,
        }, options);
        await this.completed(state.scope.cid, operationId, until, options.signal);
    }
    dataReceived(frame) {
        if (typeof frame.connid !== "string")
            return;
        const state = this.connections.get(frame.connid), p = frame.payload;
        if (!state ||
            frame.cid !== state.scope.cid ||
            frame.sid !== state.scope.sid)
            return;
        const protocol = typeof p.label === "string" ? state.data.get(p.label) : undefined;
        if (!protocol ||
            !identifier(p.msg_id) ||
            !identifier(frame.operation_id) ||
            !identifier(p.from) ||
            typeof p.body !== "string" ||
            p.body.length > 87384)
            return fail("Invalid reliable data binding");
        let body;
        if (protocol === "binary") {
            if (p.content_type !== "application/octet-stream" ||
                p.body_encoding !== "base64")
                return fail("Reliable data codec mismatch");
            const raw = atob(p.body);
            if (btoa(raw) !== p.body || raw.length > 65536)
                return fail("Invalid bounded binary body");
            body = {
                type: "binary",
                value: Uint8Array.from(raw, (c) => c.charCodeAt(0)),
            };
        }
        else {
            const bytes = new TextEncoder().encode(p.body);
            if (bytes.length > 65536 ||
                p.body_encoding !== "utf8" ||
                p.content_type !==
                    (protocol === "text" ? "text/plain" : "application/json"))
                return fail("Reliable data codec mismatch");
            body =
                protocol === "text"
                    ? { type: "text", value: p.body }
                    : { type: "json", value: parseCanonicalJson(bytes) };
        }
        this.callbacks.data?.({
            conversationId: state.scope.cid,
            sessionId: state.scope.sid,
            connectionId: frame.connid,
            streamId: String(p.label),
            operationId: frame.operation_id,
            messageId: p.msg_id,
            from: p.from,
            body,
        });
    }
    async end(id, operationId, options) {
        const state = this.state(id);
        await this.stable(state);
        await this.client.request("connection.end", { reason_code: 200, reason: "normal-clearing" }, {
            ...state.scope,
            operation_id: operationId,
            expected_revision: state.revision,
        }, options);
        await this.completed(state.scope.cid, operationId, performance.now() + (options.timeoutMs ?? 10000), options.signal);
        this.retire(id);
    }
    remove(route) {
        this.registry.retire(route.binding.handle);
        this.routes.delete(route.binding.handle);
        route.opus?.close();
        route.vp8?.close();
        route.assembler?.clear();
    }
    retire(id) {
        const state = this.connections.get(id);
        if (!state)
            return;
        for (const route of state.routes.values())
            this.remove(route);
        this.connections.delete(id);
        for (const callback of state.closed) {
            try {
                callback();
            }
            catch {
                /* Retire every resource owner. */
            }
        }
        state.closed.clear();
    }
    close() {
        if (this.stopped)
            return;
        this.stopped = true;
        clearInterval(this.timer);
        this.stopEvents?.();
        this.stopLifetime?.();
        for (const task of this.transitions.values())
            task.fail(new ConnectError("closed", "Physical media peer retired"));
        this.transitions.clear();
        for (const id of this.connections.keys())
            this.retire(id);
        for (const route of this.routes.values())
            this.remove(route);
        this.registry.clear();
        this.queue.clear();
        void this.reader.cancel().catch(() => { });
        void this.writer.abort().catch(() => { });
        this.client.close();
    }
}
export class BrowserMediaConnection {
    peer;
    id;
    constructor(peer, id) {
        this.peer = peer;
        this.id = id;
    }
    get streams() {
        return this.peer.streams(this.id);
    }
    get revision() {
        return this.peer.revision(this.id);
    }
    onClose(callback) {
        return this.peer.onConnectionClose(this.id, callback);
    }
    subscribe(operationId, participantId, kinds, options = {}) {
        return this.peer.subscribe(this.id, operationId, participantId, kinds, options);
    }
    encodeAudio(streamId, samples, timestampUs) {
        return this.peer.encodeAudio(this.id, streamId, samples, timestampUs);
    }
    encodeVideo(streamId, frame) {
        return this.peer.encodeVideo(this.id, streamId, frame);
    }
    sendData(operationId, messageId, streamId, body, options = {}) {
        return this.peer.sendData(this.id, operationId, messageId, streamId, body, options);
    }
    end(operationId, options = {}) {
        return this.peer.end(this.id, operationId, options);
    }
}
