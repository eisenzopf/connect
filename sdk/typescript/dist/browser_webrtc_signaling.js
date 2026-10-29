import { BrowserTasks } from "./browser_tasks.js";
import { ConnectError } from "./errors.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
const maximumSdpBytes = 512 * 1024;
const maximumCandidates = 64;
const maximumCandidateBytes = 64 * 1024;
const snapshot = (value) => parseCanonicalJson(new TextEncoder().encode(canonicalJson(value)));
const protocol = (message) => new ConnectError("protocol", message);
function validSdp(value) {
    return (typeof value === "string" &&
        value.startsWith("v=0\r\n") &&
        !value.includes("\0") &&
        new TextEncoder().encode(value).byteLength <= maximumSdpBytes);
}
function candidate(value) {
    if (typeof value.candidate !== "string" ||
        new TextEncoder().encode(value.candidate).byteLength > 4096 ||
        /[\r\n\0]/.test(value.candidate) ||
        typeof value.sdp_mid !== "string" ||
        !identifier(value.sdp_mid) ||
        !Number.isInteger(value.sdp_m_line_index) ||
        Number(value.sdp_m_line_index) < 0 ||
        Number(value.sdp_m_line_index) > 65535)
        throw protocol("Invalid canonical ICE candidate");
    return {
        candidate: value.candidate,
        sdpMid: value.sdp_mid,
        sdpMLineIndex: Number(value.sdp_m_line_index),
    };
}
/** Internal canonical signaling owner. Owns the supplied RTCPeerConnection, not
 * its capture tracks or the shared signed client. An applied answer establishes
 * signaling only; media readiness needs independent WebRTC/media observations.
 * The explicit server host admits its bounded negotiated profile; an answer
 * alone does not establish Core readiness or decoded playback.
 */
export class CanonicalWebRtcSignaling {
    client;
    peer;
    tasks = new BrowserTasks();
    finished;
    resolveFinished;
    rejectFinished;
    scope;
    template;
    attempted = false;
    stopped = false;
    failure;
    submitted = false;
    original;
    remoteApplied = false;
    ice = [];
    candidateCount = 0;
    candidateBytes = 0;
    draining;
    stopEvents;
    stopLifetime;
    rejectWait;
    peerStateChanged = () => {
        if (this.stopped)
            return;
        if (this.peer.connectionState === "failed" ||
            this.peer.iceConnectionState === "failed")
            this.die(new ConnectError("webrtc-failed", "WebRTC peer transport failed; retain original operation ID"));
        else if (this.peer.connectionState === "closed")
            this.close();
    };
    constructor(client, peer, options) {
        this.client = client;
        this.peer = peer;
        if (client.controlTransport !== "websocket")
            throw new ConnectError("unsupported-transport", "Canonical WebRTC requires an existing signed WebSocket client");
        if (client.closed)
            throw new ConnectError("closed", "Canonical control client closed");
        if (![
            options.conversationId,
            options.sessionId,
            options.connectionId,
            options.operationId,
            options.participantId,
        ].every(identifier) ||
            !Number.isSafeInteger(options.expectedRevision) ||
            options.expectedRevision < 0 ||
            !jsonObject(options.capabilities) ||
            options.streams.length < 1 ||
            options.streams.length > 16 ||
            new Set(options.streams.map((s) => s.id)).size !==
                options.streams.length ||
            options.streams.some((s) => !identifier(s.id) ||
                !["audio", "video", "screen", "data"].includes(s.kind) ||
                !["sendrecv", "sendonly", "recvonly", "inactive"].includes(s.direction) ||
                !s.codec_preferences.length ||
                s.codec_preferences.length > 16 ||
                s.codec_preferences.some((c) => typeof c !== "string" ||
                    !/^[a-zA-Z0-9.+_-]{1,64}$/.test(c))))
            throw new ConnectError("invalid-request", "Explicit WebRTC scope, revision and bounded stream offers required");
        this.scope = {
            cid: options.conversationId,
            sid: options.sessionId,
            connid: options.connectionId,
            operation_id: options.operationId,
            expected_revision: options.expectedRevision,
        };
        this.template = snapshot({
            by_participant: options.participantId,
            substrate: "websocket+webrtc",
            capabilities: options.capabilities,
            streams_offered: options.streams.map((s) => ({
                ...s,
                codec_preferences: [...s.codec_preferences],
            })),
        });
        this.finished = new Promise((resolve, reject) => {
            this.resolveFinished = resolve;
            this.rejectFinished = reject;
        });
        void this.finished.catch(() => { });
    }
    /** Save this exact original intent for status recovery, never synthesize a replacement offer. */
    get savedOffer() {
        return this.original
            ? {
                scope: { ...this.scope },
                payload: snapshot(this.original),
                submissionAttempted: this.submitted,
            }
            : undefined;
    }
    check() {
        if (this.stopped)
            throw (this.failure ??
                new ConnectError("closed", "WebRTC signaling owner closed"));
    }
    matches(frame) {
        return (["cid", "sid", "connid"].every((k) => frame[k] === this.scope[k]) && frame.operation_id === this.scope.operation_id);
    }
    receive(frame) {
        if (this.stopped)
            return;
        if ((frame.type === "conversation.closed" &&
            frame.cid === this.scope.cid) ||
            (frame.type === "session.ended" &&
                frame.cid === this.scope.cid &&
                frame.sid === this.scope.sid) ||
            (frame.type === "connection.end" &&
                frame.cid === this.scope.cid &&
                frame.sid === this.scope.sid &&
                frame.connid === this.scope.connid)) {
            this.close();
            return;
        }
        if (frame.type !== "connection.ice-candidate" || !this.matches(frame))
            return;
        try {
            const init = candidate(frame.payload);
            const bytes = new TextEncoder().encode(canonicalJson(frame.payload)).byteLength;
            if (++this.candidateCount > maximumCandidates ||
                (this.candidateBytes += bytes) > maximumCandidateBytes)
                throw new ConnectError("backpressure", "Canonical ICE candidate lifetime budget exceeded");
            this.ice.push(init);
            void this.drain().catch((error) => this.die(error));
        }
        catch (error) {
            this.die(error);
        }
    }
    drain() {
        if (!this.remoteApplied || this.stopped)
            return Promise.resolve();
        if (this.draining)
            return this.draining;
        this.draining = (async () => {
            while (!this.stopped && this.ice.length) {
                await this.peer.addIceCandidate(this.ice.shift());
                this.check();
            }
        })().finally(() => {
            this.draining = undefined;
            if (!this.stopped && this.ice.length)
                void this.drain().catch((error) => this.die(error));
        });
        return this.tasks.track(this.draining);
    }
    /** One complete-ICE offer. No automatic retry, fallback, redial or message send. */
    async offer(options = {}) {
        this.check();
        if (this.attempted)
            throw new ConnectError("conflict", "WebRTC original offer already attempted; inspect its original status");
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 60000)
            throw new ConnectError("invalid-request", "WebRTC deadline must be 1–60000 ms");
        this.attempted = true;
        const until = performance.now() + timeout;
        const aborted = () => this.die(new ConnectError("cancelled", "WebRTC wait cancelled; retain original operation ID"));
        const timer = setTimeout(() => this.die(new ConnectError("timeout", "WebRTC signaling deadline; retain original operation ID")), timeout);
        const stopped = new Promise((_, reject) => {
            this.rejectWait = reject;
        });
        void stopped.catch(() => { });
        const bound = async (work) => {
            const result = await Promise.race([
                this.tasks.track(work),
                stopped,
            ]);
            this.check();
            return result;
        };
        const remaining = () => ({
            timeoutMs: Math.max(1, until - performance.now()),
            signal: options.signal,
        });
        options.signal?.addEventListener("abort", aborted, { once: true });
        try {
            if (options.signal?.aborted)
                aborted();
            this.check();
            this.stopLifetime = this.client.onClose((error) => this.die(error));
            this.check();
            this.stopEvents = this.client.onEvent((frame) => this.receive(frame));
            if (this.stopped) {
                this.stopEvents();
                this.check();
            }
            this.peer.addEventListener("connectionstatechange", this.peerStateChanged);
            this.peer.addEventListener("iceconnectionstatechange", this.peerStateChanged);
            this.peerStateChanged();
            this.check();
            const offer = await bound(this.peer.createOffer());
            await bound(this.peer.setLocalDescription(offer));
            await bound(this.gather());
            const sdp = this.peer.localDescription?.sdp;
            if (!validSdp(sdp))
                throw protocol("Invalid bounded complete WebRTC offer");
            this.original = {
                ...snapshot(this.template),
                substrate_setup: { kind: "websocket+webrtc", sdp },
            };
            // Set before handing bytes to the shared client; failure can be uncertain.
            this.submitted = true;
            const frames = await bound(this.client.request("connection.offer", snapshot(this.original), { ...this.scope }, { ...remaining(), terminalType: "connection.answer" }));
            const answers = frames.filter((f) => f.type === "connection.answer");
            if (answers.length !== 1)
                throw new ConnectError("unresolved-operation", "WebRTC offer has no new answer; inspect its original operation status");
            const answer = answers[0], setup = answer.payload.substrate_setup;
            const streams = answer.payload.streams_answered;
            const offered = new Map(this.template.streams_offered.map((s) => [
                s.id,
                s.kind,
            ]));
            if (!this.matches(answer) ||
                answer.payload.substrate !== "websocket+webrtc" ||
                !jsonObject(setup) ||
                setup.kind !== "websocket+webrtc" ||
                !validSdp(setup.sdp) ||
                !jsonObject(answer.payload.capabilities) ||
                !Array.isArray(streams) ||
                !streams.length ||
                streams.length !== offered.size ||
                new Set(streams.map((s) => (jsonObject(s) ? s.id : undefined)))
                    .size !== streams.length ||
                streams.some((s) => !jsonObject(s) ||
                    !offered.has(s.id) ||
                    offered.get(s.id) !== s.kind ||
                    ![
                        "sendrecv",
                        "sendonly",
                        "recvonly",
                        "inactive",
                    ].includes(String(s.direction)) ||
                    s.codec === undefined))
                throw protocol("WebRTC answer does not match the original Connection and stream offer");
            await bound(this.peer.setRemoteDescription({
                type: "answer",
                sdp: setup.sdp,
            }));
            this.remoteApplied = true;
            for (const frame of frames)
                this.receive(frame);
            await bound(this.drain());
        }
        catch (error) {
            this.die(error);
            throw this.failure ?? error;
        }
        finally {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", aborted);
            this.rejectWait = undefined;
        }
    }
    gatheringCleanup;
    gather() {
        if (this.peer.iceGatheringState === "complete")
            return Promise.resolve();
        return new Promise((resolve) => {
            const changed = () => {
                if (this.peer.iceGatheringState === "complete") {
                    cleanup();
                    resolve();
                }
            };
            const cleanup = () => {
                this.peer.removeEventListener("icegatheringstatechange", changed);
                this.gatheringCleanup = undefined;
            };
            this.gatheringCleanup = cleanup;
            this.peer.addEventListener("icegatheringstatechange", changed);
            changed();
        });
    }
    die(value) {
        if (this.stopped)
            return;
        this.failure =
            value instanceof Error
                ? value
                : protocol("WebRTC signaling failed");
        this.retire();
        this.rejectWait?.(this.failure);
        this.rejectFinished(this.failure);
    }
    retire() {
        this.stopped = true;
        this.peer.removeEventListener("connectionstatechange", this.peerStateChanged);
        this.peer.removeEventListener("iceconnectionstatechange", this.peerStateChanged);
        this.gatheringCleanup?.();
        this.stopEvents?.();
        this.stopLifetime?.();
        this.ice = [];
        this.peer.close();
    }
    /** Local owner retirement only. It does not cancel the durable remote operation. */
    close() {
        if (this.stopped)
            return;
        this.retire();
        this.rejectWait?.(new ConnectError("closed", "WebRTC signaling owner closed"));
        this.resolveFinished();
    }
    async shutdown() {
        this.close();
        await this.tasks.join();
    }
}
