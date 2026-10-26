// Synthetic speaker markers, G.711 mu-law, mono 8 kHz / 160 samples per frame.
export function decode(byte) {
    const value = (~byte) & 255;
    const magnitude = (((value & 15) * 8 + 132) << ((value >> 4) & 7)) - 132;
    return value & 128 ? -magnitude : magnitude;
}
export function encode(sample) {
    const sign = sample < 0 ? 128 : 0;
    const value = Math.min(32635, Math.abs(Math.round(sample))) + 132;
    let exponent = 7;
    for (let mask = 0x4000; exponent > 0 && !(value & mask); mask >>= 1) exponent--;
    return (~(sign | (exponent << 4) | ((value >> (exponent + 3)) & 15))) & 255;
}
export function marker(frequency, timestamp = 0) {
    if (!Number.isInteger(frequency) || frequency < 100 || frequency > 3000) throw Error("fixture marker frequency");
    return Buffer.from(Array.from({ length: 160 }, (_, i) => encode(9000 * Math.sin(2 * Math.PI * frequency * (timestamp + i) / 8000))));
}
export function rtp(payload, { type = 0, sequence = 1, timestamp = 0, ssrc = 1 } = {}) {
    const header = Buffer.alloc(12);
    header[0] = 0x80; header[1] = type;
    header.writeUInt16BE(sequence & 65535, 2);
    header.writeUInt32BE(timestamp >>> 0, 4); header.writeUInt32BE(ssrc >>> 0, 8);
    return Buffer.concat([header, payload]);
}
export function unpack(packet) {
    if (packet.length < 12 || (packet[0] >> 6) !== 2) throw Error("RTP header");
    let start = 12 + 4 * (packet[0] & 15);
    if (packet[0] & 16) { if (packet.length < start + 4) throw Error("RTP extension"); start += 4 + 4 * packet.readUInt16BE(start + 2); }
    const padding = packet[0] & 32 ? packet.at(-1) : 0;
    if ((packet[0] & 32) && padding === 0) throw Error("RTP padding");
    const end = packet.length - padding;
    if (start > end || end > packet.length) throw Error("RTP bounds");
    return { type: packet[1] & 127, sequence: packet.readUInt16BE(2), timestamp: packet.readUInt32BE(4), ssrc: packet.readUInt32BE(8), payload: packet.subarray(start, end) };
}
