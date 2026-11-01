import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import { AppMediaClockTrace, replayMediaClockTrace } from "../lib/app_media_clock_trace.mjs";

const origin = "http://127.0.0.1:41234", lease = "private-original-lease", stream = "private-original-stream";
const drain = () => new Promise(done => setImmediate(done));
const open = () => ({ leaseId: lease, negotiation: { streams: [{ id: stream, medium: "audio" }] } });
function rtp(sequence) {
    const b = Buffer.alloc(20); b[0] = 128; b[1] = 111; b.writeUInt16BE(sequence % 65536, 2);
    b.writeUInt32BE(sequence * 960 >>> 0, 4); b.writeUInt32BE(1234567890, 8); b.write("private!", 12);
    return { kind: "rtp", base64: b.toString("base64") };
}
const clock = (ntp = "18446744073709551615") => ({ kind: "clock", report: {
    ssrc: 1234567890, rtp: 0xffffffff, ntp, packets: 1, octets: 80, cname: "private-cname" } });
function response(path, input, output, body, host = origin) {
    return { url: () => host + path, status: () => 200,
        request: () => ({ postDataJSON: () => input, timing: () => ({ responseEnd: 23.5 }) }),
        body: body ?? (async () => Buffer.from(JSON.stringify(output))) };
}
function observe(page, path, value, direction = "receive") {
    page.emit("response", response(path, { leaseId: lease, streamId: stream,
        ...(direction === "send" ? { packets: [{ packet: value }] } : {}) }, direction === "send" ? { submissions: [{ boundary: "local-media-queue" }] } : { packets: [value] }));
}

test("original send/receive clocks, packets and HTTP timing retain no payload or private identifiers", async () => {
    let now = 10; const t = new AppMediaClockTrace(() => now), page = new EventEmitter();
    t.attach(page, "owner-audio", origin); page.emit("response", response("/api/media/open", {}, open())); await drain();
    now = 20; observe(page, "/api/media/send", clock(), "send"); await drain();
    now = 30; observe(page, "/api/media/receive", clock()); await drain();
    now = 40; observe(page, "/api/media/receive", rtp(1)); await drain();
    const result = await t.finish(), route = result.pages[0].routes[0], text = JSON.stringify(result);
    assert.equal(route.medium, "audio"); assert.equal(route.rate, 48000);
    assert.equal(route.send.clocks[0].ntp_delta_ms, 0); assert.equal(route.receive.clocks[0].source_slot, 0);
    assert.equal(route.receive.rtp[0].request_duration_ms, 23.5);
    assert.equal(route.receive.rtp[0].observed_elapsed_ms, 30);
    for (const secret of [lease, stream, "private-cname", "private!", "1234567890", "18446744073709551615", rtp(1).base64])
        assert.equal(text.includes(secret), false);
    assert.equal(result.pending_reads_at_finish, 0); assert.equal(page.listenerCount("response"), 0);
});
test("bounded rings retain the original head and most recent packets/clocks", async () => {
    let now = 0; const t = new AppMediaClockTrace(() => now), page = new EventEmitter(); t.attach(page, "owner-av", origin);
    page.emit("response", response("/api/media/open", {}, open())); await drain();
    for (let i = 0; i < 100; i++) {
        now++; observe(page, "/api/media/receive", rtp(i)); await drain();
        now++; observe(page, "/api/media/receive", clock(String(1000 + i))); await drain();
    }
    const result = await t.finish(), flow = result.pages[0].routes[0].receive;
    assert.equal(flow.rtp_seen, 100); assert.equal(flow.clocks_seen, 100);
    assert.equal(flow.rtp.length, 48); assert.equal(flow.clocks.length, 24);
    assert.deepEqual(flow.rtp.slice(0, 16).map(v => v.sequence), Array.from({ length: 16 }, (_, i) => i));
    assert.deepEqual(flow.rtp.slice(16).map(v => v.sequence), Array.from({ length: 32 }, (_, i) => i + 68));
});
test("pending original reads are bounded and joined before a stable trace is returned", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(), releases = [];
    t.attach(page, "organizer-audio", origin);
    for (let i = 0; i < 5; i++) {
        const heldBody = () => new Promise(done => { releases.push(() => done(Buffer.from(JSON.stringify({ packets: [rtp(i)] })))); });
        page.emit("response", response("/api/media/receive", { leaseId: lease, streamId: stream }, null, heldBody));
    }
    const finish = t.finish(); assert.equal(page.listenerCount("response"), 0);
    releases.forEach(done => done()); const result = await finish;
    assert.equal(result.pages[0].skipped_pending, 1); assert.equal(result.pending_reads_at_finish, 0);
    assert.equal(result.pages[0].routes[0].receive.rtp_seen, 4);
});
test("wrong origins, oversized clocks and malformed bodies cannot enter a clock trace", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(); t.attach(page, "organizer-av", origin);
    page.emit("response", response("/api/media/receive", {}, clock(), undefined, "https://other.invalid"));
    observe(page, "/api/media/receive", clock("18446744073709551616")); await drain();
    page.emit("response", response("/api/media/receive", { leaseId: lease, streamId: stream }, null, async () => Buffer.alloc(65537)));
    await drain(); const result = await t.finish();
    assert.equal(result.pages[0].observation_failures, 2); assert.equal(result.pages[0].observed_responses, 0);
    assert.equal(result.pages[0].routes[0].receive.clocks_seen, 0);
});
test("observer replay identifies packets already late before browser decoding without retiming originals", () => {
    const flow = { rtp: [], clocks: [] }, row = { source_slot: 0, request_duration_ms: 150 };
    const trace = { pages: [{ label: "owner-audio", routes: [{ route_slot: 0, lease_slot: 0, medium: "audio", rate: 48000,
        send: flow, receive: { clocks: [{ ...row, usable: true, observed_elapsed_ms: 100, rtp: 4800,
            ntp_delta_ms: 0, packets: 1, octets: 80 }], rtp: [{ ...row, observed_elapsed_ms: 250, rtp: 4800 }] } }] }] };
    const original = JSON.stringify(trace), result = replayMediaClockTrace(trace).pages[0].routes[0];
    assert.equal(result.received_source_age.p50_ms, 150);
    assert.equal(result.estimated_playout_offset.p50_ms, -70);
    assert.equal(JSON.stringify(trace), original);
});

test("one original batch retains packet order and refuses more than eight entries", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(); t.attach(page, "owner-audio", origin);
    page.emit("response", response("/api/media/open", {}, open())); await drain();
    page.emit("response", response("/api/media/receive", { leaseId: lease, streamId: stream },
        { packets: [rtp(1), clock("10000000000"), rtp(2)] })); await drain();
    page.emit("response", response("/api/media/receive", { leaseId: lease, streamId: stream },
        { packets: Array.from({ length: 9 }, (_, i) => rtp(i + 3)) })); await drain();
    const result = await t.finish(), p = result.pages[0], flow = p.routes[0].receive;
    assert.equal(p.observation_failures, 1); assert.equal(flow.rtp_seen, 2); assert.equal(flow.clocks_seen, 1);
    assert.ok(flow.rtp[0].observation_order < flow.clocks[0].observation_order);
    assert.ok(flow.clocks[0].observation_order < flow.rtp[1].observation_order);
    assert.equal(result.observer_clock_replay.pages[0].routes[0].unsynchronized_retained_packets, 1);
});

test("four pending packet observations cannot discard original negotiation metadata", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(), releases = [];
    t.attach(page, "owner-av", origin);
    for (let i = 0; i < 4; i++) {
        page.emit("response", response("/api/media/receive", { leaseId: lease, streamId: stream }, null,
            () => new Promise(done => releases.push(() => done(Buffer.from(JSON.stringify({ packets: [rtp(i)] })))))));
    }
    page.emit("response", response("/api/media/open", {}, open()));
    await drain(); releases.forEach(done => done());
    const result = await t.finish(), route = result.pages[0].routes[0];
    assert.equal(route.medium, "audio"); assert.equal(route.rate, 48000);
    assert.equal(route.receive.rtp_seen, 4);
    assert.equal(result.pages[0].observed_open_responses, 1);
    assert.equal(result.pages[0].skipped_open_pending, 0);
    assert.equal(result.pending_reads_at_finish, 0);
});

test("negotiation and packet observations have independent four-read budgets and original joins", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(), releases = [];
    t.attach(page, "organizer-av", origin);
    for (const kind of ["open", "receive"]) for (let i = 0; i < 5; i++) {
        const output = kind === "open" ? open() : { packets: [rtp(i)] };
        page.emit("response", response("/api/media/" + kind, { leaseId: lease, streamId: stream }, null,
            () => new Promise(done => releases.push(() => done(Buffer.from(JSON.stringify(output)))))));
    }
    assert.equal(releases.length, 8);
    const finish = t.finish(); releases.forEach(done => done()); const result = await finish, pageResult = result.pages[0];
    assert.deepEqual(result.read_bounds, { open: 4, packets: 4, total: 8 });
    assert.equal(pageResult.skipped_pending, 2); assert.equal(pageResult.skipped_open_pending, 1);
    assert.equal(pageResult.observed_open_responses, 4); assert.equal(pageResult.routes[0].receive.rtp_seen, 4);
    assert.equal(result.pending_reads_at_finish, 0); assert.equal(result.pending_open_reads_at_finish, 0);
    assert.equal(result.pending_packet_reads_at_finish, 0); assert.equal(page.listenerCount("response"), 0);
});

test("failed negotiation reads remain unknown and cannot infer a clock rate from RTP payload types", async () => {
    const t = new AppMediaClockTrace(), page = new EventEmitter(); t.attach(page, "owner-av", origin);
    page.emit("response", response("/api/media/open", {}, null, async () => { throw Error("private body details"); }));
    await drain(); observe(page, "/api/media/receive", rtp(1)); await drain();
    const result = await t.finish(), p = result.pages[0];
    assert.equal(p.open_observation_failures, 1); assert.equal(p.observation_failure_stages.body, 1);
    assert.equal(p.observed_open_responses, 0); assert.equal(p.routes[0].medium, "unknown");
    assert.equal(p.routes[0].rate, null); assert.deepEqual(result.observer_clock_replay.pages[0].routes, []);
    assert.equal(JSON.stringify(result).includes("private body details"), false);
});
