/** Internal state owned by one authenticated BrowserMediaPeer. Wire tags and
 * notices are history, not permission; the server rechecks current authority. */
import { canonicalJson, jsonObject } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import { captureNativeVideoFeedback } from "./native_video_feedback.js";
import { VIDEO_FEEDBACK_PROFILE, encodeNativeVideoFeedback, } from "./browser_video_wire.js";
import { ConnectError } from "./errors.js";
const bad = () => {
    throw new ConnectError("protocol", "Invalid authenticated native video history");
};
const positive = (n) => Number.isSafeInteger(n) && Number(n) > 0;
const uint = (n) => Number.isInteger(n) && Number(n) >= 0 && Number(n) <= 0xffffffff;
const hex = (s) => typeof s === "string" && /^[0-9a-f]{32}$/.test(s) && !/^0+$/.test(s);
const same = (a, b) => canonicalJson(a) === canonicalJson(b);
export function videoAllocation(stream, local, remote, operation, offer, generation, previous) {
    const enabled = stream.kind === "video" &&
        local.video_feedback === VIDEO_FEEDBACK_PROFILE &&
        remote.video_feedback === VIDEO_FEEDBACK_PROFILE;
    if (stream.video_feedback === undefined)
        return enabled || previous ? bad() : undefined;
    if (!enabled)
        return bad();
    const value = captureNativeVideoFeedback(stream.video_feedback);
    if (value.streamGeneration !== generation ||
        value.connectionGeneration !== 1 ||
        (previous
            ? !same(value, previous)
            : value.originalOperation !== operation ||
                value.originalOffer !== offer))
        return bad();
    return value;
}
/** Call only with an unsolicited frame from this physical client's verified
 * event reader. An operation projection or arbitrary payload is not input. */
export function videoNotice(frame) {
    const p = frame.payload;
    const fields = [
        "profile",
        "strm_id",
        "stream_local_id",
        "original_operation",
        "original_offer",
        "incarnation",
        "stream_generation",
        "connection_generation",
        "revision",
        "direction",
        "epoch",
        "ssrc",
        "first_sequence",
        "first_timestamp",
    ];
    if (frame.type !== "stream.epoch" ||
        frame.in_reply_to !== undefined ||
        frame.operation_id !== undefined ||
        !identifier(frame.cid) ||
        !identifier(frame.sid) ||
        !identifier(frame.connid) ||
        !jsonObject(p) ||
        Object.keys(p).length !== fields.length ||
        !fields.every((k) => Object.hasOwn(p, k)) ||
        !identifier(p.strm_id) ||
        !positive(p.stream_local_id) ||
        p.stream_local_id > 65535 ||
        !hex(p.incarnation) ||
        !hex(p.epoch) ||
        !positive(p.revision) ||
        !uint(p.ssrc) ||
        !p.ssrc ||
        !uint(p.first_sequence) ||
        !uint(p.first_timestamp) ||
        !["publisher", "receiver"].includes(String(p.direction)))
        return bad();
    const allocation = captureNativeVideoFeedback({
        profile: p.profile,
        original_operation: p.original_operation,
        original_offer: p.original_offer,
        stream_generation: p.stream_generation,
        connection_generation: p.connection_generation,
    });
    return Object.freeze({
        root: Object.freeze({
            cid: frame.cid,
            sid: frame.sid,
            connid: frame.connid,
        }),
        stream: p.strm_id,
        handle: p.stream_local_id,
        allocation,
        role: p.direction,
        epoch: p.epoch,
        incarnation: p.incarnation,
        revision: p.revision,
        ssrc: p.ssrc,
        firstSequence: p.first_sequence,
        firstTimestamp: p.first_timestamp,
    });
}
export class BrowserVideoEpochs {
    root;
    binding;
    allocation;
    receiver;
    publisher;
    receiveActive = false;
    publishActive = false;
    receiveGate = true;
    publishGate = true;
    version = 0;
    stopped = false;
    pending = [];
    hint;
    feedbackSequence;
    sendSequence = 0;
    lastRequest = -Infinity;
    senderSsrc = crypto.getRandomValues(new Uint32Array(1))[0] || 1;
    constructor(root, binding, allocation) {
        this.root = root;
        this.binding = binding;
        this.allocation = allocation;
    }
    get pendingPackets() {
        return this.pending.length;
    }
    matches(n) {
        return (same(n.root, this.root) &&
            n.stream === this.binding.streamId &&
            n.handle === this.binding.handle &&
            same(n.allocation, this.allocation) &&
            (n.role === "receiver"
                ? this.binding.direction !== "sendonly"
                : this.binding.direction !== "recvonly"));
    }
    /** Retain revision high water even when a gate is disabled. Replayed signed
     * notices cannot reactivate old history after hold/unsubscribe. */
    observe(n) {
        if (this.stopped || !this.matches(n))
            return false;
        const old = n.role === "receiver" ? this.receiver : this.publisher;
        if (old && n.revision <= old.revision)
            return false;
        if (n.role === "receiver") {
            this.receiver = n;
            this.receiveActive = true;
        }
        else {
            this.publisher = n;
            this.publishActive = true;
            this.hint = undefined;
        }
        return true;
    }
    invalidateRequests() {
        this.version++;
        this.hint = undefined;
    }
    unselect() {
        this.invalidateRequests();
        this.receiveActive = false;
        this.pending = [];
    }
    gates(send, receive) {
        this.invalidateRequests();
        this.publishGate = send;
        this.receiveGate = receive;
        if (!send)
            this.publishActive = false;
        if (!receive) {
            this.receiveActive = false;
            this.pending = [];
        }
    }
    admits(value) {
        const n = this.receiver, p = value.packet;
        return (!this.stopped &&
            this.receiveGate &&
            this.receiveActive &&
            !!n &&
            value.epoch === n.epoch &&
            p.ssrc === n.ssrc &&
            (p.datagramSequence - n.firstSequence) >>> 0 < 0x80000000 &&
            (p.datagramSequence !== n.firstSequence ||
                p.timestamp === n.firstTimestamp));
    }
    stale(value) {
        const n = this.receiver;
        return (!!n &&
            (value.epoch === n.epoch ||
                (value.packet.datagramSequence - n.firstSequence) >>> 0 >=
                    0x80000000) &&
            !this.admits(value));
    }
    packet(value, arrived, now, capacity = true) {
        this.expire(now);
        if (this.stopped ||
            !this.receiveGate ||
            this.binding.direction === "sendonly" ||
            value.packet.handle !== this.binding.handle ||
            value.packet.payloadType !== this.binding.payloadType ||
            now - arrived >= 200 ||
            now < arrived)
            return undefined;
        if (this.admits(value))
            return value.packet;
        if (!this.stale(value) && capacity && this.pending.length < 32)
            this.pending.push({ value, arrived });
        return undefined;
    }
    drain(now) {
        this.expire(now);
        const accepted = [], retained = [];
        for (const item of this.pending) {
            if (this.admits(item.value))
                accepted.push({
                    packet: item.value.packet,
                    arrived: item.arrived,
                });
            else if (!this.stale(item.value))
                retained.push(item);
        }
        this.pending = retained;
        return accepted;
    }
    expire(now) {
        this.pending = this.pending.filter((p) => now >= p.arrived && now - p.arrived < 200);
    }
    feedback(value, arrived) {
        const n = this.publisher;
        if (this.stopped ||
            !this.publishGate ||
            !this.publishActive ||
            !n ||
            this.binding.direction === "recvonly" ||
            value.handle !== this.binding.handle ||
            value.epoch !== n.epoch ||
            value.mediaSsrc !== n.ssrc ||
            (this.feedbackSequence !== undefined &&
                !((value.datagramSequence - this.feedbackSequence) >>> 0 >
                    0 &&
                    (value.datagramSequence - this.feedbackSequence) >>> 0 <
                        0x80000000)))
            return;
        this.feedbackSequence = value.datagramSequence;
        this.hint = { value, arrived, notice: n, version: this.version };
    }
    /** Consumed once immediately before encoding. Age starts at physical read,
     * never at notice arrival, dequeue, encoder acceptance or retry. */
    takeFeedback(now, ssrc) {
        const hint = this.hint;
        this.hint = undefined;
        return (!!hint &&
            !this.stopped &&
            this.publishGate &&
            this.publishActive &&
            this.publisher === hint.notice &&
            hint.version === this.version &&
            hint.value.mediaSsrc === ssrc &&
            now >= hint.arrived &&
            now - hint.arrived < 200);
    }
    request(now) {
        if (this.stopped ||
            !this.receiveGate ||
            !this.receiveActive ||
            !this.receiver ||
            this.binding.direction === "sendonly" ||
            now - this.lastRequest < 100)
            return undefined;
        this.lastRequest = now;
        const sequence = this.sendSequence;
        this.sendSequence = (sequence + 1) >>> 0;
        return {
            notice: this.receiver,
            version: this.version,
            started: now,
            bytes: encodeNativeVideoFeedback({
                handle: this.binding.handle,
                datagramSequence: sequence,
                senderSsrc: this.senderSsrc,
                mediaSsrc: this.receiver.ssrc,
                epoch: this.receiver.epoch,
            }),
        };
    }
    current(write, now) {
        return (!this.stopped &&
            this.receiveGate &&
            this.receiveActive &&
            this.receiver === write.notice &&
            this.version === write.version &&
            now >= write.started &&
            now - write.started < 200);
    }
    close() {
        this.stopped = true;
        this.invalidateRequests();
        this.pending = [];
        this.receiver = this.publisher = undefined;
    }
}
