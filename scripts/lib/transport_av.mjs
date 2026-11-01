// Owned native SDK packet workers. Control authority and decoding stay with the caller.
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { SdkRtpPacket } from "../../sdk/typescript/dist/index.js";
import { RtpSender, Vp8Assembler } from "../../sdk/typescript/dist/browser.js";
import { ConnectionMediaOwner } from "./transport_media_owner.mjs";
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const pause = ms => new Promise(done => setTimeout(done, Math.max(0, ms)));
const ntp = milliseconds => (BigInt(Math.floor(milliseconds / 1000) + 2208988800) << 32n) |
    BigInt(Math.floor((milliseconds % 1000) * 2 ** 32 / 1000));

export async function nativeAv(connections, rolesBySession, sourceDirectory, outputDirectory, plan, observe = () => {}, during) {
    assert.equal(connections.length, 6);
    assert.ok(during === undefined || typeof during === "function");
    const manifest = JSON.parse(await readFile(join(sourceDirectory, "manifest.json"), "utf8"));
    assert.equal(manifest.format, "connect/native-av-sources/2");
    assert.equal(manifest.seconds, 62); assert.equal(manifest.observation_seconds, 60);
    await mkdir(outputDirectory, { mode: 0o700 });
    const routes = [], setupStarted = performance.now();
    let setupRoute = 0, setupStage = "media-refresh";
    // Complete every original owner's media readiness before starting any clock.
    try {
        for (const [index, item] of connections.entries()) {
            const roles = rolesBySession[item.sid], previous = roles[(roles.indexOf(item.role) + roles.length - 1) % roles.length];
            const peer = connections.find(v => v.sid === item.sid && v.role === previous);
            setupRoute = index; setupStage = "media-refresh";
            const negotiation = await item.connection.media.refresh({ timeoutMs: 8000 });
            assert.equal(negotiation.sourceClock, true);
            const byId = id => { const stream = negotiation.streams.find(v => v.id === id); assert.ok(stream); return stream; };
            setupStage = "connection-refresh";
            const snapshot = await item.connection.refresh({ timeoutMs: 8000 });
            const ownAudio = byId(plan[item.prefix + "-audio"]), ownVideo = byId(plan[item.prefix + "-video"]);
            byId(plan[peer.prefix + "-video"]);
            const sender = (stream, codec) => {
                assert.equal(stream.codec.name.toLowerCase(), codec);
                return new RtpSender({ handle: stream.streamLocalId, streamId: stream.id, connectionId: item.connection.id,
                    sessionId: negotiation.binding.sessionId, generation: snapshot.generation, kind: stream.medium, codec,
                    payloadType: stream.codec.params.payload_type, direction: stream.direction });
            };
            setupStage = "source-input";
            const raw = await readFile(join(sourceDirectory, "source-" + index + ".json"));
            assert.equal(sha(raw), manifest.sources[index].input_sha256);
            const input = JSON.parse(raw);
            assert.equal(input.audio.length, 3100); assert.equal(input.video.length, 1860);
            routes.push({ index, item, peerIndex: connections.indexOf(peer), input,
                audioSender: sender(ownAudio, "opus"), videoSender: sender(ownVideo, "vp8"),
                sendAudio: item.connection.streams.byId(ownAudio.id).media, sendVideo: item.connection.streams.byId(ownVideo.id).media,
                receiveAudio: item.connection.streams.byId(ownAudio.id).media,
                receiveVideo: item.connection.streams.byId(plan[peer.prefix + "-video"]).media,
                audio: [], video: [], submittedAudio: [], clockReports: { audio: [], video: [] }, stages: { send: {}, receive: {} },
                capturedBytes: 0, clocks: { audio: 0, video: 0 }, gaps: 0, lateSendFrames: 0, sent: { audio: 0, video: 0 } });
        }
    } catch (failure) {
        observe({ format: "connect/native-av-work-observation/1", original_tasks_joined: true,
            original_tasks_started: 0, elapsed_ms: performance.now() - setupStarted,
            failure: { route_index: setupRoute, medium: null, direction: "setup", stage: setupStage },
            prepared_routes: routes.length, routes: [],
            scope: "Original setup failure before media tasks or source clocks start. No retries, payloads, identities or quality qualification." });
        throw failure;
    }
    const controller = new AbortController(), start = performance.now(), unixStart = Date.now();
    const end = start + 62000, tasks = []; let error, workFailure, timer;
    let controlResult;
    for (const route of routes) route.owner = new ConnectionMediaOwner(controller.signal);
    async function sending(route, medium) {
        const video = medium === "video", entries = route.input[medium], sender = video ? route.videoSender : route.audioSender,
            stream = video ? route.sendVideo : route.sendAudio;
        let lastClock = -Infinity, submitted = { ssrc: 0, packets: 0, octets: 0 };
        for (let index = 0; index < entries.length && !route.owner.signal.aborted; index++) {
            const elapsed = video ? entries[index].timestamp / 90 : index * 20;
            await pause(start + elapsed - performance.now());
            if (route.owner.signal.aborted) return;
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
                const submittedAt = performance.now() - start;
                route.stages.send[medium] = "rtp";
                const result = await stream.sendRtp(new SdkRtpPacket(sequence, packet.subarray(8)), { timeoutMs: remaining, signal: route.owner.signal });
                assert.equal(result.boundary, "local-media-queue"); submitted = sender.submitted(packet);
                if (!video) {
                    assert.ok(route.submittedAudio.length < 3100);
                    const header = new DataView(packet.buffer, packet.byteOffset + 8);
                    route.submittedAudio.push({ source_offset_ms: elapsed, submit_started_ms: submittedAt,
                        submit_completed_ms: performance.now() - start, timestamp: header.getUint32(4),
                        sequence: header.getUint16(2), ssrc: header.getUint32(8) });
                }
                route.sent[medium]++;
            }
            if (frameCompleted && elapsed - lastClock >= 500) {
                route.stages.send[medium] = "clock";
                await stream.sendClock(sender.nextDatagramSequence(), { ...submitted, ntp: ntp(unixStart + elapsed), rtp: timestamp,
                    cname: "g6-native-av-" + route.index }, { timeoutMs: 1000, signal: route.owner.signal });
                lastClock = elapsed;
            }
        }
    }
    async function receiving(route, medium) {
        const stream = medium === "audio" ? route.receiveAudio : route.receiveVideo, assembler = new Vp8Assembler();
        while (!route.owner.signal.aborted && performance.now() < end + 100) {
            let value;
            route.stages.receive[medium] = "packet";
            try { value = await stream.receivePacket({ timeoutMs: 500, signal: route.owner.signal }); }
            catch (failure) {
                if (route.owner.signal.aborted && failure.kind === "cancelled") return;
                if (failure.kind === "deadline" && failure.certainty === "read-only") continue;
                throw failure;
            }
            const elapsed = performance.now() - start;
            if (value.kind === "gap") { route.gaps += Number(value.droppedPackets); continue; }
            if (value.kind === "clock") {
                assert.ok(route.clockReports[medium].length < 512, "bounded-native-clock-observations");
                const report = value.report;
                route.clockReports[medium].push({ elapsed_ms: elapsed, ssrc: report.ssrc,
                    ntp: report.ntp.toString(), rtp: report.rtp, packets: report.packets, octets: report.octets });
                route.clocks[medium]++; continue;
            }
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
                        ssrc: value.header.ssrc,
                        payload: Buffer.from(frame.bytes).toString("base64") });
                }
            }
        }
    }
    const launch = (task, route, medium, direction) => tasks.push(task.catch(failure => {
        if (!error) {
            if (route.owner.signal.aborted && failure.kind === "cancelled") return;
            error = failure;
            workFailure = { route_index: route.index, medium, direction, stage: route.stages[direction][medium] ?? "pending" };
        }
        controller.abort();
    }));
    try {
        for (const route of routes) for (const medium of ["audio", "video"]) {
            launch(route.owner.run(() => receiving(route, medium)), route, medium, "receive");
            launch(route.owner.run(() => sending(route, medium)), route, medium, "send");
        }
        if (during) tasks.push(Promise.resolve().then(() => during(Object.freeze({
            signal: controller.signal, programStartUnixUs: unixStart * 1000,
            routes: Object.freeze(routes.map(route => Object.freeze({ index: route.index,
                peerIndex: route.peerIndex, connection: route.item.connection,
                sessionIndex: route.item.sid, role: route.item.role,
                // Retire only this Connection's four packet tasks. A second
                // Session may still belong to the same live SDK/helper.
                stopAndJoin: () => route.owner.stopAndJoin(),
            }))),
        }))).then(value => { controlResult = value; }, failure => {
            if (!error) { error = failure; workFailure = { route_index: null, medium: null, direction: "control", stage: "during-workload" }; }
            controller.abort();
        }));
        // The deadline aborts only reads owned by this component; originals and
        // their control/mutation owners are still closed by the caller.
        await Promise.race([new Promise(done => { timer = setTimeout(done, 62200); }), Promise.all(tasks)]);
    } finally {
        clearTimeout(timer); controller.abort(); await Promise.all(tasks);
        await Promise.all(routes.map(route => route.owner.stopAndJoin().catch(() => {})));
        for (const route of routes) await writeFile(join(outputDirectory, "received-" + route.index + ".json"), JSON.stringify({
            format: "connect/native-av-received/1", index: route.index, peer_index: route.peerIndex, audio: route.audio, video: route.video,
            captured_codec_bytes: route.capturedBytes, received_clocks: route.clocks, gap_packets: route.gaps, late_send_frames: route.lateSendFrames, submitted_packets: route.sent,
            clock_reports: route.clockReports, submitted_audio: route.submittedAudio,
            unix_start_ms: unixStart, observed_seconds: (performance.now() - start) / 1000,
        }) + "\n", { flag: "wx", mode: 0o600 });
        observe({ format: "connect/native-av-work-observation/1", original_tasks_joined: true,
            original_tasks_started: routes.length * 4, control_task_started: Boolean(during), elapsed_ms: performance.now() - start, failure: workFailure ?? null,
            routes: routes.map(route => ({ index: route.index, received_audio: route.audio.length,
                received_video: route.video.length, submitted_packets: { ...route.sent },
                received_clocks: { ...route.clocks }, gap_packets: route.gaps, late_send_frames: route.lateSendFrames })),
            scope: "Bounded original work counters after all original tasks join. No payloads, identities, media clocks, decoded quality or cell qualification." });
    }
    if (error) throw error;
    return { connections: 6, participants: 4, sessions: 2, source_manifest_sha256: sha(await readFile(join(sourceDirectory, "manifest.json"))),
        original_receive_tasks_joined: true, seconds: 62, warmup_seconds: 2, observation_seconds: 60,
        routes: routes.map(r => ({ index: r.index, peer_index: r.peerIndex, received_audio: r.audio.length, received_video: r.video.length,
            gap_packets: r.gaps, late_send_frames: r.lateSendFrames, received_clocks: r.clocks, submitted_packets: r.sent })),
        ...(during ? { control_workload: controlResult } : {}),
        scope: "Real public SDK native RTP and source-clock transport only. Independent decoder and profile checks are separate; no full-cell qualification." };
}
