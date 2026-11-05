// Actual nativeAv and C encoder; public SDK packet IO is controlled locally.
// These short failure/retirement checks do not qualify a full media workload.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { Vp8Assembler } from "../../sdk/typescript/dist/browser.js";
import { nativeAv } from "../lib/transport_av.mjs";
import { addLiveVideoFixture } from "./live_video_fixture.mjs";

const gone = pid => assert.throws(() => process.kill(pid, 0), error => error.code === "ESRCH");
async function check(setupFailure) {
    const parent = resolve(".local/gate-6/live-source-integration");
    await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "owned-"));
    let pids;
    try {
        const sources = join(directory, "sources"); await mkdir(sources, { mode: 0o700 });
        const roles = [["one", "two", "three", "four"], ["one", "two"]], plan = {}, connections = [];
        for (const [sid, group] of roles.entries()) for (const role of group) {
            const prefix = sid + "-" + role;
            connections.push({ sid, role, prefix });
            for (const medium of ["audio", "video"]) plan[prefix + "-" + medium] = prefix + "-" + medium;
        }
        // Reference payloads deliberately cannot be VP8. Only the live encoder
        // can supply the complete keyframes observed below.
        const raw = JSON.stringify({ audio: Array(3100).fill("AQ=="),
            video: Array.from({ length: 1860 }, (_, index) => ({ timestamp: index * 3000, payload: "AQI=" })) });
        const manifest = { format: "connect/native-av-sources/2", seconds: 62, observation_seconds: 60,
            sources: connections.map(() => ({ input_sha256: createHash("sha256").update(raw).digest("hex") })) };
        pids = await addLiveVideoFixture(sources, manifest);
        await writeFile(join(sources, "manifest.json"), JSON.stringify(manifest));
        const frames = connections.map(() => []), assemblers = connections.map(() => new Vp8Assembler());
        let waiting = 0, waitingFeedback = 0, observations = [], retired = false;
        const failure = Error(setupFailure ? "controlled second setup refusal" : "controlled completion after encoder retirement");
        let resolveFrames;
        const firstFrames = new Promise(resolve => { resolveFrames = resolve; });
        for (const [index, item] of connections.entries()) {
            await writeFile(join(sources, "source-" + index + ".json"), raw);
            const group = roles[item.sid], previous = group[(group.indexOf(item.role) + group.length - 1) % group.length];
            const peer = connections.find(value => value.sid === item.sid && value.role === previous);
            const stream = (prefix, medium, direction, handle) => ({ id: plan[prefix + "-" + medium], medium, direction,
                ...(medium === "video" ? { videoFeedback: { profile: "rtcp-pli-epoch/1" } } : {}),
                streamLocalId: handle, codec: { name: medium === "audio" ? "opus" : "vp8",
                    params: { payload_type: medium === "audio" ? 111 : 96 } } });
            const media = id => ({
                async sendRtp(packet, { timeoutMs }) {
                    assert.ok(timeoutMs > 0 && timeoutMs <= 200);
                    if (id === plan[item.prefix + "-video"]) {
                        const frame = assemblers[index].push({ ...packet.header,
                            sequence: packet.header.sequenceNumber, payload: packet.payload });
                        if (frame) {
                            frames[index].push(frame);
                            if (frames.every(values => values.length >= 2)) resolveFrames();
                        }
                    }
                    return { boundary: "local-media-queue" };
                },
                async sendClock() {},
                receiveVideoFeedback({ signal }) {
                    waitingFeedback++;
                    return new Promise((_, reject) => {
                        const abort = () => {
                            waitingFeedback--; signal.removeEventListener("abort", abort);
                            reject(Object.assign(Error("feedback stopped"), { kind: "cancelled", certainty: "read-only" }));
                        };
                        if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
                    });
                },
                async consumeVideoFeedback() { assert.fail("No feedback hint was issued by this fixture"); },
                receivePacket({ signal }) {
                    waiting++;
                    return new Promise((_, reject) => {
                        const abort = () => { waiting--; reject(Object.assign(Error("stopped read"), { kind: "cancelled" })); };
                        if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
                    });
                },
            });
            item.connection = { id: "connection-" + index, async refresh() { return { generation: 1 }; },
                media: { async refresh() {
                    if (setupFailure && index === 1) throw failure;
                    return { sourceClock: true, binding: { sessionId: "session-" + item.sid }, streams: [
                        stream(item.prefix, "audio", "sendrecv", 1), stream(item.prefix, "video", "sendonly", 2),
                        stream(peer.prefix, "video", "recvonly", 3)] };
                } }, streams: { byId(id) { return { media: media(id) }; } } };
        }
        await assert.rejects(nativeAv(connections, roles, sources, join(directory, "received"), plan,
            observation => { assert.equal(waiting, 0); assert.equal(waitingFeedback, 0); observations.push(observation); }, setupFailure ? undefined : async work => {
                let timer;
                try {
                    await Promise.race([firstFrames, new Promise((_, reject) => { timer = setTimeout(() => reject(Error("fresh frames deadline")), 2000); })]);
                } finally { clearTimeout(timer); }
                const children = await pids(); assert.equal(children.length, 6);
                const stopped = await work.routes[0].stopAndJoin();
                assert.equal(stopped.original_packet_tasks_joined, 5);
                gone(children[0]);
                for (const pid of children.slice(1)) process.kill(pid, 0);
                assert.equal(work.signal.aborted, false); assert.equal(waiting, 10);
                retired = true;
                throw failure;
            }), error => error === failure);
        assert.equal(observations.length, 1);
        assert.equal(observations[0].fixture_payload_arrays_released, true);
        assert.equal(observations[0].encoder_processes_joined, setupFailure ? 1 : 6);
        const children = await pids(); assert.equal(children.length, setupFailure ? 1 : 6);
        for (const pid of children) gone(pid);
        if (!setupFailure) {
            assert.equal(retired, true);
            for (const [index, values] of frames.entries()) {
                assert.ok(values.length >= 2); assert.equal(values[0].key, true);
                for (const frame of values) assert.equal(frame.timestamp % 3000, 0);
                assert.ok(values[1].timestamp > values[0].timestamp);
                const recorded = JSON.parse(await readFile(join(directory, "received", "received-" + index + ".json")));
                assert.equal(recorded.encoder_process.joined, true);
            }
        }
    } finally { await rm(directory, { recursive: true, force: true }); }
}

test("native sender uses fresh current frames and joins only the retired Connection encoder", { timeout: 30000 }, () => check(false));
test("native sender setup refusal joins the encoder opened for the previous Connection", { timeout: 30000 }, () => check(true));
