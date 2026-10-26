import dgram from "node:dgram";
import { performance } from "node:perf_hooks";
import { marker, rtp, unpack, decode } from "./pcm.mjs";
const host = "127.0.0.1";
const bind = async (socket) => { await new Promise((resolve, reject) => { socket.once("error", reject); socket.bind(0, host, resolve); }); return socket.address().port; };
const close = (socket) => new Promise((resolve) => socket.close(resolve));
function parse(bytes) {
    if (bytes.length > 16384) throw Error("SIP bound");
    const [head, body = ""] = bytes.toString("utf8").split("\r\n\r\n");
    const [line, ...lines] = head.split("\r\n"); const headers = new Map();
    for (const row of lines) { const at = row.indexOf(":"); if (at < 1) throw Error("SIP header"); const name = row.slice(0, at).toLowerCase(); if (headers.has(name)) throw Error("duplicate fixture header"); headers.set(name, row.slice(at + 1).trim()); }
    if (Number(headers.get("content-length")) !== Buffer.byteLength(body)) throw Error("SIP length");
    const [method, uri, protocol] = line.split(" ");
    if (protocol !== "SIP/2.0" || !headers.get("call-id") || !headers.get("cseq")) throw Error("SIP request");
    return { method, uri, headers, body };
}
function answer(request, status, reason, tag, body = "", port) {
    const h = request.headers; let to = h.get("to");
    if (tag && !/;tag=/i.test(to)) to += `;tag=${tag}`;
    return Buffer.from(`SIP/2.0 ${status} ${reason}\r\nVia: ${h.get("via")}\r\nFrom: ${h.get("from")}\r\nTo: ${to}\r\nCall-ID: ${h.get("call-id")}\r\nCSeq: ${h.get("cseq")}\r\n${port ? `Contact: <sip:fixture@${host}:${port}>\r\n` : ""}${body ? "Content-Type: application/sdp\r\n" : ""}Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
}
function mediaTarget(sdp) {
    const address = sdp.match(/^c=IN IP4 ([^\r\n]+)$/m)?.[1]?.trim();
    const media = sdp.match(/^m=audio (\d+) RTP\/AVP ([^\r\n]+)$/m);
    if (address !== host || !media || !media[2].trim().split(/\s+/).includes("0")) throw Error("loopback PCMU SDP required");
    const port = Number(media[1]); if (port < 1 || port > 65535) throw Error("RTP port");
    return { address, port, dtmf: /^a=rtpmap:101 telephone-event\/8000\s*$/m.test(sdp) };
}
/** Bounded loopback UDP SIP/RTP participant, not a carrier or Connect controller. */
export class SipFixture {
    constructor({ user = "+12025550102", markerHz = 900, autoAnswer = true, waitForPrompt = false, confirmationDigit = null } = {}) {
        if (!/^\+1202555010[0-4]$/.test(user)) throw Error("synthetic SIP user required");
        if (confirmationDigit !== null && !/^[0-9]$/.test(String(confirmationDigit))) throw Error("fixture DTMF digit");
        marker(markerHz); this.user = user; this.frequency = markerHz; this.autoAnswer = autoAnswer; this.waitForPrompt = waitForPrompt; this.digit = confirmationDigit;
        this.dialogs = new Map(); this.invites = 0; this.effects = 0; this.stopping = false; this.pendingSends = 0;
        this.socket = dgram.createSocket("udp4");
        this.socket.on("message", (bytes, remote) => { if (remote.address === host && !this.stopping) this.receive(bytes, remote).catch((error) => { this.failure = error; }); });
    }
    async start() { this.port = await bind(this.socket); this.uri = `sip:${this.user}@${host}:${this.port}`; return this; }
    reply(bytes, remote) { if (!this.stopping) this.socket.send(bytes, remote.port, remote.address); }
    async receive(bytes, remote) {
        let request; try { request = parse(bytes); } catch { return; }
        const id = request.headers.get("call-id"); const method = request.method; let entry = this.dialogs.get(id);
        if (entry && entry.peerPort !== remote.port) { this.reply(answer(request, 481, "Unknown Dialog"), remote); return; }
        if (method === "OPTIONS") { this.reply(answer(request, 200, "OK", "", "", this.port), remote); return; }
        if (method === "INVITE") {
            this.invites++;
            if (!request.uri.startsWith(`sip:${this.user}@`)) { this.reply(answer(request, 404, "Not Found"), remote); return; }
            if (entry) {
                if (entry.original !== bytes.toString("base64")) { this.reply(answer(request, 501, "Re-INVITE Unsupported", entry.tag), remote); return; }
                await entry.ready;
                this.reply(entry.stopped ? answer(request, 481, "Ended Dialog", entry.tag) : entry.response, remote); return;
            }
            if (this.stopping || this.dialogs.size >= 8) { this.reply(answer(request, 503, "Fixture Capacity"), remote); return; }
            let target; try { target = mediaTarget(request.body); } catch { this.reply(answer(request, 488, "Unsupported Media"), remote); return; }
            const socket = dgram.createSocket("udp4");
            entry = { request, peerPort: remote.port, original: bytes.toString("base64"), tag: `fixture-${++this.effects}`, socket, target, received: 0, generated: 0, lastSample: 0, acknowledged: false, stopped: false, paused: false, sequence: 0, sentDtmf: false, timer: null, confirmation: null, response: answer(request, 180, "Ringing", `fixture-${this.effects}`) };
            // Reserve before asynchronous socket bind: duplicate INVITEs cannot allocate a second leg.
            this.dialogs.set(id, entry);
            socket.on("message", (packet, source) => this.receiveMedia(entry, packet, source));
            entry.ready = bind(socket);
            await entry.ready;
            if (entry.stopped || this.stopping) return;
            this.reply(entry.response, remote);
            if (this.autoAnswer) {
                const sdp = `v=0\r\no=fixture 1 1 IN IP4 ${host}\r\ns=SIMULATED speaker marker\r\nc=IN IP4 ${host}\r\nt=0 0\r\nm=audio ${socket.address().port} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\na=sendrecv\r\n`;
                entry.response = answer(request, 200, "OK", entry.tag, sdp, this.port);
                this.reply(entry.response, remote);
            }
        } else if (method === "ACK" && entry && !entry.stopped && this.autoAnswer && request.headers.get("cseq") === entry.request.headers.get("cseq").replace(/INVITE$/, "ACK")) {
            if (entry.acknowledged) return;
            entry.acknowledged = true; entry.epoch = performance.now();
            entry.timer = setInterval(() => {
                if (entry.stopped || entry.paused || (this.waitForPrompt && entry.received < 3)) return;
                if (this.pendingSends >= 4) { void this.stop(entry); return; }
                const timestamp = Math.floor((performance.now() - entry.epoch) * 8);
                this.send(entry, marker(this.frequency, timestamp), 0, timestamp); entry.generated++;
            }, 20);
        } else if (method === "BYE" && entry) {
            this.reply(answer(request, 200, "OK", entry.tag), remote); await this.stop(entry);
        } else if (method === "CANCEL" && entry && !entry.acknowledged && request.headers.get("cseq") === entry.request.headers.get("cseq").replace(/INVITE$/, "CANCEL")) {
            this.reply(answer(request, 200, "OK", entry.tag), remote);
            entry.response = answer(entry.request, 487, "Request Terminated", entry.tag);
            this.reply(entry.response, remote); await this.stop(entry);
        } else if (method !== "ACK") this.reply(answer(request, 481, "Unknown Dialog"), remote);
    }
    send(entry, bytes, type, timestamp) {
        if (entry.stopped) return;
        this.pendingSends++;
        entry.socket.send(rtp(bytes, { type, sequence: ++entry.sequence, timestamp, ssrc: this.frequency }), entry.target.port, entry.target.address, () => { this.pendingSends--; });
    }
    receiveMedia(entry, bytes, remote) {
        if (entry.stopped || remote.address !== host || remote.port !== entry.target.port) return;
        let packet; try { packet = unpack(bytes); } catch { return; }
        if (packet.type !== 0 || packet.payload.length !== 160) return;
        entry.received++; entry.lastSample = decode(packet.payload[0]);
        if (entry.received >= 3 && this.digit !== null && entry.target.dtmf && !entry.sentDtmf && entry.acknowledged) {
            entry.sentDtmf = true;
            const timestamp = Math.floor((performance.now() - entry.epoch) * 8);
            const event = Buffer.from([Number(this.digit), 10, 0, 160]);
            this.send(entry, event, 101, timestamp);
            entry.confirmation = setTimeout(() => { event[1] |= 128; for (let i = 0; i < 3; i++) this.send(entry, event, 101, timestamp); }, 20);
        }
    }
    pause(id, paused) { const entry = this.dialogs.get(id); if (!entry || entry.stopped) throw Error("active fixture dialog required"); entry.paused = paused; }
    async stop(entry) {
        if (entry.stopping) return entry.stopping;
        entry.stopped = true; clearInterval(entry.timer); clearTimeout(entry.confirmation);
        entry.stopping = (async () => { await entry.ready; await close(entry.socket); })();
        return entry.stopping;
    }
    close() {
        if (this.closing) return this.closing;
        this.stopping = true;
        this.closing = (async () => {
            await Promise.all([...this.dialogs.values()].map((entry) => this.stop(entry)));
            await close(this.socket);
            if (this.pendingSends) throw Error("fixture UDP queue not drained");
            if (this.failure) throw this.failure;
        })();
        return this.closing;
    }
}
