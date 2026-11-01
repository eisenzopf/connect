// Owned native SDK media component. No control mutation, decoder or authority.
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { SdkRtpPacket } from "../../sdk/typescript/dist/index.js";
import { RtpSender, Vp8Assembler } from "../../sdk/typescript/dist/browser.js";
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const pause = ms => new Promise(done => setTimeout(done, Math.max(0, ms)));
const ntp = milliseconds => (BigInt(Math.floor(milliseconds / 1000) + 2208988800) << 32n) |
    BigInt(Math.floor((milliseconds % 1000) * 2 ** 32 / 1000));

export async function nativeAv(connections, rolesBySession, sourceDirectory, outputDirectory, plan) {
    assert.equal(connections.length, 6);
    const manifest = JSON.parse(await readFile(join(sourceDirectory, "manifest.json"), "utf8"));
    assert.equal(manifest.format, "connect/native-av-sources/1");
    assert.equal(manifest.seconds, 62); assert.equal(manifest.observation_seconds, 60);
    await mkdir(outputDirectory, { mode: 0o700 });
    const routes = [];
    // Complete every original owner's media readiness before starting any clock.
    for (const [index, item] of connections.entries()) {
        const roles = rolesBySession[item.sid], previous = roles[(roles.indexOf(item.role) + roles.length - 1) % roles.length];
        const peer = connections.find(v => v.sid === item.sid && v.role === previous);
        const negotiation = await item.connection.media.refresh({ timeoutMs: 8000 });
        assert.equal(negotiation.sourceClock, true);
        const byId = id => { const stream = negotiation.streams.find(v => v.id === id); assert.ok(stream); return stream; };
        const snapshot = await item.connection.refresh({ timeoutMs: 8000 });
        const ownAudio = byId(plan[item.prefix + "-audio"]), ownVideo = byId(plan[item.prefix + "-video"]);
        byId(plan[peer.prefix + "-video"]);
        const sender = (stream, codec) => {
            assert.equal(stream.codec.name.toLowerCase(), codec);
            return new RtpSender({ handle: stream.streamLocalId, streamId: stream.id, connectionId: item.connection.id,
                sessionId: negotiation.binding.sessionId, generation: snapshot.generation, kind: stream.medium, codec,
                payloadType: stream.codec.params.payload_type, direction: stream.direction });
        };
        const raw = await readFile(join(sourceDirectory, "source-" + index + ".json"));
        assert.equal(sha(raw), manifest.sources[index].input_sha256);
        const input = JSON.parse(raw);
        assert.equal(input.audio.length, 3100); assert.equal(input.video.length, 1860);
        routes.push({ index, item, peerIndex: connections.indexOf(peer), input,
            audioSender: sender(ownAudio, "opus"), videoSender: sender(ownVideo, "vp8"),
            sendAudio: item.connection.streams.byId(ownAudio.id).media, sendVideo: item.connection.streams.byId(ownVideo.id).media,
            receiveAudio: item.connection.streams.byId(ownAudio.id).media,
            receiveVideo: item.connection.streams.byId(plan[peer.prefix + "-video"]).media,
            audio: [], video: [], capturedBytes: 0, clocks: { audio: 0, video: 0 }, gaps: 0, lateSendFrames: 0, sent: { audio: 0, video: 0 } });
    }
    const controller = new AbortController(), start = performance.now(), unixStart = Date.now();
    const end = start + 62000, tasks = []; let error, timer;
    async function sending(route, medium) {
        const video = medium === "video", entries = route.input[medium], sender = video ? route.videoSender : route.audioSender,
            stream = video ? route.sendVideo : route.sendAudio;
        let lastClock = -Infinity, submitted = { ssrc: 0, packets: 0, octets: 0 };
        for (let index = 0; index < entries.length && !controller.signal.aborted; index++) {
            const elapsed = video ? entries[index].timestamp / 90 : index * 20;
            await pause(start + elapsed - performance.now());
            if (controller.signal.aborted) return;
            // A delayed source frame is discarded at its original age. It is
            // never relabelled or replayed as current after event-loop delay.
            if (performance.now() - start - elapsed >= 200) { route.lateSendFrames++; continue; }
            const timestamp = video ? entries[index].timestamp : index * 960;
            const encoded = Buffer.from(video ? entries[index].payload : entries[index], "base64");
            let frameCompleted = true;
            for (const packet of sender.packetize(encoded, timestamp)) {
                const remaining = Math.floor(200 - (performance.now() - start - elapsed));
                if (remaining <= 0) { route.lateSendFrames++; frameCompleted = false; break; }
                const sequence = new DataView(packet.buffer, packet.byteOffset).getUint32(4);
                const result = await stream.sendRtp(new SdkRtpPacket(sequence, packet.subarray(8)), { timeoutMs: remaining, signal: controller.signal });
                assert.equal(result.boundary, "local-media-queue"); submitted = sender.submitted(packet);
                route.sent[medium]++;
            }
            if (frameCompleted && elapsed - lastClock >= 500) {
                await stream.sendClock(sender.nextDatagramSequence(), { ...submitted, ntp: ntp(unixStart + elapsed), rtp: timestamp,
                    cname: "g6-native-av-" + route.index }, { timeoutMs: 1000, signal: controller.signal });
                lastClock = elapsed;
            }
        }
    }
    async function receiving(route, medium) {
        const stream = medium === "audio" ? route.receiveAudio : route.receiveVideo, assembler = new Vp8Assembler();
        while (!controller.signal.aborted && performance.now() < end + 100) {
            let value;
            try { value = await stream.receivePacket({ timeoutMs: 500, signal: controller.signal }); }
            catch (failure) {
                if (controller.signal.aborted && failure.kind === "cancelled") return;
                if (failure.kind === "deadline" && failure.certainty === "read-only") continue;
                throw failure;
            }
            const elapsed = performance.now() - start;
            if (value.kind === "gap") { route.gaps += Number(value.droppedPackets); continue; }
            if (value.kind === "clock") { route.clocks[medium]++; continue; }
            assert.equal(value.kind, "rtp"); assert.ok(value.bytes.length <= 1180);
            if (medium === "audio") {
                assert.ok(route.audio.length < 3200);
                route.capturedBytes += value.payload.byteLength;
                assert.ok(route.capturedBytes <= 16 * 1024 * 1024, "bounded-native-media-recording-bytes");
                route.audio.push({ elapsed_ms: elapsed, timestamp: value.header.timestamp, sequence: value.header.sequenceNumber,
                    ssrc: value.header.ssrc, payload: Buffer.from(value.payload).toString("base64") });
            } else {
                const frame = assembler.push({ ...value.header, sequence: value.header.sequenceNumber, payload: value.payload });
                if (frame) {
                    assert.ok(route.video.length < 2000);
                    route.capturedBytes += frame.bytes.byteLength;
                    assert.ok(route.capturedBytes <= 16 * 1024 * 1024, "bounded-native-media-recording-bytes");
                    route.video.push({ elapsed_ms: elapsed, timestamp: frame.timestamp, key: frame.key,
                        payload: Buffer.from(frame.bytes).toString("base64") });
                }
            }
        }
    }
    const launch = task => tasks.push(task.catch(failure => { error ??= failure; controller.abort(); }));
    try {
        for (const route of routes) for (const medium of ["audio", "video"]) {
            launch(receiving(route, medium)); launch(sending(route, medium));
        }
        // The deadline aborts only reads owned by this component; originals and
        // their control/mutation owners are still closed by the caller.
        await Promise.race([new Promise(done => { timer = setTimeout(done, 62200); }), Promise.all(tasks)]);
    } finally {
        clearTimeout(timer); controller.abort(); await Promise.all(tasks);
        for (const route of routes) await writeFile(join(outputDirectory, "received-" + route.index + ".json"), JSON.stringify({
            format: "connect/native-av-received/1", index: route.index, peer_index: route.peerIndex, audio: route.audio, video: route.video,
            captured_codec_bytes: route.capturedBytes, received_clocks: route.clocks, gap_packets: route.gaps, late_send_frames: route.lateSendFrames, submitted_packets: route.sent,
            unix_start_ms: unixStart, observed_seconds: (performance.now() - start) / 1000,
        }) + "\n", { flag: "wx", mode: 0o600 });
    }
    if (error) throw error;
    return { connections: 6, participants: 4, sessions: 2, source_manifest_sha256: sha(await readFile(join(sourceDirectory, "manifest.json"))),
        original_receive_tasks_joined: true, seconds: 62, warmup_seconds: 2, observation_seconds: 60,
        routes: routes.map(r => ({ index: r.index, peer_index: r.peerIndex, received_audio: r.audio.length, received_video: r.video.length,
            gap_packets: r.gaps, late_send_frames: r.lateSendFrames, received_clocks: r.clocks, submitted_packets: r.sent })),
        scope: "Real public SDK native RTP and source-clock transport only. Independent decoder and profile checks are separate; no full-cell qualification." };
}
