/** App-owned, finite PCMU/VP8 file bridge. No devices, codec child or control IO. */
import { open, writeFile } from "node:fs/promises";
import { performance } from "node:perf_hooks";
import { SdkRtpPacket } from "@rudeless/connect";
export const MAX_FRAME = 262144,
    MAX_FILE = 4 * 1024 * 1024,
    MAX_RTP = 1180;
const invalid = () => {
    throw new Error("Unsupported or incomplete PCMU/VP8 example input");
};
async function bounded(path, maximum) {
    const file = await open(path, "r");
    try {
        const bytes = Buffer.alloc(maximum + 1);
        let used = 0;
        while (used < bytes.length) {
            const r = await file.read(bytes, used, bytes.length - used, null);
            if (!r.bytesRead) break;
            used += r.bytesRead;
        }
        if (used > maximum) invalid();
        return bytes.subarray(0, used);
    } finally {
        await file.close();
    }
}
export function parseIvf(input) {
    const b = Buffer.from(input);
    if (
        b.length < 32 ||
        b.length > MAX_FILE ||
        !b.subarray(0, 4).equals(Buffer.from("DKIF")) ||
        b.readUInt16LE(4) !== 0 ||
        b.readUInt16LE(6) !== 32 ||
        !b.subarray(8, 12).equals(Buffer.from("VP80")) ||
        b.readUInt16LE(12) !== 640 ||
        b.readUInt16LE(14) !== 360 ||
        b.readUInt32LE(28) !== 0
    )
        invalid();
    const rate = BigInt(b.readUInt32LE(16)),
        scale = BigInt(b.readUInt32LE(20));
    if (rate < 1n || scale < 1n) invalid();
    let offset = 32,
        last = -1;
    const frames = [];
    while (offset < b.length) {
        if (offset + 12 > b.length || frames.length >= 90) invalid();
        const size = b.readUInt32LE(offset),
            pts = b.readBigUInt64LE(offset + 4),
            ticks = (pts * 90000n * scale) / rate;
        offset += 12;
        if (
            !size ||
            size > MAX_FRAME ||
            offset + size > b.length ||
            ticks > 270000n ||
            Number(ticks) <= last
        )
            invalid();
        last = Number(ticks);
        frames.push({
            timestamp: last,
            encoded: Buffer.from(b.subarray(offset, offset + size)),
        });
        offset += size;
    }
    if (
        !frames.length ||
        (b.readUInt32LE(24) !== 0 && b.readUInt32LE(24) !== frames.length)
    )
        invalid();
    return frames;
}
export function ivfHeader() {
    const b = Buffer.alloc(32);
    b.write("DKIF");
    b.writeUInt16LE(32, 6);
    b.write("VP80", 8);
    b.writeUInt16LE(640, 12);
    b.writeUInt16LE(360, 14);
    b.writeUInt32LE(90000, 16);
    b.writeUInt32LE(1, 20);
    return b;
}
export function frameRecord(frame, first) {
    const b = Buffer.alloc(12 + frame.encoded.length);
    b.writeUInt32LE(frame.encoded.length);
    b.writeBigUInt64LE(BigInt((frame.timestamp - first) >>> 0), 4);
    frame.encoded.copy(b, 12);
    return b;
}
export function rtp(pt, sequence, timestamp, ssrc, payload, marker = false) {
    if (
        !Number.isInteger(sequence) ||
        sequence < 0 ||
        sequence > 65535 ||
        !Number.isInteger(timestamp) ||
        timestamp < 0 ||
        timestamp > 0xffffffff ||
        !Number.isInteger(ssrc) ||
        ssrc < 1 ||
        ssrc > 0xffffffff ||
        !(pt === 0 || pt === 96) ||
        !payload.length ||
        payload.length + 12 > MAX_RTP
    )
        invalid();
    const b = Buffer.alloc(payload.length + 12);
    b[0] = 0x80;
    b[1] = pt | (marker ? 0x80 : 0);
    b.writeUInt16BE(sequence, 2);
    b.writeUInt32BE(timestamp, 4);
    b.writeUInt32BE(ssrc, 8);
    b.set(payload, 12);
    return b;
}
export function packetizeVp8(encoded, timestamp, sequence, ssrc) {
    if (!encoded.length || encoded.length > MAX_FRAME) invalid();
    const packets = [];
    for (let offset = 0; offset < encoded.length; offset += MAX_RTP - 13) {
        const end = Math.min(offset + MAX_RTP - 13, encoded.length),
            payload = Buffer.concat([
                Buffer.from([offset === 0 ? 0x10 : 0]),
                encoded.subarray(offset, end),
            ]);
        packets.push(
            rtp(
                96,
                (sequence + packets.length) & 65535,
                timestamp,
                ssrc,
                payload,
                end === encoded.length,
            ),
        );
    }
    return packets;
}
export class Vp8Assembler {
    #pending;
    #source;
    #last;
    #needsKey = true;
    reset() {
        this.#pending = undefined;
        this.#needsKey = true;
    }
    push(packet, now = performance.now()) {
        if (
            !(packet instanceof SdkRtpPacket) ||
            packet.bytes.length > MAX_RTP ||
            packet.header.payloadType !== 96
        )
            invalid();
        const h = packet.header,
            p = packet.payload;
        if (p.length < 2 || ![0, 0x10].includes(p[0])) {
            this.reset();
            invalid();
        }
        if (this.#source !== undefined && h.ssrc !== this.#source) {
            this.reset();
            invalid();
        }
        this.#source = h.ssrc;
        if (this.#pending && now - this.#pending.started > 200) this.reset();
        if (p[0] === 0x10) {
            if (this.#pending) this.reset();
            this.#pending = {
                timestamp: h.timestamp,
                next: h.sequenceNumber,
                started: now,
                parts: [],
                bytes: 0,
            };
        }
        const f = this.#pending;
        if (!f) return undefined;
        if (f.timestamp !== h.timestamp || f.next !== h.sequenceNumber) {
            this.reset();
            return undefined;
        }
        f.next = (f.next + 1) & 65535;
        f.bytes += p.length - 1;
        if (f.bytes > MAX_FRAME) {
            this.reset();
            invalid();
        }
        f.parts.push(Buffer.from(p.subarray(1)));
        if (!h.marker) return undefined;
        this.#pending = undefined;
        const encoded = Buffer.concat(f.parts),
            key = (encoded[0] & 1) === 0;
        if (
            key &&
            (encoded.length < 10 ||
                encoded[3] !== 0x9d ||
                encoded[4] !== 1 ||
                encoded[5] !== 0x2a ||
                (encoded.readUInt16LE(6) & 0x3fff) !== 640 ||
                (encoded.readUInt16LE(8) & 0x3fff) !== 360)
        ) {
            this.reset();
            invalid();
        }
        if (this.#last === h.timestamp || (this.#needsKey && !key))
            return undefined;
        this.#needsKey = false;
        this.#last = h.timestamp;
        return { timestamp: h.timestamp, encoded };
    }
}
export async function loadMedia(audioPath, videoPath) {
    const audio = await bounded(audioPath, 24000);
    if (!audio.length || audio.length % 160) invalid();
    return { audio, frames: parseIvf(await bounded(videoPath, MAX_FILE)) };
}
/** All work is joined before returning. Caller owns and closes original Client. */
export async function exchangeMedia(
    sendAudio,
    sendVideo,
    receiveAudio,
    receiveVideo,
    input,
    outputPrefix,
    ssrc,
) {
    const deadline = performance.now() + 4000,
        options = () => {
            const timeoutMs = Math.ceil(deadline - performance.now());
            if (timeoutMs < 1) throw new Error("Media example deadline");
            return { timeoutMs };
        };
    let audioSequence = 0,
        videoSequence = 0,
        datagram = 0;
    const sending = (async () => {
        const start = performance.now();
        let ai = 0,
            vi = 0;
        while (ai < input.audio.length / 160 || vi < input.frames.length) {
            const at = ai < input.audio.length / 160 ? ai * 20 : Infinity,
                vt =
                    vi < input.frames.length
                        ? input.frames[vi].timestamp / 90
                        : Infinity;
            const wait = Math.min(at, vt) - (performance.now() - start);
            if (wait > 0) await new Promise((r) => setTimeout(r, wait));
            if (at <= vt) {
                const b = rtp(
                    0,
                    audioSequence++ & 65535,
                    ai * 160,
                    ssrc,
                    input.audio.subarray(ai * 160, ++ai * 160),
                );
                await sendAudio.sendRtp(
                    new SdkRtpPacket(datagram++, b),
                    options(),
                );
            } else {
                const f = input.frames[vi++];
                for (const b of packetizeVp8(
                    f.encoded,
                    f.timestamp,
                    videoSequence,
                    ssrc + 1,
                )) {
                    await sendVideo.sendRtp(
                        new SdkRtpPacket(datagram++, b),
                        options(),
                    );
                    videoSequence = (videoSequence + 1) & 65535;
                }
            }
        }
    })();
    const audio = (async () => {
        const parts = [];
        let previous;
        while (parts.length < input.audio.length / 160) {
            const p = await receiveAudio.receivePacket(options());
            if (
                p.kind !== "rtp" ||
                p.header.payloadType !== 0 ||
                p.payload.length !== 160 ||
                (previous !== undefined &&
                    p.header.sequenceNumber !== ((previous + 1) & 65535))
            )
                invalid();
            previous = p.header.sequenceNumber;
            parts.push(Buffer.from(p.payload));
        }
        return Buffer.concat(parts);
    })();
    const video = (async () => {
        const assembler = new Vp8Assembler(),
            frames = [];
        let size = 32;
        while (frames.length < input.frames.length) {
            const p = await receiveVideo.receivePacket(options());
            if (p.kind !== "rtp") {
                assembler.reset();
                invalid();
            }
            const f = assembler.push(p);
            if (f) {
                size += 12 + f.encoded.length;
                if (size > MAX_FILE) invalid();
                frames.push(f);
            }
        }
        return frames;
    })();
    const results = await Promise.allSettled([sending, audio, video]);
    for (const r of results) if (r.status === "rejected") throw r.reason;
    const capturedAudio = results[1].value,
        frames = results[2].value;
    await writeFile(outputPrefix + ".mulaw", capturedAudio, {
        flag: "wx",
        mode: 0o600,
    });
    await writeFile(
        outputPrefix + ".ivf",
        Buffer.concat([
            ivfHeader(),
            ...frames.map((f) => frameRecord(f, frames[0].timestamp)),
        ]),
        { flag: "wx", mode: 0o600 },
    );
    return {
        audioBytes: capturedAudio.length,
        videoFrames: frames.length,
        sentRtpPackets: datagram,
    };
}
