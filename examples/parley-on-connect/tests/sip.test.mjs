import test from "node:test";
import assert from "node:assert/strict";
import dgram from "node:dgram";
import { SipFixture } from "../fixtures/sip.mjs";
import { marker, rtp, unpack } from "../fixtures/pcm.mjs";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const until = async (fn) => { const deadline = Date.now() + 2000; while (!fn()) { if (Date.now() > deadline) throw Error("UDP fixture deadline"); await sleep(5); } };
async function socket(t) {
    const s = dgram.createSocket("udp4"); await new Promise((r) => s.bind(0, "127.0.0.1", r));
    t.after(() => new Promise((r) => s.close(r))); return s;
}
function request(method, fixture, client, id, body = "", sequence = 1) {
    return Buffer.from(`${method} ${fixture.uri} SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:${client.address().port};branch=z9hG4bK-${id}\r\nFrom: <sip:caller@127.0.0.1>;tag=caller\r\nTo: <${fixture.uri}>\r\nCall-ID: ${id}\r\nCSeq: ${sequence} ${method}\r\nContent-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
}
const send = (socket, packet, port) => new Promise((resolve, reject) => socket.send(packet, port, "127.0.0.1", (e) => e ? reject(e) : resolve()));
const sdp = (media, type = "0 101") => `v=0\r\nc=IN IP4 127.0.0.1\r\nm=audio ${media.address().port} RTP/AVP ${type}\r\na=rtpmap:101 telephone-event/8000\r\n`;
async function scene(t, options = {}, id = "original-call") {
    const fixture = await new SipFixture(options).start(); t.after(() => fixture.close());
    const client = await socket(t), media = await socket(t); const responses = [], packets = [];
    client.on("message", (b) => responses.push(b.toString())); media.on("message", (b) => packets.push(unpack(b)));
    const invite = request("INVITE", fixture, client, id, sdp(media));
    return { fixture, client, media, responses, packets, invite, id };
}
const ok = (s) => s.responses.some((b) => b.startsWith("SIP/2.0 200"));
async function connect(s) {
    await send(s.client, s.invite, s.fixture.port); await until(() => ok(s));
    const response = s.responses.find((b) => b.startsWith("SIP/2.0 200"));
    s.mediaPort = Number(response.match(/m=audio (\d+)/)[1]);
    await send(s.client, request("ACK", s.fixture, s.client, s.id), s.fixture.port);
    await until(() => s.fixture.dialogs.get(s.id).acknowledged);
}
// Independent G.711 decode plus complex tone correlation; no fixture decode reuse.
function magnitude(bytes, hz) {
    let real = 0, imaginary = 0;
    for (let i = 0; i < bytes.length; i++) {
        const u = 255 - bytes[i], exponent = (u >>> 4) & 7;
        const linear = (((u & 15) * 8 + 132) * 2 ** exponent - 132) * (u & 128 ? -1 : 1);
        real += linear * Math.cos(2 * Math.PI * hz * i / 8000); imaginary += linear * Math.sin(2 * Math.PI * hz * i / 8000);
    }
    return Math.hypot(real, imaginary) / bytes.length;
}
test("duplicate UDP invitations allocate one SIP/RTP leg; decoded marker, pause gap and terminal drain", async (t) => {
    const s = await scene(t, { markerHz: 900 });
    await Promise.all([send(s.client, s.invite, s.fixture.port), send(s.client, s.invite, s.fixture.port)]);
    await until(() => ok(s)); assert.equal(s.fixture.effects, 1); assert.equal(s.fixture.invites, 2);
    await connect(s); await until(() => s.packets.length >= 3);
    const packet = s.packets.at(-1); assert.equal(packet.payload.length, 160);
    assert.ok(magnitude(packet.payload, 900) > 4000); assert.ok(magnitude(packet.payload, 300) < 100);
    const before = packet.timestamp; s.fixture.pause(s.id, true); await sleep(100);
    s.fixture.pause(s.id, false); const n = s.packets.length; await until(() => s.packets.length > n);
    assert.ok(((s.packets.at(-1).timestamp - before) >>> 0) >= 800);
    await send(s.media, rtp(marker(300)), s.mediaPort); await until(() => s.fixture.dialogs.get(s.id).received === 1);
    await send(s.client, request("BYE", s.fixture, s.client, s.id, "", 2), s.fixture.port);
    await until(() => s.fixture.dialogs.get(s.id).stopped); await sleep(30);
    const ended = s.packets.length; await sleep(60); assert.equal(s.packets.length, ended);
    await send(s.client, s.invite, s.fixture.port); await until(() => s.responses.some((b) => b.startsWith("SIP/2.0 481")));
    assert.equal(s.fixture.effects, 1); await s.fixture.close(); await s.fixture.close(); assert.equal(s.fixture.pendingSends, 0);
});
test("phone emits configured DTMF only after real prompt frames from the offered peer", async (t) => {
    const s = await scene(t, { waitForPrompt: true, confirmationDigit: "1", markerHz: 300 }); await connect(s);
    const impostor = await socket(t); await send(impostor, rtp(marker(500)), s.mediaPort); await sleep(40);
    assert.equal(s.packets.length, 0); assert.equal(s.fixture.dialogs.get(s.id).received, 0);
    for (let i = 0; i < 2; i++) await send(s.media, rtp(marker(500), { sequence: i }), s.mediaPort);
    await sleep(40); assert.equal(s.packets.length, 0);
    await send(s.media, rtp(marker(500), { sequence: 3 }), s.mediaPort);
    await until(() => s.packets.filter((p) => p.type === 101 && (p.payload[1] & 128)).length === 3);
    const events = s.packets.filter((p) => p.type === 101); assert.equal(events.length, 4);
    assert.ok(events.every((p) => p.payload[0] === 1)); assert.equal(new Set(events.map((p) => p.timestamp)).size, 1);
    await until(() => s.packets.some((p) => p.type === 0)); assert.ok(magnitude(s.packets.find((p) => p.type === 0).payload, 300) > 4000);
});
test("wrong-digit and no-confirmation fixture profiles remain distinguishable", async (t) => {
    for (const digit of ["9", null]) {
        const s = await scene(t, { confirmationDigit: digit }, `digit-${digit}`); await connect(s);
        for (let i = 0; i < 3; i++) await send(s.media, rtp(marker(500)), s.mediaPort);
        await until(() => s.fixture.dialogs.get(s.id).received === 3); await sleep(60);
        const events = s.packets.filter((p) => p.type === 101);
        if (digit === null) assert.equal(events.length, 0); else assert.ok(events.length === 4 && events.every((p) => p.payload[0] === 9));
    }
});
test("no-answer CANCEL terminates original invitation; another SIP peer cannot control it", async (t) => {
    const s = await scene(t, { autoAnswer: false }); await send(s.client, s.invite, s.fixture.port);
    await until(() => s.responses.some((b) => b.startsWith("SIP/2.0 180")));
    const other = await socket(t); await send(other, request("CANCEL", s.fixture, other, s.id), s.fixture.port); await sleep(25);
    assert.equal(s.fixture.dialogs.get(s.id).stopped, false);
    await send(s.client, request("CANCEL", s.fixture, s.client, s.id), s.fixture.port);
    await until(() => s.responses.some((b) => b.startsWith("SIP/2.0 487")));
    assert.equal(s.packets.length, 0); assert.equal(s.fixture.effects, 1);
});
test("incompatible codec refuses before allocating any RTP worker", async (t) => {
    const s = await scene(t); await send(s.client, request("INVITE", s.fixture, s.client, s.id, sdp(s.media, "8")), s.fixture.port);
    await until(() => s.responses.some((b) => b.startsWith("SIP/2.0 488")));
    assert.equal(s.fixture.effects, 0); assert.equal(s.fixture.dialogs.size, 0);
});
test("all speaker markers decode distinctly and malformed RTP refuses", () => {
    for (const hz of [300, 500, 700, 900, 1100]) { const bytes = marker(hz); assert.ok(magnitude(bytes, hz) > 4000); for (const other of [300, 500, 700, 900, 1100].filter((n) => n !== hz)) assert.ok(magnitude(bytes, other) < 150); }
    for (const packet of [Buffer.alloc(3), Buffer.alloc(12), Buffer.from([0x90, ...Array(11).fill(0)]), Buffer.from([0xa0, ...Array(11).fill(0)])]) assert.throws(() => unpack(packet));
});
