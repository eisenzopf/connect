import { BrowserDeadline } from "./browser_deadline.js";
import { replacementFrame, replacementReceipt, replacementRoot, } from "./browser_replacement.js";
import { ConnectError } from "./errors.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
const protocol = (message) => new ConnectError("protocol", message);
const copy = (value) => parseCanonicalJson(new TextEncoder().encode(canonicalJson(value)));
const positive = (value) => typeof value === "number" && Number.isSafeInteger(value) && value > 0;
/** One target's original replacement phases. A readiness assertion is a signed
 * observation of this fresh decoder, never a new replacement mutation. */
export class BrowserWebRtcReplacement {
    client;
    root;
    allocated = false;
    readyAttempted = false;
    refused = false;
    original;
    originalScope;
    observedRevision = -1;
    selected;
    readyOffer;
    constructor(client, root) {
        this.client = client;
        this.root = replacementRoot(root);
        if (client.controlTransport !== "websocket" ||
            client.deviceId !== root.deviceId)
            throw new ConnectError("invalid-request", "Replacement belongs to the current signed target Device");
    }
    get mayHaveCommitted() {
        return this.readyAttempted && !this.refused;
    }
    get challenge() {
        if (!this.selected)
            throw new ConnectError("conflict", "Replacement has no current decoder challenge");
        return this.selected;
    }
    get savedReadiness() {
        return this.readyOffer ? copy(this.readyOffer) : undefined;
    }
    observe(value) {
        if (Number(value.revision) < this.observedRevision)
            throw protocol("Original replacement revision regressed");
        this.observedRevision = Number(value.revision);
        return value;
    }
    async status(options) {
        const root = this.root;
        const frames = await this.client.requestReplacementContinuation("operation.get", { operation_id: root.operationId }, {
            cid: root.conversationId,
            sid: root.sessionId,
            connid: root.sourceConnectionId,
        }, options);
        return this.observe(replacementFrame(root, frames, root.sourceConnectionId));
    }
    async allocate(payload, scope, options, attempted) {
        const root = this.root;
        if (this.allocated ||
            this.original ||
            scope.cid !== root.conversationId ||
            scope.sid !== root.sessionId ||
            scope.connid !== root.targetConnectionId ||
            scope.operation_id !== root.operationId ||
            scope.expected_revision !== 0)
            throw new ConnectError("conflict", "Use the original WebRTC replacement allocation once");
        const budget = new BrowserDeadline(options.timeoutMs ?? 10000);
        const observed = await this.status({
            ...options,
            timeoutMs: budget.remaining(),
        });
        if (observed.state !== "preparing" ||
            observed.native_replacement_prepared !== true)
            throw new ConnectError("conflict", "Original replacement preparation is not current");
        this.original = copy(payload);
        this.originalScope = { ...scope };
        attempted?.();
        const frames = await this.client.requestReplacementContinuation("connection.offer", {
            ...copy(payload),
            replacement_preparation: {
                profile: "connect-replacement-preparation-v1",
                operation_record_id: root.recordId,
            },
        }, { ...scope }, { ...options, timeoutMs: budget.remaining() });
        const allocated = this.observe(replacementFrame(root, frames, root.targetConnectionId));
        if (allocated.state !== "preparing")
            throw new ConnectError("conflict", "Original WebRTC replacement did not remain preparing");
        this.allocated = true;
    }
    mediaOffer(payload) {
        if (!this.allocated ||
            !this.original ||
            canonicalJson(payload) !== canonicalJson(this.original))
            throw protocol("WebRTC replacement media changed the original immutable offer");
        return {
            ...copy(payload),
            replacement_preparation: {
                profile: "connect-replacement-media-v1",
                operation_record_id: this.root.recordId,
            },
        };
    }
    answer(payload) {
        const marker = payload.replacement_preparation;
        const challenge = jsonObject(marker) ? marker.readiness : undefined;
        if (!this.allocated ||
            this.selected ||
            !jsonObject(marker) ||
            marker.profile !== "connect-replacement-media-v1" ||
            marker.operation_record_id !== this.root.recordId ||
            !jsonObject(challenge) ||
            typeof challenge.nonce !== "string" ||
            !identifier(challenge.nonce) ||
            !positive(challenge.source_generation) ||
            !positive(challenge.target_generation) ||
            challenge.target_generation !== challenge.source_generation + 1 ||
            !positive(challenge.stream_revision))
            throw protocol("Replacement answer changed its current decoder challenge");
        this.selected = Object.freeze({
            nonce: challenge.nonce,
            source_generation: challenge.source_generation,
            target_generation: challenge.target_generation,
            stream_revision: challenge.stream_revision,
        });
    }
    async complete(evidence, options) {
        const root = this.root, challenge = this.challenge;
        if (this.readyAttempted || !this.original || !this.originalScope)
            throw new ConnectError("conflict", "Use the original decoder readiness continuation once");
        const streams = this.original.streams_offered;
        if (!Array.isArray(streams))
            throw protocol("Replacement has no frozen stream offer");
        const audio = streams.filter((s) => jsonObject(s) &&
            s.kind === "audio" &&
            s.direction === "sendrecv");
        const video = streams.filter((s) => jsonObject(s) &&
            s.kind === "video" &&
            s.direction === "recvonly");
        if (audio.length !== 1 ||
            !jsonObject(audio[0]) ||
            evidence.audio.stream_id !== audio[0].id ||
            !positive(evidence.audio.decoded_samples) ||
            video.length > 1 ||
            (video.length === 0
                ? evidence.video !== null
                : !jsonObject(video[0]) ||
                    !evidence.video ||
                    evidence.video.stream_id !== video[0].id ||
                    !positive(evidence.video.decoded_frames) ||
                    evidence.video.decoder_startup !==
                        "fresh-peer-first-decoded-frame" ||
                    (evidence.video.decoded_keyframes !== null &&
                        !positive(evidence.video.decoded_keyframes))))
            throw protocol("Decoded readiness does not name the frozen target receive streams");
        const budget = new BrowserDeadline(options.timeoutMs ?? 10000);
        this.readyOffer = {
            ...copy(this.original),
            replacement_preparation: {
                profile: "connect-replacement-ready-v1",
                operation_record_id: root.recordId,
                readiness: {
                    ...challenge,
                    audio: { ...evidence.audio },
                    video: evidence.video ? { ...evidence.video } : null,
                },
            },
        };
        // Mark before sending: lost response cannot cause a fresh ready effect.
        this.readyAttempted = true;
        let value;
        try {
            const frames = await this.client.requestReplacementContinuation("connection.offer", copy(this.readyOffer), { ...this.originalScope }, { ...options, timeoutMs: budget.remaining() });
            value = this.observe(replacementFrame(root, frames, root.targetConnectionId));
        }
        catch (error) {
            // Recovery reads the original journal. It never replays the ready
            // mutation or drops the target merely because its reply was lost.
            if (options.signal?.aborted)
                throw error;
            try {
                budget.remaining();
            }
            catch {
                throw error;
            }
            value = await this.status({
                ...options,
                timeoutMs: budget.remaining(),
            });
        }
        for (;;) {
            if (value.state === "completed") {
                const receipt = replacementReceipt(root, value);
                const result = value.result;
                const actual = result.readiness
                    .streams;
                const offered = new Set(streams.filter(jsonObject).map((s) => String(s.id)));
                if (receipt.generation !== challenge.target_generation ||
                    receipt.streamRevision <= challenge.stream_revision ||
                    actual.length !== offered.size ||
                    new Set(actual).size !== actual.length ||
                    actual.some((id) => !offered.has(id)))
                    throw protocol("Replacement receipt changed the prepared generation or streams");
                return receipt;
            }
            if (["failed", "cancelled"].includes(String(value.state))) {
                this.refused = true;
                throw new ConnectError("refused", "Original WebRTC replacement did not complete");
            }
            if (![
                "accepted",
                "preparing",
                "ready",
                "committing",
                "unknown",
                "interrupted",
            ].includes(String(value.state)))
                throw protocol("Invalid original WebRTC replacement state");
            await budget.run(() => new Promise((resolve) => setTimeout(resolve, Math.min(20, budget.remaining()))), options.signal);
            value = await this.status({
                ...options,
                timeoutMs: budget.remaining(),
            });
        }
    }
}
