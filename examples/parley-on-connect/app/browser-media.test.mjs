import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import { MediaTimeline, senderReport, rtpTimestamp, playoutTime } from "./ui/media-clock.mjs";
const report = (ssrc, rtp, ntp) => ({ ssrc, rtp, ntp: ntp.toString(), packets: 1, octets: 3, cname: "synthetic" });
test("playout uses the mapped local clock after SR correction and never makes a delayed frame fresh", () => {
    // An old first decoded packet cannot pin the presentation origin. Later SR
    // mapped AV frames must use their real local monotonic targets directly.
    assert.ok(playoutTime(1000000, 2020000, 0.5) < 0.5);
    assert.ok(Math.abs(playoutTime(2000000, 2020000, 0.5) - 0.56) < 1e-10);
    assert.ok(Math.abs(playoutTime(2010000, 2020000, 0.5) - 0.57) < 1e-10);
    assert.ok(playoutTime(2300000, 2020000, 0.5) > 0.7);
    // A suspended or differently advancing audio clock introduces no retained
    // offset; the same local target still has60ms remaining at current audioNow.
    assert.ok(Math.abs(playoutTime(5000000, 5020000, 1.1) - 1.16) < 1e-10);
    assert.throws(() => playoutTime(1, 2, NaN), { code: "app-media-clock" });
});
test("capture clock uses exact NTP integers and wraps RTP without truncating the source timeline", () => {
    const value = senderReport(1000000, 48000, { ssrc: 7, packets: 1, octets: 3 }, "synthetic", 0);
    assert.equal(value.ntp, (2208988801n << 32n).toString());
    assert.equal(value.rtp, 48000);
    assert.equal(rtpTimestamp(90000000000, 48000), 25032704);
    assert.throws(() => rtpTimestamp(NaN, 48000), { code: "app-media-clock" });
    assert.throws(() => senderReport(1, 44100, { ssrc: 7, packets: 1, octets: 3 }, "synthetic", 0), { code: "app-media-clock" });
});
test("audio and video use one NTP playout origin across independent RTP clock rates", () => {
    let now = 1000000; const clock = new MediaTimeline(() => now), ntp = 10n << 32n;
    clock.register("audio", 48000); clock.register("video", 90000);
    assert.ok(clock.report("audio", report(7, 0, ntp)));
    now = 1050000;
    assert.ok(clock.report("video", report(8, 4500, ntp + (1n << 32n) / 20n)));
    now = 1100000;
    const audio = clock.timestamp("audio", 7, 4800), video = clock.timestamp("video", 8, 9000);
    assert.equal(audio.synchronized, true); assert.equal(video.synchronized, true);
    assert.ok(Math.abs(audio.timestampUs - video.timestampUs) <= 1);
    assert.equal(audio.timestampUs, 1099999);
    assert.equal(clock.report("audio", report(7, 0, ntp)), false);
});
test("a corrected SR cannot map its generation into the future or make delayed media fresh", () => {
    let now = 1000000; const clock = new MediaTimeline(() => now), ntp = 10n << 32n;
    clock.register("audio", 48000); clock.register("video", 90000);
    clock.report("audio", report(7, 0, ntp)); clock.timestamp("audio", 7, 0);
    now = 1100000;
    const corrected = ntp + (3n << 32n) / 10n;
    assert.ok(clock.report("audio", report(7, 4800, corrected)));
    const audio = clock.timestamp("audio", 7, 4800);
    assert.equal(audio.clockChanged, true); assert.equal(audio.timestampUs, now);
    clock.report("video", report(8, 27000, corrected));
    const video = clock.timestamp("video", 8, 27000);
    assert.equal(video.timestampUs, audio.timestampUs);
    assert.ok(Math.abs(clock.timestamp("audio", 7, 0).timestampUs - 1000000) <= 1);
    // A later report with a larger receive delay cannot move the shared
    // offset forward and replay queued old content as newly captured media.
    now = 3000000;
    clock.report("audio", report(7, 9600, corrected + (1n << 32n) / 10n));
    const delayed = clock.timestamp("audio", 7, 9600);
    assert.equal(delayed.clockChanged, false);
    assert.ok(Math.abs(delayed.timestampUs - 1200000) <= 1);
    assert.ok(playoutTime(delayed.timestampUs, now, 2) < 2);
});
test("RTP wrap is continuous; an expired report or source change supplies no old synchronization", () => {
    let now = 1000000; const clock = new MediaTimeline(() => now);
    clock.register("audio", 48000); clock.report("audio", report(7, 0xffffff00, 10n << 32n));
    clock.timestamp("audio", 7, 0xffffff00); now += 20000;
    assert.ok(Math.abs(clock.timestamp("audio", 7, 704).timestampUs - now) <= 1);
    now += 4000000;
    assert.equal(clock.timestamp("audio", 7, 1664).synchronized, false);
    const changed = clock.timestamp("audio", 8, 0);
    assert.equal(changed.changed, true); assert.equal(changed.synchronized, false);
    assert.equal(changed.timestampUs, now);
});
test("unreported RTP retains its original gaps and never acquires an invented synchronized clock", () => {
    let now = 1000000; const clock = new MediaTimeline(() => now);
    clock.register("audio", 48000);
    assert.equal(clock.timestamp("audio", 7, 0).timestampUs, 1000000);
    now += 100000;
    const next = clock.timestamp("audio", 7, 4800);
    assert.equal(next.timestampUs, 1100000); assert.equal(next.synchronized, false);
    clock.clear(); assert.throws(() => clock.timestamp("audio", 7, 4800), { code: "app-media-clock" });
});
test("the actual capture worklet holds one message credit, preserves source sample time, and stops", async () => {
    const messages = []; let Processor;
    const context = { currentFrame: 0, registerProcessor: (_name, value) => { Processor = value; },
        AudioWorkletProcessor: class { constructor() { this.port = { postMessage: value => messages.push(value) }; } } };
    vm.runInNewContext(await readFile(new URL("./ui/audio-worklet.mjs", import.meta.url), "utf8"), context);
    const capture = new Processor(), input = new Float32Array(128).fill(0.25);
    const quantum = () => { const value = capture.process([[input]], [[new Float32Array(128)]]); context.currentFrame += 128; return value; };
    for (let i = 0; i < 30; i++) quantum();
    assert.equal(messages.length, 1); assert.equal(messages[0].firstFrame, 0);
    assert.equal(messages[0].samples.length, 960);
    assert.ok(messages[0].samples.every(n => n === 0.25));
    capture.port.onmessage({ data: "credit" });
    for (let i = 0; i < 8; i++) quantum();
    assert.equal(messages.length, 2); assert.equal(messages[1].firstFrame, 3840);
    capture.port.onmessage({ data: "stop" }); assert.equal(quantum(), false); assert.equal(messages.length, 2);
});
