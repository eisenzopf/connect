import { recordPrivateTargetCleanup } from "../lib/transport_failure_isolation.mjs";
// Real nativeAv lifetime coordination with owned test-double packet IO only.
// These opaque source bytes do not establish codec or transport qualification.
import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
import { addLiveVideoFixture } from "./live_video_fixture.mjs";
import { nativeAv } from "../lib/transport_av.mjs";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
async function exerciseLifetime(isolated = false) {
    const privateRoot = resolve(".local"); await mkdir(privateRoot, { mode: 0o700, recursive: true });
    const retained = process.env.CONNECT_PROGRAMME_LIFETIME_TEST_OUTPUT;
    const base = retained ? resolve(retained + (isolated ? "-isolated-private-failure" : "")) : await mkdtemp(join(privateRoot, "programme-lifetime-"));
    assert.ok(base.startsWith(privateRoot + "/"));
    if (retained) await mkdir(base, { mode: 0o700 });
    const sources = join(base, "sources"); await mkdir(sources, { mode: 0o700 });
    const groups = [["owner", "two", "three", "four"], ["owner", "two"]];
    const roles = [...groups[0], ...groups[1]], peers = [3, 0, 1, 2, 5, 4], plan = {};
    const connections = roles.map((role, index) => ({ role, sid: Number(index >= 4), prefix: "source-" + index }));
    for (const [index, item] of connections.entries()) for (const medium of ["audio", "video"])
        plan[item.prefix + "-" + medium] = "stream-" + index + "-" + medium;
    const input = JSON.stringify({ audio: Array(3100).fill("AQ=="),
        video: Array.from({ length: 1860 }, (_, i) => ({ timestamp: i * 3000, payload: "AQ==" })) });
    for (let index = 0; index < 6; index++) await writeFile(join(sources, "source-" + index + ".json"), input);
    const manifest = { format: "connect/native-av-sources/2",
        seconds: 62, observation_seconds: 60, sources: Array.from({ length: 6 }, () => ({ input_sha256: sha(input) })) };
    await addLiveVideoFixture(sources, manifest);
    await writeFile(join(sources, "manifest.json"), JSON.stringify(manifest));
    let stoppedReads = 0, pendingReads = 0, stoppedFeedbackReads = 0, pendingFeedbackReads = 0,
        controlJoined = false, observation, joinedProgramme;
    const original = Object.assign(Error("original original-submission refusal"), { kind: "authorization", certainty: "refused" });
    const media = {
        async sendRtp() { return { boundary: "local-media-queue" }; }, async sendClock() {},
        receiveVideoFeedback({ signal }) {
            pendingFeedbackReads++;
            return new Promise((_, reject) => {
                const abort = () => { pendingFeedbackReads--; stoppedFeedbackReads++;
                    reject(Object.assign(Error("owned feedback read stopped"), { kind: "cancelled" })); };
                if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
            });
        },
        async consumeVideoFeedback() { assert.fail("no actual feedback hint was supplied by this lifetime fixture"); },
        async requestKeyframe() { assert.fail("no actual receiving video packet was supplied by this lifetime fixture"); },
        receivePacket({ signal }) {
            pendingReads++;
            return new Promise((_, reject) => {
                const abort = () => { pendingReads--; stoppedReads++; reject(Object.assign(Error("owned read stopped"), { kind: "cancelled" })); };
                if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
            });
        },
    };
    for (const [index, item] of connections.entries()) {
        const stream = (source, medium, direction, handle) => ({ id: plan[connections[source].prefix + "-" + medium],
            medium, direction, streamLocalId: handle,
            ...(medium === "video" ? { videoFeedback: { profile: "rtcp-pli-epoch/1" } } : {}), codec: { name: medium === "audio" ? "opus" : "vp8",
                params: { payload_type: medium === "audio" ? 111 : 96 } } });
        item.connection = { id: "connection-" + index,
            async refresh() { return { generation: 1 }; },
            media: { async refresh() { return { sourceClock: true, binding: { sessionId: "session-" + item.sid },
                streams: [stream(index, "audio", "sendrecv", 1), stream(index, "video", "sendonly", 2),
                    stream(peers[index], "video", "recvonly", 3)] }; } },
            streams: { byId() { return { media }; } } };
    }
    const began = performance.now();
    const report = {};
    try {
        const run = nativeAv(connections, groups, sources, join(base, "received"), plan,
            (value, programme) => { observation = value; joinedProgramme = programme; report.native_av = programme; }, ({ signal, reportFailure }) => new Promise((done, reject) => {
                if (isolated) {
                    const abort = () => { clearTimeout(timer); reject(signal.reason ?? Error("original parent cancelled")); };
                    const timer = setTimeout(() => {
                        signal.removeEventListener("abort", abort);
                        recordPrivateTargetCleanup(original, { format: "connect/original-private-target-cleanup/1", logical_outcome: "not-submitted", submission_definitively_refused: true, completion_observed: false,
                            transferred:false, original_root_completed:false, original_source_owner_joined:false,
                            worker_started:false, target_worker_joined:true, target_worker_forced:false,
                            target_executable_unchanged:true, target_config_unchanged:true });
                        reportFailure(original);controlJoined = true;reject(original);
                    }, 23000);
                    signal.addEventListener("abort", abort, { once:true });
                    if (signal.aborted) abort();
                    return;
                }
                const abort = () => { clearTimeout(timer); reject(Object.assign(Error("Control owner cancelled at normal read completion"), { kind: "cancelled" })); };
                const timer = setTimeout(() => { signal.removeEventListener("abort", abort); controlJoined = true; done({ final_capture_joined: true }); }, 62500);
                signal.addEventListener("abort", abort, { once: true });
            }));
        let result;
        if (isolated) await assert.rejects(run, failure => failure === original);
        else {
            // Match the conversation runner's assignment after its callback.
            report.native_av = await run;
            result = report.native_av;
        }
        assert.equal(controlJoined, true);
        if (isolated) {
            assert.equal(joinedProgramme.passed, false);
            assert.equal(joinedProgramme.failure_observed, true);
            assert.equal(joinedProgramme.programme_window_observed, true);
            assert.equal(joinedProgramme.original_receive_tasks_joined, true);
            assert.equal(joinedProgramme.movement_requested, true);
        } else {
            assert.equal(result.control_workload.final_capture_joined, true);
            assert.equal(joinedProgramme.programme_window_observed, true);
            assert.equal(report.native_av.programme_window_observed, true);
        }
        assert.equal(observation.original_tasks_joined, true);
        assert.equal(observation.original_tasks_started, 30);
        assert.equal(pendingReads, 0); assert.equal(stoppedReads, 12);
        assert.equal(pendingFeedbackReads, 0); assert.equal(stoppedFeedbackReads, 6);
        assert.ok(performance.now() - began >= (isolated ? 62200 : 62500));
    } finally {
        if (retained) await writeFile(join(base, "observation.json"), JSON.stringify({ controlJoined, stoppedReads, pendingReads, stoppedFeedbackReads, pendingFeedbackReads, observation,
            scope: "Actual lifetime coordinator with packet IO test doubles only; no codec or transport quality." }, null, 2) + "\n");
        else await rm(base, { recursive: true, force: true });
    }
}
test("the real62s AV controller joins final control work after normal packet-read completion", { timeout:72000 }, () => exerciseLifetime());
test("the real AV controller retains all original readers and joined captures through62s after a definitively refused23s submission", { timeout:72000 }, () => exerciseLifetime(true));
