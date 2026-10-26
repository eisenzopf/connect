import http from "node:http";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { readFile, writeFile, rename } from "node:fs/promises";
import { isAbsolute } from "node:path";
// Public test seed and bearer, never production credentials. Exact Telnyx-style
// callback bytes are signed; server-side admission/consent remains Connect's job.
const key = createPrivateKey({ key: Buffer.from("302e020100300506032b657004220420" + "19".repeat(32), "hex"), format: "der", type: "pkcs8" });
export const fixtureBearer = "parley-synthetic-sms-only";
export const fixturePublicKey = createPublicKey(key).export({ format: "der", type: "spki" }).subarray(-32).toString("base64");
const number = /^\+1202555010[0-4]$/;
export class SmsFixture {
    constructor({ ledger, holdAcknowledgement = false }) {
        if (typeof ledger !== "string" || !isAbsolute(ledger)) throw Error("absolute fixture ledger required");
        this.ledger = ledger; this.hold = holdAcknowledgement;
        this.messages = []; this.attempts = 0; this.pending = new Set(); this.accepting = true;
        this.queue = Promise.resolve(); this.responses = new Set(); this.receivers = new Set(); this.responseClosures = new Set();
        this.server = http.createServer((req, res) => void this.receive(req, res));
    }
    async start() {
        try { this.messages = JSON.parse(await readFile(this.ledger, "utf8")); } catch (e) { if (e.code !== "ENOENT") throw e; }
        if (!Array.isArray(this.messages) || this.messages.length > 256 || this.messages.some((m, i) => m.id !== this.messageId(i + 1) || m.simulated !== true || m.state !== "accepted" || !this.valid(m, true))) throw Error("corrupt fixture ledger");
        await new Promise((resolve) => this.server.listen(0, "127.0.0.1", resolve));
        this.url = `http://127.0.0.1:${this.server.address().port}/v2`;
        return this;
    }
    async receive(req, res) {
        let responseClosed;
        const closed = new Promise((resolve) => { responseClosed = resolve; }); this.responseClosures.add(closed);
        this.responses.add(res); res.once("close", () => { this.responses.delete(res); this.pending.delete(res); this.responseClosures.delete(closed); responseClosed(); });
        let finish; const task = new Promise((resolve) => { finish = resolve; }); this.receivers.add(task);
        try {
            if (!this.accepting) { res.writeHead(503).end(); return; }
            if (req.headers.authorization !== `Bearer ${fixtureBearer}`) { res.writeHead(401).end(); return; }
            if (req.method !== "POST" || req.url !== "/v2/messages") { res.writeHead(404).end(); return; }
            let bytes = 0; const chunks = [];
            for await (const chunk of req) { bytes += chunk.length; if (bytes > 8192) { res.writeHead(413).end(); return; } chunks.push(chunk); }
            const value = JSON.parse(Buffer.concat(chunks).toString("utf8"));
            if (!this.valid(value)) { res.writeHead(422).end(); return; }
            // Count attempts before any backend behavior. Equal text is never an
            // idempotency key: two intended messages may have identical bodies.
            this.attempts++;
            const transaction = this.queue.then(async () => {
                if (!this.accepting) throw Error("fixture closing");
                if (this.messages.length >= 256) return null;
                const entry = { ...value, id: this.messageId(this.messages.length + 1), state: "accepted", simulated: true };
                const next = [...this.messages, entry];
                await writeFile(this.ledger + ".tmp", JSON.stringify(next), { mode: 0o600 });
                await rename(this.ledger + ".tmp", this.ledger);
                this.messages = next; return entry;
            });
            this.queue = transaction.catch(() => {});
            const entry = await transaction;
            if (!entry) { res.writeHead(503).end(); return; }
            if (!this.accepting || res.destroyed) { res.destroy(); return; }
            if (this.hold) {
                this.pending.add(res);
            } else res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify({ data: { id: entry.id, from: { phone_number: value.from }, to: [{ phone_number: value.to, status: "queued" }], text: value.text }, simulated: true }));
        } catch { if (!res.destroyed) res.writeHead(400).end(); }
        finally { this.receivers.delete(task); finish(); }
    }
    messageId(sequence) { return `00000000-0000-4000-8000-${String(sequence).padStart(12, "0")}`; }
    valid(value, stored = false) {
        const allowed = ["from", "to", "text", "type", "messaging_profile_id", ...(stored ? ["id", "state", "simulated"] : [])];
        return value && typeof value === "object" && Object.keys(value).every((k) => allowed.includes(k)) && number.test(value.from) && number.test(value.to) && value.from !== value.to && typeof value.text === "string" && value.text.length > 0 && [...value.text].length <= 1600 && (value.type === undefined || value.type === "SMS") && (value.messaging_profile_id === undefined || typeof value.messaging_profile_id === "string");
    }
    callback(value, timestamp = Math.floor(Date.now() / 1000)) {
        const body = Buffer.from(JSON.stringify(value)); const stamp = String(timestamp);
        return { body, headers: { "telnyx-timestamp": stamp, "telnyx-signature-ed25519": sign(null, Buffer.concat([Buffer.from(stamp + "|"), body]), key).toString("base64") } };
    }
    incoming(id, text, from = "+12025550102", to = "+12025550100") {
        if (!number.test(from) || !number.test(to) || from === to) throw Error("fixture SMS pair");
        return this.callback({ data: { id, event_type: "message.received", payload: { id: `fixture-${id}`, from: { phone_number: from }, to: [{ phone_number: to }], text } } });
    }
    delivery(id, state) {
        if (!["delivered", "delivery_failed"].includes(state)) throw Error("explicit fixture delivery state");
        const message = this.messages.find((m) => m.id === id); if (!message) throw Error("original fixture message");
        return this.callback({ data: { id: `${id}:${state}`, event_type: "message.finalized", payload: { id, from: { phone_number: message.from }, to: [{ phone_number: message.to, status: state }] } } });
    }
    close() {
        if (this.closing) return this.closing;
        this.accepting = false;
        for (const response of this.responses) response.destroy();
        this.server.closeAllConnections();
        this.closing = (async () => {
            await new Promise((resolve, reject) => this.server.close((err) => err && err.code !== "ERR_SERVER_NOT_RUNNING" ? reject(err) : resolve()));
            await Promise.all([...this.receivers]); await this.queue;
            await Promise.all([...this.responseClosures]);
            if (this.pending.size || this.responses.size) throw Error("SMS response futures not drained");
        })();
        return this.closing;
    }
}
