import { ConnectError } from "./errors.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import { replacementFrame, replacementReceipt, } from "./browser_replacement.js";
const copy = (v) => parseCanonicalJson(new TextEncoder().encode(canonicalJson(v)));
const positive = (v) => Number.isSafeInteger(v) && Number(v) > 0;
function bad(message) {
    throw new ConnectError("protocol", message);
}
const transient = (error) => error instanceof ConnectError &&
    (["timeout", "transport"].includes(error.code) ||
        (error.code === "refused" &&
            error.problem?.code === 503 &&
            ((error.problem.category === "authorization" &&
                error.problem.reason === "authority_unavailable") ||
                (error.problem.category === "storage" &&
                    error.problem.reason === "storage_unavailable"))));
/** A new instance belongs to precisely one installed target route set. */
export class BrowserNativeReadiness {
    root;
    audioStreamId;
    videoStreamId;
    challenge;
    original;
    scope;
    audio;
    audioSamples = 0;
    video;
    videoFrames = 0;
    videoKeys = 0;
    frozen;
    frozenAudio;
    frozenVideo;
    attempted = false;
    refused = false;
    stopped = false;
    completed = false;
    revision = -1;
    targetGeneration;
    constructor(root, original, scope, answer, callback, requestId, audioStreamId, videoStreamId, connectionGeneration) {
        this.root = root;
        this.audioStreamId = audioStreamId;
        this.videoStreamId = videoStreamId;
        for (const frame of [answer, callback]) {
            if (frame.cid !== root.conversationId ||
                frame.sid !== root.sessionId ||
                frame.connid !== root.targetConnectionId ||
                frame.operation_id !== root.operationId)
                bad("Native readiness changed the original physical scope");
        }
        if (answer.type !== "connection.answer" ||
            answer.in_reply_to !== requestId ||
            callback.type !== "connection.offer" ||
            callback.in_reply_to !== undefined ||
            scope.operation_id !== root.operationId ||
            scope.connid !== root.targetConnectionId ||
            scope.cid !== root.conversationId ||
            scope.sid !== root.sessionId ||
            scope.expected_revision !== 0)
            bad("Native readiness requires both original signed negotiation legs");
        const marker = callback.payload.replacement_preparation;
        if (!jsonObject(marker) ||
            Object.keys(marker).sort().join() !==
                "operation_record_id,profile,readiness" ||
            marker.profile !== "connect-replacement-media-v1" ||
            marker.operation_record_id !== root.recordId ||
            !jsonObject(marker.readiness) ||
            canonicalJson(marker) !==
                canonicalJson(answer.payload.replacement_preparation ?? null))
            bad("Native readiness challenge changed between negotiation legs");
        const challenge = marker.readiness;
        if (Object.keys(challenge).sort().join() !==
            "nonce,source_generation,stream_revision,target_generation" ||
            typeof challenge.nonce !== "string" ||
            !identifier(challenge.nonce) ||
            !positive(challenge.source_generation) ||
            !positive(challenge.target_generation) ||
            challenge.target_generation !== challenge.source_generation + 1 ||
            challenge.stream_revision !== 2 ||
            (connectionGeneration !== 0 &&
                connectionGeneration !== challenge.target_generation) ||
            (videoStreamId !== undefined && connectionGeneration === 0))
            bad("Native readiness challenge changed its original generation or revision");
        for (const value of [
            answer.payload.substrate_setup,
            callback.payload.substrate_setup,
        ])
            if (!jsonObject(value) ||
                value.kind !== "native-prepared/1" ||
                value.revision !== 1 ||
                value.offer_id !== requestId)
                bad("Native negotiation and private readiness revisions were conflated");
        if (canonicalJson(answer.payload.substrate_setup) !==
            canonicalJson(callback.payload.substrate_setup))
            bad("Native answer changed the original selected resources");
        if (!jsonObject(original.replacement_preparation) ||
            original.replacement_preparation.profile !==
                "connect-replacement-media-v1" ||
            original.replacement_preparation.operation_record_id !==
                root.recordId ||
            !identifier(audioStreamId) ||
            (videoStreamId !== undefined &&
                (!identifier(videoStreamId) || videoStreamId === audioStreamId)))
            bad("Native readiness lost the immutable original media offer");
        this.targetGeneration = challenge.target_generation;
        this.challenge = copy(challenge);
        this.original = copy(original);
        this.scope = { ...scope };
    }
    get mayHaveCommitted() {
        return this.attempted && !this.refused;
    }
    get observing() {
        return !this.stopped && !this.completed && !this.frozen;
    }
    get savedReadiness() {
        return this.frozen ? copy(this.frozen) : undefined;
    }
    accepts(identity, stream) {
        return (identity.connectionId === this.root.targetConnectionId &&
            identity.streamId === stream &&
            positive(identity.generation) &&
            Number.isInteger(identity.ssrc) &&
            identity.ssrc >= 0 &&
            identity.ssrc <= 0xffffffff);
    }
    audioDecoded(value) {
        if (!this.observing)
            return;
        const now = performance.now();
        if (!this.accepts(value.identity, this.audioStreamId) ||
            value.samples !== 960 ||
            !Number.isFinite(value.arrived) ||
            now < value.arrived ||
            now - value.arrived >= 200)
            return;
        const previous = this.audio;
        // Repeated original input is not another actual eligible observation.
        if (previous &&
            previous.identity.ssrc === value.identity.ssrc &&
            (value.sequence === previous.sequence ||
                ((value.sequence - previous.sequence) & 65535) >= 32768))
            return;
        if (previous &&
            (canonicalJson(previous.identity) !==
                canonicalJson(value.identity) ||
                value.sequence !== ((previous.sequence + 1) & 65535) ||
                value.timestamp !== (previous.timestamp + 960) >>> 0))
            this.audioSamples = 0;
        this.audio = { ...value, identity: { ...value.identity } };
        this.audioSamples += value.samples;
    }
    videoDecoded(value) {
        if (!this.observing || !this.videoStreamId)
            return;
        const now = performance.now();
        if (!this.accepts(value.identity, this.videoStreamId) ||
            !value.epoch ||
            value.width !== 640 ||
            value.height !== 360 ||
            !Number.isFinite(value.arrived) ||
            now < value.arrived ||
            now - value.arrived >= 200)
            return;
        if (this.video &&
            (this.video.epoch !== value.epoch ||
                canonicalJson(this.video.identity) !==
                    canonicalJson(value.identity))) {
            this.videoFrames = 0;
            this.videoKeys = 0;
        }
        this.video = { ...value, identity: { ...value.identity } };
        this.videoFrames++;
        if (value.key)
            this.videoKeys++;
    }
    sameCurrent(current) {
        const audio = this.frozenAudio ?? this.audio?.identity, video = this.frozenVideo ??
            (this.video
                ? { identity: this.video.identity, epoch: this.video.epoch }
                : undefined);
        return (!!audio &&
            canonicalJson((current.audio ?? null)) ===
                canonicalJson(audio) &&
            (!this.videoStreamId ||
                (!!video &&
                    canonicalJson((current.video ?? null)) === canonicalJson(video))));
    }
    freeze(current) {
        if (this.frozen) {
            if (!this.sameCurrent(current))
                bad("Frozen native readiness physical source changed");
            return true;
        }
        const now = performance.now();
        if (!this.audio ||
            this.audioSamples < 2880 ||
            now - this.audio.arrived >= 200 ||
            !this.sameCurrent(current) ||
            (this.videoStreamId &&
                (!this.video ||
                    !this.videoKeys ||
                    now - this.video.arrived >= 200)))
            return false;
        this.frozenAudio = { ...this.audio.identity };
        this.frozenVideo = this.video
            ? { identity: { ...this.video.identity }, epoch: this.video.epoch }
            : undefined;
        this.frozen = {
            ...copy(this.original),
            replacement_preparation: {
                profile: "connect-replacement-ready-v1",
                operation_record_id: this.root.recordId,
                readiness: {
                    ...this.challenge,
                    audio: {
                        stream_id: this.audioStreamId,
                        decoded_samples: this.audioSamples,
                    },
                    video: this.videoStreamId
                        ? {
                            stream_id: this.videoStreamId,
                            decoded_frames: this.videoFrames,
                            decoded_keyframes: this.videoKeys,
                            decoder_startup: "fresh-peer-first-decoded-frame",
                        }
                        : null,
                },
            },
        };
        return true;
    }
    close() {
        this.stopped = true;
        this.audio = this.video = undefined;
    }
    /** The original caller supplies one absolute deadline and actual route check. */
    async finish(client, until, current, signal) {
        const check = () => {
            if (this.stopped || client.closed)
                throw new ConnectError("closed", "Original native readiness owner closed");
            if (signal?.aborted)
                throw new ConnectError("cancelled", "Original native readiness wait cancelled");
            if (until - performance.now() < 1)
                throw new ConnectError("timeout", "Original native readiness deadline expired");
            return current();
        };
        const pause = async () => {
            check();
            await new Promise((resolve) => setTimeout(resolve, Math.min(10, Math.max(1, until - performance.now()))));
            check();
        };
        const options = () => ({
            signal,
            timeoutMs: Math.max(1, Math.min(1000, Math.floor((until - performance.now()) / 2))),
        });
        while (!this.freeze(check()))
            await pause();
        let attempts = 0, replay = false, replayDisabled = false, next = 0, definitive;
        const observe = (value) => {
            if (Number(value.revision) < this.revision)
                bad("Original native replacement revision regressed");
            this.revision = Number(value.revision);
            return value;
        };
        const send = async () => {
            const physical = check();
            if (this.attempted && !this.sameCurrent(physical)) {
                // Real public activation may replace preparation silence's
                // SSRC before its durable receipt arrives. A changed source is
                // never commitment evidence and never refreshes frozen proof.
                replay = false;
                replayDisabled = true;
                return;
            }
            this.freeze(physical);
            this.attempted = true;
            attempts++;
            try {
                observe(replacementFrame(this.root, await client.requestReplacementContinuation("connection.offer", copy(this.frozen), { ...this.scope }, options()), this.root.targetConnectionId));
                replay = false;
            }
            catch (error) {
                if (signal?.aborted || client.closed)
                    throw error;
                if (!transient(error)) {
                    if (error instanceof ConnectError &&
                        error.code === "refused") {
                        definitive = error;
                        replay = false;
                        return;
                    }
                    throw error;
                }
                replay = true;
                next =
                    performance.now() +
                        Math.min(1000, 50 * 2 ** (attempts - 1));
            }
        };
        try {
            await send();
            for (;;) {
                check();
                let status;
                try {
                    status = observe(replacementFrame(this.root, await client.requestReplacementContinuation("operation.get", { operation_id: this.root.operationId }, {
                        cid: this.root.conversationId,
                        sid: this.root.sessionId,
                        connid: this.root.sourceConnectionId,
                    }, options()), this.root.sourceConnectionId));
                }
                catch (error) {
                    if (signal?.aborted || client.closed || !transient(error))
                        throw error;
                    await pause();
                    continue;
                }
                if (status.state === "completed") {
                    let receipt;
                    try {
                        receipt = replacementReceipt(this.root, status);
                    }
                    catch (error) {
                        if (error instanceof ConnectError &&
                            error.code === "refused")
                            this.refused = true;
                        throw error;
                    }
                    if (receipt.generation !== this.targetGeneration ||
                        receipt.streamRevision <=
                            Number(this.challenge.stream_revision))
                        bad("Native completion changed the prepared generation");
                    this.completed = true;
                    this.audio = this.video = undefined;
                    if (definitive)
                        throw definitive;
                    return status;
                }
                if (["failed", "cancelled"].includes(String(status.state))) {
                    this.refused = true;
                    throw (definitive ??
                        new ConnectError("refused", "Original native replacement did not complete"));
                }
                if (![
                    "accepted",
                    "preparing",
                    "ready",
                    "committing",
                    "unknown",
                    "interrupted",
                ].includes(String(status.state)))
                    bad("Invalid original native replacement state");
                // Pending alone proves no absence. The server's exact captured
                // observation replay contract permits only these identical bytes.
                if (replay &&
                    !replayDisabled &&
                    attempts < 8 &&
                    performance.now() >= next &&
                    status.state === "preparing" &&
                    status.native_replacement_prepared === true)
                    await send();
                else
                    await pause();
            }
        }
        catch (error) {
            throw definitive ?? error;
        }
    }
}
