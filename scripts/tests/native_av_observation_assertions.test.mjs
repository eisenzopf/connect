import test from "node:test";
import assert from "node:assert/strict";
import { assertNativeCounterObservation, OriginalFailureJoinClock } from "./native_av_observation_assertions.mjs";

const observation = () => ({ format: "connect/native-av-work-observation/1", original_tasks_joined: true,
    fixture_payload_arrays_released: true, encoder_processes_joined: 1, original_tasks_started: 5,
    control_task_started: false, elapsed_ms: 12, failure: { route_index: 0, medium: "audio", direction: "send", stage: "rtp" },
    routes: [{ index: 0, received_audio: 0, received_video: 0, submitted_packets: { audio: 0, video: 0 },
        received_clocks: { audio: 1, video: 1 }, gap_packets: 0, late_send_frames: 0 }],
    scope: "Counter observation after joined owners." });

test("counter metadata explicitly allows only a boolean release observation", () => {
    assertNativeCounterObservation(observation());
    for (const invalid of ["AQI=", Buffer.from([1, 2]), [1, 2], { payload: "AQI=" }, 1]) {
        const value = observation(); value.fixture_payload_arrays_released = invalid;
        assert.throws(() => assertNativeCounterObservation(value));
    }
});

test("actual packet payloads, raw PCM samples and typed sample buffers remain rejected", () => {
    const mutations = [
        value => { value.routes[0].payload = "AQI="; },
        value => { value.routes[0].pcm = new Float32Array([.25, -.25]); },
        value => { value.routes[0].samples = [.25, -.25]; },
        value => { value.routes[0].received_audio = [.25, -.25]; },
        value => { value.routes[0].submitted_packets.audio = Buffer.from([1, 2]); },
        value => { value.routes[0].received_clocks.video = new Uint8Array([1, 2]); },
        value => { value.failure = { ...value.failure, packet: { payload: "AQI=" } }; },
    ];
    for (const mutate of mutations) {
        const value = observation(); mutate(value);
        assert.throws(() => assertNativeCounterObservation(value));
    }
});

test("setup is separately recorded and a repeated failure cannot reset the original one-second join bound", () => {
    let now = 0;
    const clock = new OriginalFailureJoinClock(() => now);
    now = 5000; clock.beginFailure(); now = 5500; clock.beginFailure(); now = 5999; clock.joined();
    assert.deepEqual(clock.assertBound(), { setup_ms: 5000, cancellation_join_ms: 999 });
    const expired = new OriginalFailureJoinClock(() => now);
    expired.beginFailure(); now += 1000; expired.joined();
    assert.throws(() => expired.assertBound(), /unchanged 1000ms bound/);
    const missing = new OriginalFailureJoinClock(() => now);
    assert.throws(() => missing.joined(), /boundary/);
});

test("a genuinely slow original join still fails the one-second bound", async () => {
    const clock = new OriginalFailureJoinClock(); clock.beginFailure();
    await new Promise(resolve => setTimeout(resolve, 1050)); clock.joined();
    assert.throws(() => clock.assertBound(), /unchanged 1000ms bound/);
});
