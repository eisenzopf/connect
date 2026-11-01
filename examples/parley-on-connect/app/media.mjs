// Browser-facing packet ownership over genuine public Node SDK media handles.
// This is an application boundary, not an implementation of the SDK transport.
import { setTimeout as delay } from "node:timers/promises";
import { randomUUID } from "node:crypto";
const fail = code => { throw Object.assign(Error(code), { code }); };
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
function exact(value, keys) {
    if (!value || Object.getPrototypeOf(value) !== Object.prototype ||
        Object.keys(value).sort().join() !== [...keys].sort().join()) fail("invalid-app-request");
}
const fingerprint = value => JSON.stringify({ generation: value.generation, negotiation: value.negotiation });
const directions = { send: ["sendonly", "sendrecv"], receive: ["recvonly", "sendrecv"] };

export class AppMediaBridge {
    #open; #packet; #owners = new Map(); #closed = false;
    constructor({ openConnection, createPacket }) {
        if (typeof openConnection !== "function") fail("invalid-app-request");
        this.#open = openConnection; this.#packet = createPacket;
    }
    async open(role, owner, binding) {
        exact(binding, ["conversationId", "sessionId", "connectionId"]);
        if (!Object.values(binding).every(id)) fail("invalid-app-request");
        if (this.#closed || typeof this.#packet !== "function") fail("unsupported-app-action");
        const capturedBinding = structuredClone(binding);
        await this.closeOwner(owner);
        if (this.#closed || this.#owners.size >= 16) fail("app-request-capacity");
        if (this.#owners.has(owner) || [...this.#owners.values()].some(v =>
            v.binding.connectionId === capturedBinding.connectionId))
            fail("app-media-busy");
        const lease = { role, owner, binding: capturedBinding, id: randomUUID(),
            abort: new AbortController(), active: new Map(), timer: null };
        this.#owners.set(owner, lease);
        lease.opening = (async () => {
            // A cancelled stdio control read fences its whole SDK owner.
            // Drain bounded read-only admission instead; discard its late result.
            const result = await this.#open(role, lease.binding, this.#controlOptions());
            lease.handle = result; // Own disposal even if admission finishes after retirement.
            if (lease.abort.signal.aborted || this.#owners.get(owner) !== lease) fail("app-media-stopped");
            if (!Number.isSafeInteger(result.generation) || result.generation < 1 ||
                typeof result.controlCheck !== "function" ||
                !Array.isArray(result.negotiation?.streams) || result.negotiation.streams.length > 32 ||
                Object.keys(lease.binding).some(k => result.negotiation.binding?.[k] !== lease.binding[k])) fail("app-resync-required");
            lease.handle = result; lease.fingerprint = fingerprint(result); this.#touch(lease);
            lease.supervision = this.#supervise(lease);
            return { leaseId: lease.id, generation: result.generation,
                negotiation: structuredClone(result.negotiation) };
        })();
        try { return await lease.opening; }
        catch (error) {
            lease.abort.abort(); clearTimeout(lease.timer);
            await this.#dispose(lease);
            if (this.#owners.get(owner) === lease) this.#owners.delete(owner);
            throw error;
        }
    }
    async #supervise(lease) {
        try {
            while (!lease.abort.signal.aborted) {
                await delay(500, undefined, { signal: lease.abort.signal, ref: false });
                const current = await lease.handle.controlCheck(this.#controlOptions());
                if (current.generation !== lease.handle.generation || !["ready", "active"].includes(current.state))
                    fail("app-media-stopped");
            }
        } catch {
            if (!lease.abort.signal.aborted) {
                lease.abort.abort();
                // Do not await this watcher from itself; the owner joins it.
                void this.closeOwner(lease.owner).catch(() => {});
            }
        }
    }
    #controlOptions() {
        return { timeoutMs: 2000, signal: AbortSignal.timeout(2000) };
    }
    #options(lease) {
        return { timeoutMs: 2000, signal: AbortSignal.any([lease.abort.signal, AbortSignal.timeout(2000)]) };
    }
    #touch(lease) {
        clearTimeout(lease.timer);
        lease.timer = setTimeout(() => { void this.closeOwner(lease.owner).catch(() => {}); }, 5000);
        lease.timer.unref();
    }
    #require(role, owner, leaseId) {
        const lease = this.#owners.get(owner);
        if (!lease || lease.role !== role || lease.id !== leaseId || lease.abort.signal.aborted || !lease.handle)
            fail("app-media-stopped");
        return lease;
    }
    async #run(role, owner, input, direction, work) {
        if (!id(input.leaseId) || !id(input.streamId)) fail("invalid-app-request");
        const leaseId = input.leaseId, streamId = input.streamId;
        const lease = this.#require(role, owner, leaseId);
        const stream = lease.handle.negotiation.streams.find(v => v.id === streamId);
        if (!stream || !["audio", "video", "screen"].includes(stream.medium) ||
            !directions[direction].includes(stream.direction)) fail("invalid-app-request");
        const key = direction + ":" + streamId;
        if (lease.active.has(key)) fail("app-media-busy");
        const options = this.#options(lease); this.#touch(lease);
        const operation = (async () => {
            // The SDK's local negotiated handles/revision fence packet IO.
            // Signed Connection supervision has its own bounded control lane;
            // it must not serialize a network RPC ahead of every media packet.
            // Core and SDK current authorization remain mandatory for real IO.
            const check = async () => {
                const current = await lease.handle.check(options);
                if (fingerprint(current) !== lease.fingerprint) {
                    lease.abort.abort(); fail("app-media-stopped");
                }
                this.#require(role, owner, leaseId);
            };
            await check();
            const value = await work(lease.handle.media.streamById(streamId), options, check);
            this.#require(role, owner, leaseId); // Discard late original replies.
            return value;
        })().finally(() => lease.active.delete(key));
        lease.active.set(key, operation);
        return operation;
    }
    #capture(input) {
        exact(input, ["sequence", "packet", "remainingMs"]);
        if (!Number.isInteger(input.sequence) || input.sequence < 0 || input.sequence > 0xffffffff)
            fail("invalid-app-request");
        if (!Number.isInteger(input.remainingMs) || input.remainingMs < 1 || input.remainingMs > 200)
            fail("invalid-app-request");
        const expires = performance.now() + input.remainingMs, sequence = input.sequence;
        const packet = input.packet;
        if (packet?.kind === "rtp") {
            exact(packet, ["kind", "base64"]);
            if (typeof packet.base64 !== "string" || packet.base64.length > 1576 ||
                !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(packet.base64))
                fail("invalid-app-request");
            const bytes = Buffer.from(packet.base64, "base64");
            if (bytes.length < 12 || bytes.length > 1180 || bytes.toString("base64") !== packet.base64)
                fail("invalid-app-request");
            // Copy/validate through the actual installed SDK before async IO.
            const captured = this.#packet(input.sequence, bytes);
            return { expires, sequence, send: (stream, options) => stream.sendRtp(captured, options) };
        }
        if (packet?.kind === "clock") {
            exact(packet, ["kind", "report"]);
            const report = packet.report;
            exact(report, ["ssrc", "ntp", "rtp", "packets", "octets", "cname"]);
            if (["ssrc", "rtp", "packets", "octets"].some(k => !Number.isInteger(report[k]) ||
                report[k] < 0 || report[k] > 0xffffffff) || typeof report.ntp !== "string" ||
                !/^(0|[1-9][0-9]{0,19})$/.test(report.ntp) || BigInt(report.ntp) >= 2n ** 64n ||
                typeof report.cname !== "string" || !report.cname || Buffer.byteLength(report.cname) > 255)
                fail("invalid-app-request");
            const captured = { ...report, ntp: BigInt(report.ntp) };
            return { expires, sequence, send: (stream, options) => stream.sendClock(sequence, captured, options) };
        }
        fail("invalid-app-request");
    }
    send(role, owner, input) {
        exact(input, ["leaseId", "streamId", "packets"]);
        if (!Array.isArray(input.packets) || input.packets.length < 1 || input.packets.length > 8)
            fail("invalid-app-request");
        // Validate and copy the entire bounded batch before the first SDK write.
        const captured = input.packets.map(packet => this.#capture(packet));
        const binding = { leaseId: input.leaseId, streamId: input.streamId };
        return this.#run(role, owner, binding, "send", async (stream, options, check) => {
            const submissions = [];
            for (const packet of captured) {
                await check();
                const remaining = Math.floor(packet.expires - performance.now());
                if (remaining <= 0) {
                    submissions.push({ boundary: "expired", datagramSequence: packet.sequence }); continue;
                }
                submissions.push(await packet.send(stream, { ...options, timeoutMs: remaining }));
            }
            return { submissions };
        });
    }
    receive(role, owner, input) {
        exact(input, ["leaseId", "streamId"]);
        return this.#run(role, owner, input, "receive", async (stream, options, check) => {
            const packets = []; let drainUntil;
            while (packets.length < 8) {
                // Long-poll the first packet, then spend at most four milliseconds
                // draining original queued packets. Never wait to fill a batch.
                await check();
                const timeoutMs = packets.length ? Math.floor(drainUntil - performance.now()) : 1000;
                if (timeoutMs <= 0) break;
                let packet;
                try { packet = await stream.receivePacket({ ...options, timeoutMs }); }
                catch (error) {
                    if (error?.kind === "deadline" && error?.certainty === "read-only") break;
                    throw error;
                }
                await check();
                drainUntil ??= performance.now() + 4;
                if (packet.kind === "rtp") {
                    if (packet.bytes.length < 12 || packet.bytes.length > 1180) fail("app-resync-required");
                    packets.push({ kind: "rtp", sequence: packet.datagramSequence,
                        base64: Buffer.from(packet.bytes).toString("base64") });
                } else if (packet.kind === "clock") packets.push({ kind: "clock", sequence: packet.datagramSequence,
                    report: { ...packet.report, ntp: packet.report.ntp.toString() } });
                else if (packet.kind === "gap") packets.push({ kind: "gap", droppedPackets: packet.droppedPackets.toString() });
                else fail("app-resync-required");
            }
            return { packets };
        });
    }
    async retire(role, owner, input) {
        exact(input, ["leaseId"]);
        if (!id(input.leaseId)) fail("invalid-app-request");
        this.#require(role, owner, input.leaseId);
        await this.closeOwner(owner);
        return { stopped: true };
    }
    async closeOwner(owner) {
        const lease = this.#owners.get(owner); if (!lease) return;
        lease.abort.abort(); clearTimeout(lease.timer);
        await Promise.allSettled([lease.opening, lease.supervision, ...lease.active.values()]);
        await this.#dispose(lease);
        if (this.#owners.get(owner) === lease) this.#owners.delete(owner);
    }
    #dispose(lease) {
        return lease.disposal ??= Promise.resolve().then(() => lease.handle?.close?.());
    }
    async closeRole(role) {
        await Promise.all([...this.#owners.values()].filter(v => v.role === role).map(v => this.closeOwner(v.owner)));
    }
    async close() {
        this.#closed = true;
        await Promise.all([...this.#owners.keys()].map(owner => this.closeOwner(owner)));
    }
}
