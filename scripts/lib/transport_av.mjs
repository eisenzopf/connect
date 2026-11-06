import { isolatedProgrammeFailure, emitOriginalObservation } from "./transport_failure_isolation.mjs";
import { monotonicNow, startProgrammeClock } from "./transport_programme_clock.mjs";
// Owned native SDK packet workers. Control authority and decoding stay with the caller.
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir, lstat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { SdkRtpPacket } from "../../sdk/typescript/dist/index.js";
import { RtpSender } from "../../sdk/typescript/dist/browser.js";
import { LiveVp8Encoder } from "./live_vp8_encoder.mjs";
import { ConnectionMediaOwner } from "./transport_media_owner.mjs";
import { SwitchingMediaReader } from "./transport_media_reader.mjs";
import { joinMediaProgramme } from "./transport_programme_lifetime.mjs";
import { PublisherVideoFeedback, VideoReceiveHistory } from "./transport_video_recovery.mjs";
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
async function fileSha(path, size) {
    const before = await lstat(path), limit = size ?? 16 * 1024 * 1024;
    assert.ok(before.isFile() && !before.isSymbolicLink() && before.size <= limit);
    if (size !== undefined) assert.equal(before.size, size);
    const hash = createHash("sha256");
    let bytes = 0;
    for await (const chunk of createReadStream(path)) {
        bytes += chunk.length; assert.ok(bytes <= limit); hash.update(chunk);
    }
    const after = await lstat(path);
    for (const key of ["dev", "ino", "size", "mode", "mtimeMs", "ctimeMs"]) assert.equal(after[key], before[key]);
    assert.equal(bytes, before.size);
    return hash.digest("hex");
}
const pause = ms => new Promise(done => setTimeout(done, Math.max(0, ms)));
const ntp = milliseconds => (BigInt(Math.floor(milliseconds / 1000) + 2208988800) << 32n) |
    BigInt(Math.floor((milliseconds % 1000) * 2 ** 32 / 1000));
const payloadArrays = route => [route.input.audio, route.input.video, route.audio, route.video,
    route.submittedAudio, route.clockReports.audio, route.clockReports.video];
// Call only after the original readers, senders, control work and encoders join.
// Clear the arrays themselves so a retained fixture closure cannot keep payloads.
function releasePayloadArrays(route) { for (const array of payloadArrays(route)) array.length = 0; }

export async function nativeAv(connections, rolesBySession, sourceDirectory, outputDirectory, plan, observe = () => {}, during) {
    assert.equal(connections.length, 6);
    assert.ok(during === undefined || typeof during === "function");
    const manifest = JSON.parse(await readFile(join(sourceDirectory, "manifest.json"), "utf8"));
    assert.equal(manifest.format, "connect/native-av-sources/2");
    assert.equal(manifest.seconds, 62); assert.equal(manifest.observation_seconds, 60);
    await mkdir(outputDirectory, { mode: 0o700 });
    const routes = [], setupStarted = monotonicNow();
    let setupRoute = 0, setupStage = "media-refresh";
    // Complete every original owner's media readiness before starting any clock.
    try {
        assert.equal(manifest.live_encoder?.file, "live-vp8-encoder");
        const executable = resolve(sourceDirectory, manifest.live_encoder.file);
        assert.equal(await fileSha(executable), manifest.live_encoder.executable_sha256);
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
            const incomingVideo = byId(plan[peer.prefix + "-video"]);
            assert.ok(ownVideo.videoFeedback && incomingVideo.videoFeedback,
                "full media workload requires selected native video feedback on both directions");
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
            const sourceVideoFrames = input.video.length;
            // The live encoder reads original I420 frames. These pre-encoded
            // reference payloads only supplied a validated schedule length;
            // retaining six copies for the programme serves no live consumer.
            input.video.length = 0;
            const route = { index, item, peerIndex: connections.indexOf(peer), input, sourceVideoFrames,
                generation: snapshot.generation,
                feedbackSelected: Boolean(ownVideo.videoFeedback),
                audioSender: sender(ownAudio, "opus"), videoSender: sender(ownVideo, "vp8"),
                sendAudio: item.connection.streams.byId(ownAudio.id).media, sendVideo: item.connection.streams.byId(ownVideo.id).media,
                receiveAudio: item.connection.streams.byId(ownAudio.id).media,
                receiveVideo: item.connection.streams.byId(plan[peer.prefix + "-video"]).media,
                audio: [], video: [], submittedAudio: [], clockReports: { audio: [], video: [] }, stages: { send: {}, receive: {} },
                capturedBytes: 0, clocks: { audio: 0, video: 0 }, gaps: 0, lateSendFrames: 0, sent: { audio: 0, video: 0 } };
            routes.push(route);
            setupStage = "live-video-source";
            assert.equal(manifest.sources[index].raw_video_file, "source-" + index + ".i420");
            const rawVideo = resolve(sourceDirectory, manifest.sources[index].raw_video_file);
            assert.equal(await fileSha(rawVideo, 1860 * 640 * 360 * 3 / 2), manifest.sources[index].raw_video_sha256);
            route.videoEncoder = await LiveVp8Encoder.open(executable, rawVideo, 1860);
        }
    } catch (failure) {
        await Promise.all(routes.map(async route => {
            if (route.videoEncoder) route.encoderJoin = await route.videoEncoder.stopAndJoin();
        }));
        for (const route of routes) releasePayloadArrays(route);
        emitOriginalObservation(observe, [{ format: "connect/native-av-work-observation/1", original_tasks_joined: true,
            fixture_payload_arrays_released: routes.every(route => payloadArrays(route).every(array => array.length === 0)),
            original_tasks_started: 0, elapsed_ms: monotonicNow() - setupStarted,
            failure: { route_index: setupRoute, medium: null, direction: "setup", stage: setupStage },
            prepared_routes: routes.length, routes: [],
            encoder_processes_joined: routes.filter(route => route.encoderJoin?.joined).length,
            scope: "Original setup failure before media tasks or source clocks start. No retries, payloads, identities or quality qualification." }], failure);
        throw failure;
    }
    const controller = new AbortController(), programClock = startProgrammeClock(),
        start = Number(BigInt(programClock.origin_ns)) / 1e6, unixStart = Date.now();
    const end = start + 62000, tasks = [], packetTasks = []; let error, workFailure;
    let controlResult, controlTask, routeResults, programmeWindowObserved = false;
    for (const route of routes) {
        route.owner = new ConnectionMediaOwner(controller.signal);
        const peer = connections[route.peerIndex];
        route.videoReader = new SwitchingMediaReader(plan[peer.prefix + "-video"], route.receiveVideo, route.owner.signal);
        route.videoHistory = new VideoReceiveHistory(route.feedbackSelected);
        if (route.feedbackSelected) route.videoFeedback = new PublisherVideoFeedback(route.sendVideo, route.owner.signal);
    }
    async function sending(route, medium) {
        let failed = false;
        try { await sendingFrames(route, medium); }
        catch (failure) { failed = true; throw failure; }
        finally {
            if (medium === "video") {
                route.encoderJoin = await route.videoEncoder.stopAndJoin();
                if (!failed && !route.owner.signal.aborted) assert.equal(route.encoderJoin.clean, true, "video encoder clean shutdown");
            }
        }
    }
    async function sendingFrames(route, medium) {
        const video = medium === "video", entries = video ? undefined : route.input.audio,
            frameCount = video ? route.sourceVideoFrames : entries.length, sender = video ? route.videoSender : route.audioSender,
            stream = video ? route.sendVideo : route.sendAudio;
        let forceKeyframe = true;
        let lastClock = -Infinity, submitted = { ssrc: 0, packets: 0, octets: 0 };
        for (let index = 0; index < frameCount && !route.owner.signal.aborted; index++) {
            const elapsed = video ? index * 1000 / 30 : index * 20;
            await pause(start + elapsed - monotonicNow());
            if (route.owner.signal.aborted) return;
            // A delayed source frame is discarded at its original age. It is
            // never relabelled or replayed as current after event-loop delay.
            if (monotonicNow() - start - elapsed >= 200) { route.lateSendFrames++; continue; }
            const timestamp = video ? index * 3000 : index * 960;
            let encoded;
            if (video) {
                let remaining = Math.min(200, Math.floor(200 - (monotonicNow() - start - elapsed)));
                if (remaining <= 0) { route.lateSendFrames++; continue; }
                route.stages.send[medium] = "encode-original-frame";
                let frame;
                try {
                    const permit = await route.videoFeedback?.take(submitted.ssrc, Math.min(100, remaining));
                    remaining = Math.min(200, Math.floor(200 - (monotonicNow() - start - elapsed)));
                    if (remaining <= 0) { route.lateSendFrames++; continue; }
                    frame = await route.videoEncoder.frame(index, forceKeyframe || Boolean(permit?.isFresh()),
                        { signal: route.owner.signal, timeoutMs: Math.min(100, remaining) });
                } catch (failure) {
                    // A stopped owner must still join the encoder in sending's
                    // finally. The process wrapper does not invent SDK errors.
                    if (route.owner.signal.aborted) return;
                    throw failure;
                }
                assert.equal(frame.timestamp, timestamp);
                encoded = frame.bytes;
            } else encoded = Buffer.from(entries[index], "base64");
            let frameCompleted = true;
            for (const packet of sender.packetize(encoded, timestamp)) {
                const remaining = Math.min(200, Math.floor(200 - (monotonicNow() - start - elapsed)));
                if (remaining <= 0) { route.lateSendFrames++; frameCompleted = false; break; }
                const sequence = new DataView(packet.buffer, packet.byteOffset).getUint32(4);
                const submittedAt = monotonicNow() - start;
                route.stages.send[medium] = "rtp";
                const result = await stream.sendRtp(new SdkRtpPacket(sequence, packet.subarray(8)), { timeoutMs: remaining, signal: route.owner.signal });
                assert.equal(result.boundary, "local-media-queue"); submitted = sender.submitted(packet);
                if (!video) {
                    assert.ok(route.submittedAudio.length < 3100);
                    const header = new DataView(packet.buffer, packet.byteOffset + 8);
                    route.submittedAudio.push({ source_offset_ms: elapsed, submit_started_ms: submittedAt,
                        submit_completed_ms: monotonicNow() - start, timestamp: header.getUint32(4),
                        sequence: header.getUint16(2), ssrc: header.getUint32(8) });
                }
                route.sent[medium]++;
            }
            // A partially submitted frame cannot be a dependency of the next
            // published frame. Request a fresh current frame without replay.
            if (video) forceKeyframe = !frameCompleted;
            if (frameCompleted && elapsed - lastClock >= 500) {
                route.stages.send[medium] = "clock";
                await stream.sendClock(sender.nextDatagramSequence(), { ...submitted, ntp: ntp(unixStart + elapsed), rtp: timestamp,
                    cname: "g6-native-av-" + route.index }, { timeoutMs: 1000, signal: route.owner.signal });
                lastClock = elapsed;
            }
        }
    }
    async function receiving(route, medium) {
        let lastVideoRead;
        const requestRecovery = async () => {
            if (route.feedbackSelected && lastVideoRead && route.videoHistory.takeRequest())
                await route.videoReader.requestKeyframe(lastVideoRead, { timeoutMs: 100 });
        };
        while (!route.owner.signal.aborted && monotonicNow() < end + 100) {
            let value, streamId;
            route.stages.receive[medium] = "packet";
            try {
                if (medium === "audio") value = await route.receiveAudio.receivePacket({ timeoutMs: 500, signal: route.owner.signal });
                else {
                    lastVideoRead = await route.videoReader.receivePacket({ timeoutMs: 100 });
                    ({ value, streamId } = lastVideoRead);
                }
            }
            catch (failure) {
                if (route.owner.signal.aborted && failure.kind === "cancelled") return;
                if (failure.kind === "deadline" && failure.certainty === "read-only") {
                    if (medium === "video") { route.videoHistory.expire(); await requestRecovery(); }
                    continue;
                }
                throw failure;
            }
            const elapsed = monotonicNow() - start;
            if (value.kind === "gap") {
                route.gaps += Number(value.droppedPackets);
                if (medium === "video") { route.videoHistory.gap(); await requestRecovery(); }
                continue;
            }
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
                const frame = route.videoHistory.push(streamId, value);
                await requestRecovery();
                if (frame) {
                    assert.ok(route.video.length < 2000);
                    route.capturedBytes += frame.bytes.byteLength;
                    assert.ok(route.capturedBytes <= 16 * 1024 * 1024, "bounded-native-media-recording-bytes");
                    route.video.push({ elapsed_ms: elapsed, timestamp: frame.timestamp, key: frame.key,
                        stream_id: streamId,
                        decoder_history: frame.decoderHistory,
                        ...(frame.videoEpoch ? { video_epoch: frame.videoEpoch } : {}),
                        ssrc: value.header.ssrc,
                        payload: Buffer.from(frame.bytes).toString("base64") });
                }
            }
        }
    }
    const launch = (task, route, medium, direction) => {
        const owned = task.catch(failure => {
            if (!error) {
                if (route.owner.signal.aborted && failure.kind === "cancelled") return;
                error = failure;
                workFailure = { route_index: route.index, medium, direction, stage: route.stages[direction][medium] ?? "pending" };
            }
            controller.abort(error);
        });
        packetTasks.push(owned); tasks.push(owned);
    };
    const reportControlFailure = (failure, context = {}) => {
        if (!error) {
            error = failure;
            workFailure = { route_index: null, medium: null, direction: "control", stage: "during-workload" };
        }
        // Only an exact certified private-target cleanup or settled original
        // read-only observer refusal can leave independent owners running.
        if (context.fatal || !isolatedProgrammeFailure(failure)) controller.abort(error);
    };
    try {
        for (const route of routes) for (const medium of ["audio", "video"]) {
            launch(route.owner.run(() => receiving(route, medium)), route, medium, "receive");
            launch(route.owner.run(() => sending(route, medium)), route, medium, "send");
        }
        for (const route of routes) if (route.videoFeedback)
            launch(route.owner.run(() => route.videoFeedback.run(end, monotonicNow)), route, "video", "receive");
        if (during) {
            controlTask = Promise.resolve().then(() => during(Object.freeze({
                signal: controller.signal, reportFailure: reportControlFailure,
                programStartUnixUs: unixStart * 1000, programClock, now: monotonicNow, startedAt: start,
                routes: Object.freeze(routes.map(route => Object.freeze({ index: route.index,
                    peerIndex: route.peerIndex, connection: route.item.connection,
                    sessionIndex: route.item.sid, role: route.item.role, generation: route.generation,
                    // Retire this Connection's packet and feedback tasks. A second
                    // Session may still belong to the same live SDK/helper.
                    stopAndJoin: () => route.owner.stopAndJoin(),
                    expectRetirement: operationId => route.owner.expectRetirement({ operationId,
                        connectionId: route.item.connection.id, generation: route.generation }),
                    switchIncomingVideo: (operationId, streamId, select) => route.videoReader.switchWith(operationId, async () => {
                        const operation = await select();
                        const reference = operation.referenceJson();
                        assert.equal(reference.operation_id, operationId);
                        assert.equal(reference.command, "stream.subscribe");
                        assert.equal(reference.scope.connection_id, route.item.connection.id);
                        const completed = await operation.wait({ timeoutMs: 8000 });
                        assert.equal(completed.snapshot.state, "completed");
                        const receipt = completed.value;
                        assert.equal(receipt.connection.id, route.item.connection.id);
                        assert.equal(receipt.connectionGeneration, route.generation);
                        assert.ok(Array.isArray(receipt.bindings), "camera switch requires paired capture");
                        const native = await route.item.connection.media.refresh({ timeoutMs: 8000 });
                        assert.equal(native.streamRevision, receipt.streamRevision);
                        const stream = native.streams.find(s => s.id === streamId && s.medium === "video" && s.direction === "recvonly");
                        assert.ok(stream);
                        assert.equal(Boolean(stream.videoFeedback), route.feedbackSelected);
                        const captured = receipt.bindings.find(b => b.parameters.id === streamId);
                        assert.ok(captured, "camera switch requires its exact captured binding");
                        assert.equal(captured.connectionGeneration, route.generation);
                        assert.deepEqual(captured.parameters, stream);
                        return { streamId, media: route.item.connection.streams.byId(streamId).media };
                    }),
                }))),
            }))).then(value => { controlResult = value; }, reportControlFailure);
            tasks.push(controlTask);
        }
        await joinMediaProgramme({ now: monotonicNow, startedAt: start, signal: controller.signal,
            abort: failure => controller.abort(failure), owners: routes.map(route => route.owner),
            packetTasks, controlTask, isolateFailure: isolatedProgrammeFailure,
            observe: result => { programmeWindowObserved = result.programme_window_observed; } });
    } catch (failure) {
        if (!error) {
            error = failure;
            workFailure = { route_index: null, medium: null, direction: "control", stage: "programme-join" };
        }
        controller.abort(error);
    } finally {
        controller.abort(error); await Promise.all(tasks);
        await Promise.all(routes.map(route => route.owner.stopAndJoin().catch(() => {})));
        await Promise.all(routes.map(async route => {
            route.encoderJoin = await route.videoEncoder.stopAndJoin();
        }));
        // Snapshot counters before clearing persisted captures. These small
        // values cannot retain a recording through the caller's report.
        routeResults = routes.map(route => ({ index: route.index, peer_index: route.peerIndex,
            received_audio: route.audio.length, received_video: route.video.length,
            submitted_packets: { ...route.sent }, received_clocks: { ...route.clocks },
            gap_packets: route.gaps, late_send_frames: route.lateSendFrames }));
        for (const route of routes) {
            await writeFile(join(outputDirectory, "received-" + route.index + ".json"), JSON.stringify({
            format: "connect/native-av-received/1", index: route.index, peer_index: route.peerIndex, audio: route.audio, video: route.video,
            connection_id: route.item.connection.id, generation: route.generation, session_index: route.item.sid,
            original_audio_stream_id: plan[route.item.prefix + "-audio"],
            initial_video_stream_id: plan[connections[route.peerIndex].prefix + "-video"],
            captured_codec_bytes: route.capturedBytes, received_clocks: route.clocks, gap_packets: route.gaps, late_send_frames: route.lateSendFrames, submitted_packets: route.sent,
            clock_reports: route.clockReports, submitted_audio: route.submittedAudio,
            video_reader: route.videoReader.observation, encoder_process: route.encoderJoin,
            video_feedback: route.videoFeedback ? { received: route.videoFeedback.received,
                consumed: route.videoFeedback.consumed, expired: route.videoFeedback.expired } : null,
            program_clock: programClock, unix_start_ms: unixStart, observed_seconds: (monotonicNow() - start) / 1000,
            }) + "\n", { flag: "wx", mode: 0o600 });
            // Successful persistence retains every original packet, submitted
            // timestamp and clock before its in-memory copy is released.
            releasePayloadArrays(route);
        }
        const observationReceipt = { persisted: false };
        const emitted = emitOriginalObservation(observe, [{ format: "connect/native-av-work-observation/1", original_tasks_joined: true,
            fixture_payload_arrays_released: routes.every(route => payloadArrays(route).every(array => array.length === 0)),
            encoder_processes_joined: routes.filter(route => route.encoderJoin?.joined).length,
            original_tasks_started: routes.reduce((n, r) => n + 4 + Number(Boolean(r.videoFeedback)), 0), control_task_started: Boolean(during), elapsed_ms: monotonicNow() - start, failure: workFailure ?? null,
            routes: routeResults.map(({ peer_index, ...counters }) => counters),
            scope: "Bounded original work counters after all original tasks join. No payloads, identities, media clocks, decoded quality or cell qualification." }, Object.freeze({
            passed: !error, original_receive_tasks_joined: true,
            programme_window_observed: programmeWindowObserved,
            observation_receipt: observationReceipt,
            movement_requested: Boolean(during), seconds: 62, warmup_seconds: 2,
            observation_seconds: 60, routes: routeResults,
            control_workload: controlResult ?? null,
            failure_observed: Boolean(error), cell_qualified: false,
        })], error);
        observationReceipt.persisted = emitted.emitted;
        Object.freeze(observationReceipt);
        if (!emitted.emitted) { error = emitted.error; controller.abort(error); throw error; }

    }
    if (error) throw error;
    return { connections: 6, participants: 4, sessions: 2, source_manifest_sha256: sha(await readFile(join(sourceDirectory, "manifest.json"))),
        original_receive_tasks_joined: true, seconds: 62, warmup_seconds: 2, observation_seconds: 60,
        routes: routeResults,
        ...(during ? { control_workload: controlResult } : {}),
        scope: "Real public SDK native RTP and source-clock transport only. Independent decoder and profile checks are separate; no full-cell qualification." };
}
