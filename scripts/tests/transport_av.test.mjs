// Failure at the first native write must cancel and join every owned reader.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { addLiveVideoFixture } from "./live_video_fixture.mjs";
import { nativeAv } from "../lib/transport_av.mjs";
import { assertNativeCounterObservation, OriginalFailureJoinClock } from "./native_av_observation_assertions.mjs";

async function checkFailure(context, firstSubmissionAccepted, setupFailure = false, withControl = false, cleanupFailure = false) {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "transport-av-cancel-"));
    const sources = join(directory, "sources"); await mkdir(sources, { mode: 0o700 });
    const roles = [["one", "two", "three", "four"], ["one", "two"]], plan = {}, connections = [];
    let waiting = 0, waitingFeedback = 0, aborted = 0, clocksPending = 12;
    let accepted = false, timing;
    const clock = { ssrc: 4294967295, ntp: 17191887989801992191n, rtp: 4294967290,
        packets: 4294967294, octets: 4294967293, cname: "private-synthetic-clock" };
    const failure = Object.assign(Error("synthetic first-write refusal"), { kind: "authority" });
    const originalFailure = () => { timing.beginFailure(); return failure; };
    for (const [sid, group] of roles.entries()) for (const role of group) {
        const prefix = sid + "-" + role, index = connections.length;
        for (const medium of ["audio", "video"]) plan[prefix + "-" + medium] = prefix + "-" + medium;
        connections.push({ sid, role, prefix, index });
    }
    const raw = JSON.stringify({ audio: Array(3100).fill("AQ=="), video: Array.from({ length: 1860 }, (_, index) => ({ timestamp: index * 3000, payload: "AQI=" })) });
    const manifest = { format: "connect/native-av-sources/2", seconds: 62, observation_seconds: 60,
        sources: connections.map(() => ({ input_sha256: createHash("sha256").update(raw).digest("hex") })) };
    await addLiveVideoFixture(sources, manifest);
    await writeFile(join(sources, "manifest.json"), JSON.stringify(manifest));
    const pendingReads = new Set();
    const cleanupRefusal = Object.assign(Error("publisher stopped during control cleanup"), { kind: "receipt-unavailable", certainty: "refused" });
    const stream = {
        async sendRtp(_packet, options) {
            assert.ok(options.timeoutMs > 0 && options.timeoutMs <= 200, "public-write-cannot-outlive-original-200ms-source-age");
            if (withControl) return new Promise((_, reject) => {
                options.signal.addEventListener("abort", () => reject(Object.assign(Error(), { kind: "cancelled", certainty: "read-only" })), { once: true });
            });
            if (firstSubmissionAccepted && !accepted) {
                accepted = true;
                return { boundary: "local-media-queue" };
            }
            throw originalFailure();
        }, async sendClock() { throw originalFailure(); },
        receiveVideoFeedback({ signal }) {
            waitingFeedback++;
            return new Promise((_, reject) => {
                const abort = () => {
                    waitingFeedback--; signal.removeEventListener("abort", abort);
                    reject(Object.assign(Error("feedback owner stopped"), { kind: "cancelled", certainty: "read-only" }));
                };
                if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
            });
        },
        async consumeVideoFeedback() { assert.fail("No feedback hint was issued by this fixture"); },
        receivePacket({ signal }) {
            if (clocksPending-- > 0) return Promise.resolve({ kind: "clock", report: clock });
            waiting++;
            return new Promise((_, reject) => {
                const fail = failure => {
                    if (!pendingReads.delete(fail)) return;
                    waiting--; signal.removeEventListener("abort", abort); reject(failure);
                };
                const abort = () => {
                    aborted++; fail(Object.assign(Error(), { kind: "cancelled", certainty: "read-only" }));
                };
                pendingReads.add(fail);
                if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
            });
        },
    };
    for (const item of connections) {
        await writeFile(join(sources, "source-" + item.index + ".json"), raw);
        const group = roles[item.sid], previous = group[(group.indexOf(item.role) + group.length - 1) % group.length];
        const peer = connections.find(v => v.sid === item.sid && v.role === previous);
        const negotiated = [
            { id: plan[item.prefix + "-audio"], medium: "audio", direction: "sendrecv", streamLocalId: 1, codec: { name: "opus", params: { payload_type: 111 } } },
            { id: plan[item.prefix + "-video"], medium: "video", direction: "sendonly", streamLocalId: 2,
                videoFeedback: { profile: "rtcp-pli-epoch/1" }, codec: { name: "vp8", params: { payload_type: 96 } } },
            { id: plan[peer.prefix + "-video"], medium: "video", direction: "recvonly", streamLocalId: 3,
                videoFeedback: { profile: "rtcp-pli-epoch/1" }, codec: { name: "vp8", params: { payload_type: 96 } } },
        ];
        item.negotiated = negotiated; item.revision = 1;
        item.connection = { id: "connection-" + item.index, async refresh() { return { generation: 1 }; },
            media: { async refresh() { if (setupFailure && item.index === 1) throw originalFailure(); return { sourceClock: true, streamRevision: item.revision, binding: { sessionId: "session-" + item.sid }, streams: item.negotiated }; } },
            streams: { byId() { return { media: stream }; } } };
    }
    try {
        timing = new OriginalFailureJoinClock();
        const observations = [];
        await assert.rejects(nativeAv(connections, roles, sources, join(directory, "received"), plan,
            observation => { assert.equal(waiting, 0); assert.equal(waitingFeedback, 0); observations.push(observation); timing.joined(); }, withControl ? async workload => {
                await new Promise(setImmediate);
                assert.equal(waiting, 12);
                assert.ok(Number.isSafeInteger(workload.programStartUnixUs));
                assert.equal(workload.routes[0].role, workload.routes[4].role);
                assert.ok(Number.isFinite(workload.startedAt) && workload.startedAt >= 0);
                assert.ok(workload.routes.every(route => route.generation === 1));
                assert.notEqual(workload.routes[0].sessionIndex, workload.routes[4].sessionIndex);
                timing.beginFailure(); // First actual cancellation owned by this control fixture.
                const joined = await workload.routes[0].stopAndJoin();
                assert.equal(joined.original_packet_tasks_joined, 5);
                assert.equal(joined.shared_sdk_closed, false);
                assert.equal(workload.signal.aborted, false);
                assert.equal(waiting, 10); assert.equal(aborted, 2);
                let selectionCalls = 0;
                const target = connections[1];
                await workload.routes[1].switchIncomingVideo("original-camera-selection", "new-camera", async () => {
                    selectionCalls++;
                    return { referenceJson: () => ({ operation_id: "original-camera-selection", command: "stream.subscribe",
                        scope: { connection_id: target.connection.id } }), async wait() {
                        target.revision = 2;
                        target.negotiated = target.negotiated.filter(s => s.direction !== "recvonly").concat({
                            id: "new-camera", medium: "video", direction: "recvonly", streamLocalId: 4,
                            videoFeedback: { profile: "rtcp-pli-epoch/1" },
                            codec: { name: "vp8", params: { payload_type: 96 } },
                        });
                        return { snapshot: { state: "completed" }, value: { connection: target.connection,
                            connectionGeneration: 1, streamRevision: 2,
                            bindings: target.negotiated.map(parameters => ({ parameters, connectionGeneration: 1, streamGeneration: 1 })) } };
                    } };
                });
                await new Promise(setImmediate);
                assert.equal(selectionCalls, 1);
                assert.equal(workload.signal.aborted, false);
                assert.equal(waiting, 10); assert.equal(aborted, 3);
                if (cleanupFailure) {
                    // This is the control failure boundary, before target cleanup
                    // can close a publisher while the control task still joins.
                    timing.beginFailure();
                    workload.reportFailure?.(failure);
                    for (const fail of [...pendingReads]) fail(cleanupRefusal);
                    await new Promise(setImmediate);
                }
                throw originalFailure();
            } : undefined), error => error === failure);
        assert.equal(observations.length, 1);
        const observed = observations[0];
        context.diagnostic(JSON.stringify(timing.assertBound()));
        assertNativeCounterObservation(observed);
        assert.equal(observed.original_tasks_joined, true);
        assert.equal(observed.fixture_payload_arrays_released, true);
        if (setupFailure) {
            assert.deepEqual(observed.failure, { route_index: 1, medium: null, direction: "setup", stage: "media-refresh" });
            assert.equal(observed.original_tasks_started, 0); assert.equal(observed.prepared_routes, 1);
            assert.deepEqual(observed.routes, []); assert.equal(waiting, 0); assert.equal(aborted, 0);
            assert.equal(JSON.stringify(observed).includes("synthetic first-write refusal"), false);
            return;
        }
        assert.equal(observed.original_tasks_started, 30);
        assert.deepEqual(observed.failure, withControl ? { route_index: null, medium: null,
            direction: "control", stage: "during-workload" } : { route_index: 0, medium: "audio", direction: "send",
            stage: firstSubmissionAccepted ? "clock" : "rtp" });
        assert.equal(observed.routes.length, 6);
        assert.equal(observed.routes.reduce((sum, r) => sum + r.submitted_packets.audio, 0), Number(firstSubmissionAccepted));
        for (const hidden of ["private-synthetic-clock", "synthetic first-write refusal", "connection-"])
            assert.equal(JSON.stringify({ ...observed, scope: undefined }).includes(hidden), false);
        assert.equal(waiting, 0); assert.equal(aborted, 12 + Number(withControl));
        let submissions = 0;
        for (const item of connections) {
            const result = JSON.parse(await readFile(join(directory, "received", "received-" + item.index + ".json"), "utf8"));
            const counters = observed.routes[item.index];
            assert.equal(counters.received_audio, result.audio.length);
            assert.equal(counters.received_video, result.video.length);
            assert.deepEqual(counters.received_clocks, result.received_clocks);
            assert.deepEqual(counters.submitted_packets, result.submitted_packets);
            assert.equal(result.audio.length, 0); assert.equal(result.video.length, 0);
            assert.equal(result.video_reader.completed_switches, Number(withControl && item.index === 1));
            submissions += result.submitted_audio.length;
            for (const receipt of result.submitted_audio) {
                assert.equal(receipt.timestamp, 0);
                assert.equal(receipt.source_offset_ms, 0);
                assert.ok(receipt.submit_completed_ms >= receipt.submit_started_ms);
                assert.ok(Number.isInteger(receipt.sequence) && receipt.sequence >= 0 && receipt.sequence <= 65535);
                assert.ok(Number.isInteger(receipt.ssrc) && receipt.ssrc >= 0 && receipt.ssrc < 2 ** 32);
                assert.equal(result.submitted_packets.audio, 1);
            }
            for (const medium of ["audio", "video"]) {
                assert.equal(result.received_clocks[medium], 1);
                const [report] = result.clock_reports[medium];
                assert.ok(report.elapsed_ms >= 0);
                assert.deepEqual({ ...report, elapsed_ms: 0 }, { elapsed_ms: 0, ssrc: clock.ssrc,
                    ntp: clock.ntp.toString(), rtp: clock.rtp, packets: clock.packets, octets: clock.octets });
                assert.equal("cname" in report, false);
            }
        }
        assert.equal(submissions, Number(firstSubmissionAccepted));
    } finally { await rm(directory, { recursive: true, force: true }); }
}

test("first send failure joins all twelve native readers and retains original clocks", context => checkFailure(context, false));
test("clock failure retains only the completed original audio queue receipt and joins readers", context => checkFailure(context, true));

test("original media setup failure remains explicit before any source clock or media task starts", context => checkFailure(context, false, true));

test("in-workload control retires one Connection, rebinds a captured camera and preserves the shared participant second Session", context => checkFailure(context, false, false, true));

test("the original control failure survives publisher refusals raised during its joined cleanup", context => checkFailure(context, false, false, true, true));
