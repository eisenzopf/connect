import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
export const MEDIA_LIMITS = Object.freeze({
    rtpBytes: 1180,
    frameBytes: 256 * 1024,
    assemblyMs: 200,
    frames: 8,
    fragments: 64,
    audioPackets: 10,
    videoPackets: 64,
    sendBytes: 76032,
});
const invalid = () => {
    throw new ConnectError("protocol", "Invalid UCTP/RTP media packet");
};
/** Complete RTP v2 parsing, including bounded RFC 8285 extension elements. */
export function decodeRtpDatagram(packet) {
    if (packet.length < 20 || packet.length > MEDIA_LIMITS.rtpBytes + 8)
        return invalid();
    const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
    const handle = view.getUint16(2);
    if (packet[0] !== 1 || !handle || packet[8] >>> 6 !== 2)
        return invalid();
    let offset = 20 + (packet[8] & 15) * 4;
    if (offset > packet.length)
        return invalid();
    const csrc = [];
    for (let i = 20; i < offset; i += 4)
        csrc.push(view.getUint32(i));
    let extensionProfile;
    if (packet[8] & 16) {
        if (offset + 4 > packet.length)
            return invalid();
        extensionProfile = view.getUint16(offset);
        const length = view.getUint16(offset + 2) * 4;
        offset += 4;
        const end = offset + length;
        if (end > packet.length)
            return invalid();
        if (extensionProfile === 0xbede) {
            for (let i = offset; i < end;) {
                const head = packet[i++], id = head >>> 4;
                if (id === 15 || (id === 0 && (head & 15) !== 0))
                    break;
                if (id === 0)
                    continue;
                i += (head & 15) + 1;
                if (i > end)
                    return invalid();
            }
        }
        else if ((extensionProfile & 0xfff0) === 0x1000) {
            for (let i = offset; i < end;) {
                if (!packet[i++])
                    continue;
                if (i >= end)
                    return invalid();
                i += 1 + packet[i];
                if (i > end)
                    return invalid();
            }
        }
        offset = end;
    }
    let end = packet.length, padding = 0;
    if (packet[8] & 32) {
        padding = packet[end - 1];
        if (!padding || padding > end - offset)
            return invalid();
        end -= padding;
    }
    return {
        flags: packet[1],
        handle,
        datagramSequence: view.getUint32(4),
        payloadType: packet[9] & 127,
        marker: !!(packet[9] & 128),
        sequence: view.getUint16(10),
        timestamp: view.getUint32(12),
        ssrc: view.getUint32(16),
        csrc,
        extensionProfile,
        padding,
        payload: packet.subarray(offset, end),
    };
}
/** Local routing remembers retired physical handles. Server authorization is authoritative. */
export class MediaBindings {
    used = new Set();
    active = new Map();
    register(binding) {
        if (!Number.isInteger(binding.handle) ||
            binding.handle < 1 ||
            binding.handle > 65535 ||
            !identifier(binding.streamId) ||
            !identifier(binding.connectionId) ||
            !identifier(binding.sessionId) ||
            !Number.isSafeInteger(binding.generation) ||
            binding.generation < 1 ||
            !Number.isInteger(binding.payloadType) ||
            binding.payloadType < 0 ||
            binding.payloadType > 127 ||
            !["sendrecv", "sendonly", "recvonly"].includes(binding.direction) ||
            !((binding.kind === "audio" && binding.codec === "opus") ||
                (["video", "screen"].includes(binding.kind) &&
                    binding.codec === "vp8")))
            throw new ConnectError("unsupported-media-profile", "Use a negotiated Opus/VP8 binding");
        if (this.used.has(binding.handle) || this.active.size >= 256)
            throw new ConnectError("protocol", "Physical media handle reused or binding limit exceeded");
        this.used.add(binding.handle);
        this.active.set(binding.handle, Object.freeze({ ...binding }));
    }
    retire(handle) {
        this.active.delete(handle);
    }
    retireConnection(id) {
        for (const [handle, binding] of this.active)
            if (binding.connectionId === id)
                this.active.delete(handle);
    }
    require(handle, direction, payloadType) {
        const binding = this.active.get(handle);
        if (!binding ||
            (direction === "send"
                ? binding.direction === "recvonly"
                : binding.direction === "sendonly") ||
            (payloadType !== undefined && payloadType !== binding.payloadType))
            throw new ConnectError("protocol", "Unknown, retired or incompatible media binding");
        return binding;
    }
    clear() {
        this.active.clear();
    }
}
/** Each sender has its own RTP SSRC/sequence and physical datagram sequence. */
export class RtpSender {
    binding;
    maxDatagramBytes;
    sequence;
    datagramSequence = 0;
    ssrc;
    packets = 0;
    octets = 0;
    constructor(binding, maxDatagramBytes = MEDIA_LIMITS.rtpBytes + 8) {
        this.binding = binding;
        this.maxDatagramBytes = maxDatagramBytes;
        const checked = new MediaBindings();
        checked.register(binding);
        checked.require(binding.handle, "send");
        this.binding = Object.freeze({ ...binding });
        const random = crypto.getRandomValues(new Uint32Array(2));
        this.sequence = random[0] & 65535;
        this.ssrc = random[1];
        if (!Number.isInteger(maxDatagramBytes) ||
            maxDatagramBytes < 64 ||
            maxDatagramBytes > MEDIA_LIMITS.rtpBytes + 8)
            throw new ConnectError("unsupported-media-profile", "Invalid negotiated datagram MTU");
    }
    /** Record only a successful local RTP writer submission, never packetization. */
    submitted(bytes) {
        const packet = decodeRtpDatagram(bytes);
        if (packet.handle !== this.binding.handle ||
            packet.ssrc !== this.ssrc ||
            packet.payloadType !== this.binding.payloadType)
            return invalid();
        this.packets = (this.packets + 1) >>> 0;
        this.octets = (this.octets + packet.payload.length) >>> 0;
        return { ssrc: this.ssrc, packets: this.packets, octets: this.octets };
    }
    nextDatagramSequence() {
        return this.datagramSequence++ >>> 0;
    }
    packetize(encoded, timestamp) {
        if (!encoded.length ||
            encoded.length > MEDIA_LIMITS.frameBytes ||
            !Number.isInteger(timestamp) ||
            timestamp < 0 ||
            timestamp > 0xffffffff)
            return invalid();
        const video = this.binding.codec === "vp8";
        const capacity = this.maxDatagramBytes - 20 - (video ? 1 : 0);
        const count = Math.ceil(encoded.length / capacity);
        if ((!video && count !== 1) || count > MEDIA_LIMITS.fragments)
            return invalid();
        const packets = [];
        for (let i = 0; i < count; i++) {
            const payload = encoded.subarray(i * capacity, (i + 1) * capacity);
            const packet = new Uint8Array(20 + (video ? 1 : 0) + payload.length), v = new DataView(packet.buffer);
            packet[0] = 1;
            v.setUint16(2, this.binding.handle);
            v.setUint32(4, this.datagramSequence++ >>> 0);
            packet[8] = 0x80;
            packet[9] =
                this.binding.payloadType | (video && i === count - 1 ? 128 : 0);
            v.setUint16(10, this.sequence++ & 65535);
            v.setUint32(12, timestamp);
            v.setUint32(16, this.ssrc);
            if (video)
                packet[20] = i === 0 ? 0x10 : 0;
            packet.set(payload, video ? 21 : 20);
            packets.push(packet);
        }
        return packets;
    }
}
/** Loss never produces a partial decoded frame; expired assemblies release all bytes. */
export class Vp8Assembler {
    pending = new Map();
    completed = new Map();
    fragments = 0;
    remove(key) {
        const frame = this.pending.get(key);
        if (frame)
            this.fragments -= frame.fragments.size;
        this.pending.delete(key);
    }
    expire(now = performance.now()) {
        let expired = 0;
        for (const [key, frame] of this.pending)
            if (now - frame.created >= MEDIA_LIMITS.assemblyMs) {
                this.remove(key);
                expired++;
            }
        for (const [key, time] of this.completed)
            if (now - time >= MEDIA_LIMITS.assemblyMs)
                this.completed.delete(key);
        return expired;
    }
    get queuedFrames() {
        return this.pending.size;
    }
    get queuedFragments() {
        return this.fragments;
    }
    clear() {
        this.pending.clear();
        this.completed.clear();
        this.fragments = 0;
    }
    push(packet, now = performance.now()) {
        this.expire(now);
        const p = packet.payload;
        if (p.length < 2)
            return invalid();
        let offset = 1;
        // RFC 7741 X/I/L/T/K fields are bounded before reading codec bytes.
        if (p[0] & 128) {
            if (offset >= p.length)
                return invalid();
            const extension = p[offset++];
            if (extension & 128) {
                if (offset >= p.length)
                    return invalid();
                const picture = p[offset++];
                if (picture & 128)
                    offset++;
            }
            if (extension & 64)
                offset++;
            if (extension & 48)
                offset++;
        }
        if (offset >= p.length || (p[0] & 15) !== 0)
            return invalid();
        const key = packet.ssrc + ":" + packet.timestamp;
        if (this.completed.has(key))
            return undefined;
        let frame = this.pending.get(key);
        if (!frame) {
            while (this.pending.size >= MEDIA_LIMITS.frames ||
                this.fragments >= MEDIA_LIMITS.fragments)
                this.remove(this.pending.keys().next().value);
            frame = { created: now, fragments: new Map(), bytes: 0 };
            this.pending.set(key, frame);
        }
        if (frame.fragments.has(packet.sequence))
            return undefined;
        if (this.fragments >= MEDIA_LIMITS.fragments ||
            frame.bytes + p.length - offset > MEDIA_LIMITS.frameBytes) {
            this.remove(key);
            return undefined;
        }
        frame.fragments.set(packet.sequence, p.slice(offset));
        this.fragments++;
        frame.bytes += p.length - offset;
        if (p[0] & 16) {
            if (frame.start !== undefined && frame.start !== packet.sequence) {
                this.remove(key);
                return undefined;
            }
            frame.start = packet.sequence;
        }
        if (packet.marker) {
            if (frame.end !== undefined && frame.end !== packet.sequence) {
                this.remove(key);
                return undefined;
            }
            frame.end = packet.sequence;
        }
        if (frame.start === undefined || frame.end === undefined)
            return undefined;
        const count = ((frame.end - frame.start + 65536) & 65535) + 1;
        if (count > MEDIA_LIMITS.fragments) {
            this.remove(key);
            return undefined;
        }
        for (let i = 0; i < count; i++)
            if (!frame.fragments.has((frame.start + i) & 65535))
                return undefined;
        if (frame.fragments.size !== count) {
            this.remove(key);
            return undefined;
        }
        const output = new Uint8Array(frame.bytes);
        let at = 0;
        for (let i = 0; i < count; i++) {
            const bytes = frame.fragments.get((frame.start + i) & 65535);
            output.set(bytes, at);
            at += bytes.length;
        }
        this.remove(key);
        if (this.completed.size >= 64)
            this.completed.delete(this.completed.keys().next().value);
        this.completed.set(key, now);
        return {
            bytes: output,
            timestamp: packet.timestamp,
            ssrc: packet.ssrc,
            key: (output[0] & 1) === 0,
            firstSequence: frame.start,
            lastSequence: frame.end,
        };
    }
}
/** Drop obsolete media instead of allowing control backpressure to accumulate speech. */
export class MediaSendQueue {
    packets = [];
    size = 0;
    droppedPackets = 0;
    get queuedBytes() {
        return this.size;
    }
    get queuedPackets() {
        return this.packets.length;
    }
    clear() {
        this.packets = [];
        this.size = 0;
    }
    shift() {
        const p = this.packets.shift();
        if (p)
            this.size -= p.bytes.length;
        return p?.bytes;
    }
    push(packets, audio, now = performance.now()) {
        this.expire(now);
        const bytes = packets.reduce((n, p) => n + p.length, 0);
        const limit = audio
            ? MEDIA_LIMITS.audioPackets
            : MEDIA_LIMITS.videoPackets;
        if (!packets.length ||
            packets.length > limit ||
            bytes > MEDIA_LIMITS.sendBytes ||
            packets.some((p) => p.length > MEDIA_LIMITS.rtpBytes + 8))
            return false;
        while (this.size + bytes > MEDIA_LIMITS.sendBytes ||
            this.packets.filter((p) => p.audio === audio).length +
                packets.length >
                limit) {
            this.shift();
            this.droppedPackets++;
        }
        for (const packet of packets)
            this.packets.push({ bytes: packet.slice(), audio, created: now });
        this.size += bytes;
        return true;
    }
    expire(now) {
        while (this.packets[0] &&
            now - this.packets[0].created >= MEDIA_LIMITS.assemblyMs) {
            this.shift();
            this.droppedPackets++;
        }
    }
    pop(now = performance.now()) {
        this.expire(now);
        return this.shift();
    }
}
