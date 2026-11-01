import test from "node:test";
import assert from "node:assert/strict";
import { AppMediaBridge } from "./media.mjs";

const binding = { conversationId: "conversation-1", sessionId: "session-1", connectionId: "connection-1" };
const owner = () => ({});
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const rtp = Buffer.from([0x80, 111, 0, 1, 0, 0, 3, 192, 0, 0, 0, 7, 1]);
function fixture(t, overrides = {}) {
    const calls = [], state = { generation: 1, revision: 1 };
    const negotiation = () => ({ binding, sourceClock: true, streamRevision: state.revision,
        streams: [{ id: "audio-1", medium: "audio", direction: "sendrecv", codec: { name: "opus" } },
            { id: "incoming-1", medium: "audio", direction: "recvonly", codec: { name: "opus" } }] });
    const snapshot = () => ({ generation: state.generation, negotiation: negotiation() });
    const stream = {
        sendRtp: async (packet, options) => { calls.push({ kind: "send", packet, options }); return { boundary: "local-media-queue", datagramSequence: packet.sequence }; },
        sendClock: async (sequence, report, options) => { calls.push({ kind: "clock", sequence, report, options }); return { boundary: "local-media-queue", datagramSequence: sequence }; },
        receivePacket: async options => { calls.push({ kind: "receive", options }); return { kind: "rtp", bytes: rtp, datagramSequence: 7 }; },
        ...overrides.stream,
    };
    const bridge = new AppMediaBridge({ createPacket: (sequence, bytes) => ({ sequence, bytes: Buffer.from(bytes) }),
        openConnection: async (role, scope, options) => {
            calls.push({ kind: "open", role, scope, options });
            if (overrides.open) await overrides.open(options);
            return { ...snapshot(), media: { streamById: id => { assert.ok(["audio-1", "incoming-1"].includes(id)); return stream; } },
                check: async options => { calls.push({ kind: "check", options }); return snapshot(); } };
        } });
    t.after(() => bridge.close());
    return { bridge, calls, state, stream };
}
const input = leaseId => ({ leaseId, streamId: "audio-1", sequence: 1, packet: { kind: "rtp", base64: rtp.toString("base64") } });

test("packet IO is scoped to the original actor and browser owner, with one physical publisher", async t => {
    const f = fixture(t), a = owner(), b = owner(), lease = await f.bridge.open("owner", a, binding);
    await assert.rejects(f.bridge.send("organizer", a, input(lease.leaseId)), { code: "app-media-stopped" });
    await assert.rejects(f.bridge.send("owner", b, input(lease.leaseId)), { code: "app-media-stopped" });
    await assert.rejects(f.bridge.open("owner", b, binding), { code: "app-media-busy" });
    const result = await f.bridge.send("owner", a, input(lease.leaseId));
    assert.deepEqual(result, { boundary: "local-media-queue", datagramSequence: 1 });
    assert.deepEqual(f.calls.find(c => c.kind === "send").packet.bytes, rtp);
    const received = await f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" });
    assert.deepEqual(received, { kind: "rtp", sequence: 7, base64: rtp.toString("base64") });
    assert.equal(f.calls.filter(c => c.kind === "send").length, 1);
});

test("physical generation and negotiated revision changes fence old packets before SDK IO", async t => {
    for (const change of ["generation", "revision"]) {
        const f = fixture(t), a = owner(), lease = await f.bridge.open("owner", a, binding);
        f.state[change]++;
        await assert.rejects(f.bridge.send("owner", a, input(lease.leaseId)), { code: "app-media-stopped" });
        assert.equal(f.calls.some(c => c.kind === "send"), false);
        await assert.rejects(f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" }), { code: "app-media-stopped" });
    }
});

test("closing an owner aborts and joins a pending read and discards its late original result", async t => {
    const entered = deferred(), release = deferred();
    let signal;
    const f = fixture(t, { stream: { receivePacket: async options => {
        signal = options.signal; entered.resolve(); await release.promise;
        return { kind: "rtp", bytes: rtp, datagramSequence: 7 };
    } } }), a = owner(), lease = await f.bridge.open("owner", a, binding);
    const read = f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" });
    const refused = assert.rejects(read, { code: "app-media-stopped" }); await entered.promise;
    let joined = false; const close = f.bridge.closeOwner(a).then(() => { joined = true; });
    assert.equal(signal.aborted, true); await Promise.resolve(); assert.equal(joined, false);
    release.resolve(); await refused; await close; assert.equal(joined, true);
    const next = await f.bridge.open("organizer", a, binding); assert.notEqual(next.leaseId, lease.leaseId);
});

test("only one read or write per Stream direction is admitted; no waiting packet queue grows", async t => {
    const entered = deferred(), release = deferred();
    const f = fixture(t, { stream: { sendRtp: async () => { entered.resolve(); await release.promise; return { boundary: "local-media-queue", datagramSequence: 1 }; } } });
    const a = owner(), lease = await f.bridge.open("owner", a, binding);
    const send = f.bridge.send("owner", a, input(lease.leaseId)); await entered.promise;
    await assert.rejects(f.bridge.send("owner", a, input(lease.leaseId)), { code: "app-media-busy" });
    // The independent incoming direction remains usable.
    assert.equal((await f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" })).kind, "rtp");
    release.resolve(); await send;
});

test("owner retirement during authorization joins the opener and never publishes an old lease", async t => {
    const entered = deferred(), release = deferred(); let signal;
    const f = fixture(t, { open: async options => { signal = options.signal; entered.resolve(); await release.promise; } }), a = owner();
    const opening = f.bridge.open("owner", a, binding);
    const refused = assert.rejects(opening, { code: "app-media-stopped" }); await entered.promise;
    let joined = false; const closing = f.bridge.closeOwner(a).then(() => { joined = true; });
    assert.equal(signal.aborted, true); await Promise.resolve(); assert.equal(joined, false);
    release.resolve(); await refused; await closing; assert.equal(joined, true);
});

test("malformed packets, receive-only publication, unknown Streams and invalid clocks never reach SDK IO", async t => {
    const f = fixture(t), a = owner(), lease = await f.bridge.open("owner", a, binding);
    for (const patch of [{ streamId: "unknown" }, { streamId: "incoming-1" },
        { packet: { kind: "rtp", base64: rtp.toString("base64") + "\n" } }, { sequence: -1 },
        { packet: { kind: "clock", report: { ssrc: 7, ntp: "18446744073709551616", rtp: 960, packets: 1, octets: 1, cname: "synthetic" } } }]) {
        await assert.rejects(async () => f.bridge.send("owner", a, { ...input(lease.leaseId), ...patch }), { code: "invalid-app-request" });
    }
    assert.equal(f.calls.some(c => ["send", "clock", "receive"].includes(c.kind)), false);
});

test("source clocks and receiver clock/gap observations preserve exact integer values", async t => {
    const f = fixture(t), a = owner(), lease = await f.bridge.open("owner", a, binding);
    const report = { ssrc: 7, ntp: "18446744073709551615", rtp: 960, packets: 1, octets: 1, cname: "synthetic" };
    await f.bridge.send("owner", a, { ...input(lease.leaseId), packet: { kind: "clock", report } });
    assert.equal(f.calls.find(c => c.kind === "clock").report.ntp, 18446744073709551615n);
    f.stream.receivePacket = async () => ({ kind: "clock", datagramSequence: 8, report: { ...report, ntp: BigInt(report.ntp) } });
    assert.deepEqual((await f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" })).report, report);
    f.stream.receivePacket = async () => ({ kind: "gap", droppedPackets: 9007199254740993n });
    assert.deepEqual(await f.bridge.receive("owner", a, { leaseId: lease.leaseId, streamId: "audio-1" }),
        { kind: "gap", droppedPackets: "9007199254740993" });
});

test("concurrent opens cannot replace an unjoined owner or create two physical publishers", async t => {
    const entered = deferred(), release = deferred();
    const f = fixture(t, { open: async () => { entered.resolve(); await release.promise; } }), a = owner();
    const one = f.bridge.open("owner", a, binding), two = f.bridge.open("owner", a, binding);
    const refused = assert.rejects(two, { code: "app-media-busy" }); await entered.promise; await refused;
    assert.equal(f.calls.filter(c => c.kind === "open").length, 1); release.resolve(); await one;
});

test("abandoned browser leases retire after the fixed idle bound", async t => {
    const f = fixture(t), a = owner(), lease = await f.bridge.open("owner", a, binding);
    await new Promise(done => setTimeout(done, 5100));
    await assert.rejects(f.bridge.send("owner", a, input(lease.leaseId)), { code: "app-media-stopped" });
    assert.equal(f.calls.some(c => c.kind === "send"), false);
});
