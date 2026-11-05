// Test-only checks for the public counter observation and original join bound.
import assert from "node:assert/strict";

const fields = (value, expected) => {
    assert.ok(value && Object.getPrototypeOf(value) === Object.prototype, "plain counter object required");
    assert.deepEqual(Object.keys(value).sort(), [...expected].sort(), "unexpected raw observation field");
};
const count = value => assert.ok(Number.isSafeInteger(value) && value >= 0, "numeric counter required; no packet or PCM data");
const pair = value => { fields(value, ["audio", "video"]); count(value.audio); count(value.video); };

export function assertNativeCounterObservation(value) {
    const setup = value.failure?.direction === "setup";
    fields(value, ["format", "original_tasks_joined", "fixture_payload_arrays_released", "encoder_processes_joined",
        "original_tasks_started", "elapsed_ms", "failure", "routes", "scope", setup ? "prepared_routes" : "control_task_started"]);
    assert.equal(value.format, "connect/native-av-work-observation/1");
    for (const name of ["original_tasks_joined", "fixture_payload_arrays_released"])
        assert.equal(typeof value[name], "boolean", "release/join metadata must be boolean");
    for (const name of ["encoder_processes_joined", "original_tasks_started"]) count(value[name]);
    if (setup) count(value.prepared_routes); else assert.equal(typeof value.control_task_started, "boolean");
    assert.ok(Number.isFinite(value.elapsed_ms) && value.elapsed_ms >= 0);
    assert.equal(typeof value.scope, "string");
    if (value.failure !== null) {
        fields(value.failure, ["route_index", "medium", "direction", "stage"]);
        if (value.failure.route_index !== null) count(value.failure.route_index);
        assert.ok([null, "audio", "video"].includes(value.failure.medium));
        assert.ok(["setup", "control", "send", "receive"].includes(value.failure.direction));
        assert.ok(["media-refresh", "connection-refresh", "source-input", "live-video-source", "clock", "rtp",
            "during-workload", "programme-join", "pending", "feedback", "packet", "receive"].includes(value.failure.stage));
    }
    assert.ok(Array.isArray(value.routes));
    for (const route of value.routes) {
        fields(route, ["index", "received_audio", "received_video", "submitted_packets", "received_clocks", "gap_packets", "late_send_frames"]);
        for (const name of ["index", "received_audio", "received_video", "gap_packets", "late_send_frames"]) count(route[name]);
        pair(route.submitted_packets); pair(route.received_clocks);
    }
    if (setup) assert.equal(value.routes.length, 0);
}

export class OriginalFailureJoinClock {
    constructor(now = () => performance.now()) { this.now = now; this.started = now(); }
    beginFailure() { this.failed ??= this.now(); }
    joined() {
        assert.ok(Number.isFinite(this.failed), "the original failure/cancellation boundary must be recorded first");
        assert.equal(this.completed, undefined, "the original join is recorded once");
        this.completed = this.now();
    }
    assertBound() {
        assert.ok(Number.isFinite(this.completed) && this.failed >= this.started && this.completed >= this.failed);
        const result = Object.freeze({ setup_ms: this.failed - this.started, cancellation_join_ms: this.completed - this.failed });
        assert.ok(result.cancellation_join_ms < 1000, "original cancellation/join must finish within the unchanged 1000ms bound");
        return result;
    }
}
