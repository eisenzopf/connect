// Failure at the first native write must cancel and join every owned reader.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { nativeAv } from "../lib/transport_av.mjs";

async function checkFailure(firstSubmissionAccepted) {
    const parent = resolve(".local"); await mkdir(parent, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(parent, "transport-av-cancel-"));
    const sources = join(directory, "sources"); await mkdir(sources, { mode: 0o700 });
    const roles = [["one", "two", "three", "four"], ["one", "two"]], plan = {}, connections = [];
    let waiting = 0, aborted = 0, clocksPending = 12;
    let accepted = false;
    const clock = { ssrc: 4294967295, ntp: 17191887989801992191n, rtp: 4294967290,
        packets: 4294967294, octets: 4294967293, cname: "private-synthetic-clock" };
    const failure = Object.assign(Error("synthetic first-write refusal"), { kind: "authority" });
    for (const [sid, group] of roles.entries()) for (const role of group) {
        const prefix = sid + "-" + role, index = connections.length;
        for (const medium of ["audio", "video"]) plan[prefix + "-" + medium] = prefix + "-" + medium;
        connections.push({ sid, role, prefix, index });
    }
    const raw = JSON.stringify({ audio: Array(3100).fill("AQ=="), video: Array.from({ length: 1860 }, (_, index) => ({ timestamp: index * 3000, payload: "AQI=" })) });
    const manifest = { format: "connect/native-av-sources/1", seconds: 62, observation_seconds: 60,
        sources: connections.map(() => ({ input_sha256: createHash("sha256").update(raw).digest("hex") })) };
    await writeFile(join(sources, "manifest.json"), JSON.stringify(manifest));
    const stream = {
        async sendRtp(_packet, options) {
            assert.ok(options.timeoutMs > 0 && options.timeoutMs <= 200, "public-write-cannot-outlive-original-200ms-source-age");
            if (firstSubmissionAccepted && !accepted) {
                accepted = true;
                return { boundary: "local-media-queue" };
            }
            throw failure;
        }, async sendClock() { throw failure; },
        receivePacket({ signal }) {
            if (clocksPending-- > 0) return Promise.resolve({ kind: "clock", report: clock });
            waiting++;
            return new Promise((_, reject) => {
                signal.addEventListener("abort", () => { waiting--; aborted++; reject(Object.assign(Error(), { kind: "cancelled" })); }, { once: true });
            });
        },
    };
    for (const item of connections) {
        await writeFile(join(sources, "source-" + item.index + ".json"), raw);
        const group = roles[item.sid], previous = group[(group.indexOf(item.role) + group.length - 1) % group.length];
        const peer = connections.find(v => v.sid === item.sid && v.role === previous);
        const negotiated = [
            { id: plan[item.prefix + "-audio"], medium: "audio", direction: "sendrecv", streamLocalId: 1, codec: { name: "opus", params: { payload_type: 111 } } },
            { id: plan[item.prefix + "-video"], medium: "video", direction: "sendonly", streamLocalId: 2, codec: { name: "vp8", params: { payload_type: 96 } } },
            { id: plan[peer.prefix + "-video"], medium: "video", direction: "recvonly", streamLocalId: 3, codec: { name: "vp8", params: { payload_type: 96 } } },
        ];
        item.connection = { id: "connection-" + item.index, async refresh() { return { generation: 1 }; },
            media: { async refresh() { return { sourceClock: true, binding: { sessionId: "session-" + item.sid }, streams: negotiated }; } },
            streams: { byId() { return { media: stream }; } } };
    }
    try {
        const started = performance.now();
        await assert.rejects(nativeAv(connections, roles, sources, join(directory, "received"), plan), error => error === failure);
        assert.ok(performance.now() - started < 1000);
        assert.equal(waiting, 0); assert.equal(aborted, 12);
        let submissions = 0;
        for (const item of connections) {
            const result = JSON.parse(await readFile(join(directory, "received", "received-" + item.index + ".json"), "utf8"));
            assert.equal(result.audio.length, 0); assert.equal(result.video.length, 0);
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

test("first send failure joins all twelve native readers and retains original clocks", () => checkFailure(false));
test("clock failure retains only the completed original audio queue receipt and joins readers", () => checkFailure(true));
