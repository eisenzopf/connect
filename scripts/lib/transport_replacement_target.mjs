import { validateProgrammeClock, programmeElapsed } from "./transport_programme_clock.mjs";
// Public SDK metadata controller and an independently owned physical Rust peer.
// Packet sources/receivers and independent decoding remain the caller's owners.
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { spawn } from "node:child_process";
import { link, mkdir, open, readFile, realpath, stat, unlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { liveVideoProgram } from "./transport_live_video_program.mjs";
import { operationFailure } from "./transport_failure.mjs";
import { inspectReplacementCapture, verifyInspectedDataReceipt } from "./transport_capture_inspection.mjs";
import { validatePhysicalData, verifyPhysicalDataReceipt, readPhysicalDataReceipt } from "./transport_movement_data.mjs";

const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const movementFacts = value => ({ connection: value.connection.id,
    retained: value.retained.map(v => v.id), admitted: value.admitted.map(v => v.id), retired: value.retired,
    topologyRevision: value.topologyRevision, conversationRevision: value.conversationRevision ?? null, binding: value.binding });
async function save(path, value) {
    const file = await open(path, "wx", 0o600);
    try { await file.writeFile(JSON.stringify(value, null, 2) + "\n"); await file.sync(); }
    finally { await file.close(); }
    const directory = await open(dirname(path), "r");
    try { await directory.sync(); } finally { await directory.close(); }
}
// Publish a complete, exclusive stop marker while the child is reading.
async function saveAtomic(path, value) {
    const temporary = path + "." + randomUUID() + ".tmp";
    await save(temporary, value);
    try { await link(temporary, path); }
    finally { await unlink(temporary); }
    const directory = await open(dirname(path), "r");
    try { await directory.sync(); } finally { await directory.close(); }
}
async function bounded(promise, milliseconds) {
    let timer;
    try { return await Promise.race([promise, new Promise((_, reject) => {
        timer = setTimeout(() => reject(Error("original-target-join-deadline")), milliseconds);
    })]); } finally { clearTimeout(timer); }
}

export function replacementInput({ operationId, conversationId, sessionId, connectionId,
    retainedConnectionId, expectedTopologyRevision, originalGeneration, retainedGeneration, target, streamId, dataStreamId, source, output }) {
    assert.ok([operationId, conversationId, sessionId, connectionId, retainedConnectionId,
        target?.endpointId, target?.deviceId, streamId].every(id));
    assert.notEqual(connectionId, retainedConnectionId);
    assert.ok(Number.isSafeInteger(expectedTopologyRevision) && expectedTopologyRevision > 0);
    assert.ok([originalGeneration, retainedGeneration].every(value => Number.isSafeInteger(value) && value > 0));
    assert.ok(Number.isInteger(source?.seconds) && source.seconds >= 1 && source.seconds <= (source.video ? 62 : 30));
    assert.equal(source.programStartUnixUs !== undefined, source.programClock !== undefined);
    if (source.programStartUnixUs !== undefined) {
        assert.ok(Number.isSafeInteger(source.programStartUnixUs) && source.programStartUnixUs > 0);
        validateProgrammeClock(source.programClock);
    }
    assert.ok(Number.isInteger(source.ssrc) && source.ssrc > 0 && source.ssrc <= 0xffffffff);
    assert.ok(Array.isArray(source.audioPackets) && source.audioPackets.length === source.seconds * 50);
    if (dataStreamId !== undefined) {
        assert.ok(id(dataStreamId));
        assert.ok(![streamId, source.video?.streamId, source.video?.receiveAudioStreamId,
            source.video?.receiveVideoStreamId, source.video?.receiveDataStreamId].includes(dataStreamId));
    }
    if (source.terminalEndOperationId !== undefined) {
        assert.ok(source.video && source.seconds === 62 && source.programStartUnixUs !== undefined);
        assert.ok(id(source.terminalEndOperationId));
        assert.notEqual(source.terminalEndOperationId, operationId);
    }
    if (source.data !== undefined) {
        validatePhysicalData(source.data);
        assert.ok(source.video && source.seconds === 62 && source.programStartUnixUs !== undefined);
        assert.equal(source.data.outgoing.stream_id, dataStreamId);
        assert.equal(source.data.incoming.stream_id, source.video.receiveDataStreamId);
        for (const original of Object.values(source.data)) {
            assert.notEqual(original.operation_id, operationId);
            assert.notEqual(original.operation_id, source.terminalEndOperationId);
        }
    }
    const audio = source.audioPackets.map(bytes => {
        assert.ok(Array.isArray(bytes) && bytes.length >= 1 && bytes.length <= 1168 &&
            bytes.every(value => Number.isInteger(value) && value >= 0 && value <= 255));
        return [...bytes];
    });
    let video;
    if (source.video) {
        const v = source.video;
        assert.ok([v.streamId, v.receiveAudioStreamId, v.receiveVideoStreamId, v.previousStreamId, v.previousReceiveVideoStreamId].every(id));
        assert.equal(new Set([streamId, v.streamId, v.receiveAudioStreamId, v.receiveVideoStreamId]).size, 4);
        if (v.receiveDataStreamId !== undefined) {
            assert.ok(id(dataStreamId) && id(v.receiveDataStreamId));
            assert.ok(![streamId, v.streamId, v.receiveAudioStreamId, v.receiveVideoStreamId,
                dataStreamId].includes(v.receiveDataStreamId));
        }
        assert.ok(Number.isInteger(v.ssrc) && v.ssrc > 0 && v.ssrc <= 0xffffffff && v.ssrc !== source.ssrc);
        assert.equal(v.frames, undefined, "replacement requires the live original source");
        const program = liveVideoProgram(v.program, source.seconds * 30);
        video = { stream_id: v.streamId, receive_audio_stream_id: v.receiveAudioStreamId,
            receive_video_stream_id: v.receiveVideoStreamId,
            ...(v.receiveDataStreamId === undefined ? {} : { receive_data_stream_id: v.receiveDataStreamId }),
            ssrc: v.ssrc, program };
    }
    const command = { operation_id: operationId, conversation_id: conversationId,
        session_id: sessionId, connection_id: connectionId, expected_revision: expectedTopologyRevision,
        intent: { type: "connection.replace", endpoint_id: target.endpointId,
            candidate_connid: null, retained_connid: retainedConnectionId,
            ...(video ? { video: { publications: [{ previous: source.video.previousStreamId, replacement: video.stream_id }],
                receivers: [{ previous: source.video.previousReceiveVideoStreamId, replacement: video.receive_video_stream_id }] } } : {}) }, payload_extensions: {} };
    return { mode: "replacement_peer", config_path: target.nativeConfig, command,
        offer: { substrate: "quic", capabilities: {
            audio_codecs: [{ name: "opus", params: { sample_rate: 48000, channels: 1 } }],
            video_codecs: video ? [{ name: "vp8", params: { sample_rate: 90000, channels: 0 } }] : [],
            data_protocols: dataStreamId === undefined ? [] : ["binary"],
            max_streams_per_connection: video ? 8 : dataStreamId === undefined ? 1 : 2,
            identity_assurance_offered: "identified", source_clock: "rtcp-sr-sdes/1",
            ...(video ? { video_feedback: "rtcp-pli-epoch/1" } : {}),
        }, streams_offered: [{ id: streamId, kind: "audio", direction: "sendrecv", codec_preferences: ["opus"] },
            ...(video ? [{ id: video.stream_id, kind: "video", direction: "sendonly", codec_preferences: ["vp8"] },
                { id: video.receive_video_stream_id, kind: "video", direction: "recvonly", codec_preferences: ["vp8"] }] : []),
            ...(dataStreamId === undefined ? [] : [{ id: dataStreamId, kind: "data", direction: "sendrecv", codec_preferences: ["binary"] }])] },
        media: { seconds: source.seconds, ssrc: source.ssrc, audio_packets: audio, ...(video ? { video } : {}),
            ...(source.data === undefined ? {} : { data: structuredClone(source.data) }),
            ...(source.programStartUnixUs !== undefined ? { program_start_unix_us: source.programStartUnixUs, program_clock: structuredClone(source.programClock) } : {}),
            ...(source.terminalEndOperationId !== undefined ? { terminal_end_operation: source.terminalEndOperationId } : {}) }, output };
}

// Both admission and joined captures retain every original negotiated resource.
// Data is a reliable stream identity, never an RTP payload handle.
export function verifyReplacementStreamIdentities(streams, intent) {
    assert.ok(Array.isArray(streams) && streams.every(stream => id(stream?.strm_id)));
    const expected = [intent.streamId,
        ...(intent.dataStreamId === undefined ? [] : [intent.dataStreamId]),
        ...(intent.source.video ? [intent.source.video.streamId, intent.source.video.receiveVideoStreamId,
            ...(intent.source.video.receiveDataStreamId === undefined ? [] : [intent.source.video.receiveDataStreamId])] : [])];
    assert.ok(expected.every(id) && new Set(expected).size === expected.length);
    assert.deepEqual(streams.map(stream => stream.strm_id).sort(), expected.sort());
}

// This binds a trusted peer's verified-response observation to the actual
// admitted connection. Independent stopped-journal audit still proves effects.
export function verifyPhysicalOwnerEndReceipt(receipt, intent, admittedConnectionId) {
    assert.ok(id(admittedConnectionId) && id(intent.source.terminalEndOperationId));
    assert.equal(receipt?.format, "connect/physical-owner-end-receipt/1");
    assert.equal(receipt.completed, true);
    assert.equal(receipt.original_operation_id_preserved, true);
    assert.equal(receipt.mutation_submissions, 1);
    assert.equal(receipt.control_only_after_programme, true);
    assert.equal(receipt.media_programme_extended, false);
    assert.ok(Number.isFinite(receipt.elapsed_since_programme_end_ms) &&
        receipt.elapsed_since_programme_end_ms >= 0 && receipt.elapsed_since_programme_end_ms < 5000);
    assert.deepEqual(receipt.original, {
        format: "connect/physical-owner-original-end/1", type: "connection.end",
        scope: { conversation_id: intent.conversationId, session_id: intent.sessionId,
            connection_id: admittedConnectionId, operation_id: intent.source.terminalEndOperationId },
        payload: { reason_code: 200, reason: "Synthetic workload complete" },
        programme_end_monotonic_ms: 62000, original_cutoff_monotonic_ms: 67000,
    });
}

// Preparing includes reservation and in-flight Core preparation. Only the
// exact durable prepare receipt permits launching the independent target.
// This is an observation, never authority: allocation revalidates everything.
export function replacementPreparationRecorded(root) {
    return root.state === "preparing" && root.native_replacement_prepared === true;
}

export const physicalReplacementTarget = options => physicalTarget(options, false);
// Admission returns a live physical owner. Its report is final only after join.
export const startPhysicalReplacementTarget = options => physicalTarget(options, true);

async function physicalTarget({ controller, retireOriginal, privateRoot,
    directory, executable, executableSha256, ...intent }, live) {
    assert.equal(typeof retireOriginal, "function");
    assert.ok(live || intent.source.terminalEndOperationId === undefined, "final end requires live ownership transfer");
    assert.ok(live || intent.source.data === undefined, "Data exchange requires live ownership transfer");
    const root = await realpath(privateRoot), parent = await realpath(dirname(resolve(directory)));
    assert.ok(parent === root || parent.startsWith(root + "/"));
    assert.equal(await realpath(executable), resolve(executable));
    assert.equal(hash(await readFile(executable)), executableSha256);
    const config = await realpath(intent.target.nativeConfig);
    assert.ok(config.startsWith(root + "/"));
    assert.equal((await stat(config)).mode & 0o077, 0);
    const output = join(resolve(directory), "target-observation.json");
    const input = replacementInput({ ...intent, target: { ...intent.target, nativeConfig: config }, output });
    if (intent.source.programStartUnixUs !== undefined) {
        programmeElapsed(intent.source.programClock, intent.source.seconds);
    }
    if (live) input.media.owner_control = true;
    const configHash = hash(await readFile(config));
    await mkdir(directory, { mode: 0o700 });
    // The complete original command is durable before the mutation is attempted.
    await save(join(directory, "original-command.json"), input.command);
    await save(join(directory, "original-generations.json"), { source: intent.originalGeneration, retained: intent.retainedGeneration });
    const report = { format: "connect/physical-native-replacement-stage/1", passed: false,
        target_executable_sha256: executableSha256, original_command_sha256: hash(await readFile(join(directory, "original-command.json"))),
        original_reference_preserved: false, original_root_completed: false,
        original_source_owner_joined: false, target_worker_joined: false, target_worker_forced: false,
        target_config_unchanged: false, target_executable_unchanged: false,
        source_input_sha256: hash(Buffer.from(JSON.stringify(input.media))), video_enabled: Boolean(intent.source.video),
        decoded_media_verified: false, cell_qualified: false, task_complete: false, whole_gate_qualified: false };
    let phase = "original-submission", worker, completed, log, exited, admittedConnectionId;
    let transferred = false, finishPromise, stopPromise;
    const stopPath = output.replace(/\.json$/, ".owner-stop.json");
    const readyPath = output.replace(/\.json$/, ".owner-ready.json");
    const recordFailure = error => {
        report.passed = false;
        report.failure ??= { phase, name: error.name, kind: error.kind ?? null,
            certainty: error.certainty ?? null, ...operationFailure(error) };
        const at = error.stack?.match(/transport_replacement_target\.mjs:(\d+):(\d+)/);
        if (at) report.failure.source ??= { line: Number(at[1]), column: Number(at[2]) };
    };
    const requestStop = () => stopPromise ??= (async () => {
        if (worker && !exited && live) {
            await saveAtomic(stopPath, { command: "stop", operation_id: intent.operationId });
            report.target_stop_requested = true;
        }
    })();
    const cleanup = async () => {
        if (worker && !report.target_worker_joined) {
            try {
                if (live) await requestStop();
                await bounded(completed, (live ? 10 : intent.source.seconds + 10) * 1000);
                report.target_worker_joined = true;
            } catch {
                report.target_worker_forced = true; worker.kill("SIGTERM");
                try { await bounded(completed, 5000); report.target_worker_joined = true; }
                catch { worker.kill("SIGKILL"); await completed; report.target_worker_joined = true; }
            }
        }
        await log?.close();
        report.target_executable_unchanged = await readFile(executable).then(bytes => hash(bytes) === executableSha256, () => false);
        report.target_config_unchanged = await readFile(config).then(bytes => hash(bytes) === configHash, () => false);
        report.passed &&= report.target_worker_joined && !report.target_worker_forced &&
            report.target_executable_unchanged && report.target_config_unchanged;
        await save(join(directory, "stage.json"), report);
    };
    const finish = () => finishPromise ??= (async () => {
        try {
            phase = "target-worker-join";
            const exit = await bounded(completed, (intent.source.seconds + 10) * 1000);
            report.target_worker_joined = true;
            assert.equal(exit.code, 0); assert.equal(exit.signal, null); assert.equal(exit.error, undefined);
            const info = await stat(output); assert.ok(info.size > 0 && info.size <= (intent.source.video ? 80 : 24) * 1024 * 1024);
            assert.equal(info.mode & 0o077, 0);
            const inspected = await inspectReplacementCapture(output,
                (intent.source.video ? 80 : 24) * 1024 * 1024, intent.source.video ? 16000 : 4000);
            const observed = inspected.observation;
            assert.equal(observed.format, "connect/native-replacement-peer-observation/1");
            assert.equal(observed.passed, true); assert.equal(observed.original_client_joined, true);
            assert.equal(observed.operation_completed_inferred, false); assert.equal(observed.decoded_media_verified, false);
            assert.ok(Number.isInteger(observed.sent_audio_packets) && observed.sent_audio_packets > 0 &&
                observed.sent_audio_packets <= intent.source.seconds * 50);
            assert.equal(observed.video_enabled, Boolean(intent.source.video));
            if (intent.source.programStartUnixUs !== undefined) {
                assert.equal(observed.program_start_unix_us, intent.source.programStartUnixUs);
                assert.deepEqual(observed.program_clock, intent.source.programClock);
                assert.ok(Number.isInteger(observed.source_start_audio_index) && observed.source_start_audio_index >= 0 &&
                    observed.source_start_audio_index < intent.source.seconds * 50);
                assert.ok(Number.isInteger(observed.source_start_video_index) && observed.source_start_video_index >= 0 &&
                    observed.source_start_video_index < intent.source.seconds * 30);
                assert.ok(observed.sent_audio_packets <= intent.source.seconds * 50 - observed.source_start_audio_index);
                report.original_program_start_unix_us = observed.program_start_unix_us;
                report.original_program_clock = observed.program_clock;
                report.target_start_audio_index = observed.source_start_audio_index;
                report.target_start_video_index = observed.source_start_video_index;
            }
            assert.ok(Array.isArray(observed.received) && observed.received.length <= (intent.source.video ? 16000 : 4000));
            if (intent.source.video) {
                assert.equal(observed.original_root_completed_observed, true);
                assert.equal(observed.video_transfer_admitted, true);
                assert.equal(observed.encoder_process?.joined, true);
                assert.ok(observed.encoder_process.clean || observed.encoder_process.cancelled);
                const packetLimit = intent.source.video.program.frame_count * Math.ceil(256 * 1024 / 1167);
                assert.ok(Number.isInteger(observed.sent_video_packets) && observed.sent_video_packets > 0 && observed.sent_video_packets <= packetLimit);
                verifyReplacementStreamIdentities(observed.negotiated_streams, intent);
                report.video_control_files = {};
                for (const suffix of ["completed-root", ...(intent.dataStreamId === undefined ? [] : ["data-unmute-intent", "data-unmute-receipt"]),
                    ...(intent.source.video.receiveDataStreamId === undefined ? [] : ["data-subscribe-intent", "data-subscribe-receipt"])]) {
                    const path = output.replace(/\.json$/, "." + suffix + ".json");
                    const info = await stat(path); assert.equal(info.mode & 0o077, 0); assert.ok(info.size > 0 && info.size <= 65536);
                    report.video_control_files[suffix] = hash(await readFile(path));
                }
                report.target_sent_video_packets = observed.sent_video_packets;
            }
            if (intent.source.terminalEndOperationId !== undefined) {
                assert.equal(observed.terminal_end_requested, true);
                verifyPhysicalOwnerEndReceipt(observed.terminal_end_receipt, intent, admittedConnectionId);
                report.final_current_owner_end = {
                    operation_id: intent.source.terminalEndOperationId, connection_id: admittedConnectionId,
                    elapsed_since_programme_end_ms: observed.terminal_end_receipt.elapsed_since_programme_end_ms,
                    original_cutoff_ms: 67000, mutation_submissions: 1,
                };
                report.final_owner_end_files = {};
                for (const suffix of ["owner-end-intent", "owner-end-receipt"]) {
                    const path = output.replace(/\.json$/, "." + suffix + ".json");
                    const info = await stat(path); assert.equal(info.mode & 0o077, 0);
                    assert.ok(info.size > 0 && info.size <= 65536);
                    report.final_owner_end_files[suffix] = hash(await readFile(path));
                }
            }
            if (intent.source.data !== undefined) {
                const receipt = await readPhysicalDataReceipt(output.replace(/\.json$/, ".data-receipt.json"));
                verifyInspectedDataReceipt(observed, receipt);
                verifyPhysicalDataReceipt(receipt, { contract: intent.source.data,
                    conversationId: intent.conversationId, sessionId: intent.sessionId,
                    connectionId: admittedConnectionId, rootOperationId: intent.operationId });
                report.physical_data_exchange_verified = true;
                report.physical_data_files = {};
                for (const suffix of ["data-original-send", "data-start", "data-wire", "data-receipt", "data-upstream-original", "data-exchange"]) {
                    const path = ["data-upstream-original", "data-exchange"].includes(suffix) ? join(directory, suffix + ".json") : output.replace(/\.json$/, "." + suffix + ".json");
                    const info = await stat(path); assert.equal(info.mode & 0o077, 0);
                    assert.ok(info.size > 0 && info.size <= 2 * 1024 * 1024);
                    report.physical_data_files[suffix] = hash(await readFile(path));
                }
            }
            report.target_observation_sha256 = inspected.sha256;
            report.capture_inspection_worker_joined = inspected.worker_joined;
            report.target_sent_packets = observed.sent_audio_packets;
            report.target_skipped_frames = observed.skipped_original_frames;
            report.target_received_datagrams = observed.received.length;
            if (live) {
                assert.equal(observed.owner_control, true);
                assert.equal(observed.owner_ready, true);
                if (report.target_stop_requested) assert.equal(observed.stop_requested, true);
            }
            report.passed = !report.failure;
        } catch (error) { recordFailure(error); throw error; }
        finally { await cleanup(); }
        assert.equal(report.passed, true);
        return structuredClone(report);
    })();
    try {
        const operationUntil = performance.now() + 8000;
        const remaining = () => { const value = Math.floor(operationUntil - performance.now()); assert.ok(value > 0); return value; };
        const connection = controller.conversations.byId(intent.conversationId).sessions.byId(intent.sessionId)
            .connections.byId(intent.connectionId);
        const operation = await connection.replaceDevice({ retain: intent.retainedConnectionId,
            target: { endpoint: intent.target.endpointId }, expectedTopologyRevision: intent.expectedTopologyRevision,
            ...(input.command.intent.video ? { video: input.command.intent.video } : {}) },
        { timeoutMs: remaining(), operationId: intent.operationId });
        const reference = operation.referenceJson();
        assert.equal(reference.command, "connection.replace"); assert.equal(reference.operation_id, intent.operationId);
        assert.deepEqual(Object.keys(reference.scope).sort(), ["connection_id", "conversation_id", "session_id"]);
        assert.equal(reference.scope.conversation_id, intent.conversationId);
        assert.equal(reference.scope.session_id, intent.sessionId);
        assert.equal(reference.scope.connection_id, intent.connectionId);
        await save(join(directory, "original-reference.json"), reference);
        phase = "original-allocation-root";
        let root, recordId, revision = -1, savedAccepted = false;
        for (;;) {
            const frames = await controller.raw.request(controller.raw.validate({ v: 2, id: "target_" + randomUUID(),
                ts: new Date().toISOString(), type: "operation.get", cid: intent.conversationId, sid: intent.sessionId,
                connid: intent.connectionId, payload: { operation_id: intent.operationId } }), { timeoutMs: remaining() });
            const updates = frames.filter(frame => frame.value.type === "operation.updated");
            assert.equal(updates.length, 1);
            root = updates[0].value.payload;
            assert.equal(root.operation_id, intent.operationId); assert.ok(id(root.record_id));
            recordId ??= root.record_id; assert.equal(root.record_id, recordId);
            assert.equal(root.command_type, "connection.replace");
            assert.equal(root.expected_revision, intent.expectedTopologyRevision);
            assert.ok(Number.isSafeInteger(root.revision) && root.revision >= 0 && root.revision >= revision);
            revision = root.revision;
            assert.ok(["accepted", "preparing"].includes(root.state));
            if (replacementPreparationRecorded(root)) break;
            if (!savedAccepted) { await save(join(directory, "accepted-root.json"), root); savedAccepted = true; }
            // Read only the same root under the original cutoff. Neither
            // acceptance nor a preparation reservation grants a captured ticket.
            await new Promise(done => setTimeout(done, Math.min(20, remaining())));
        }
        input.root = root;
        await save(join(directory, "target-input.json"), input);
        phase = "independent-physical-target";
        log = await open(join(directory, "target.log"), "wx", 0o600);
        worker = spawn(executable, [join(resolve(directory), "target-input.json")], { stdio: ["ignore", log.fd, log.fd] });
        completed = new Promise(done => {
            worker.once("error", error => done({ error: error.code ?? error.name }));
            worker.once("close", (code, signal) => done({ code, signal }));
        }).then(exit => { exited = exit; return exit; });
        phase = "original-completion";
        const waiting = new AbortController();
        let targetFailed = false;
        let observingCompletion = true;
        let done;
        try {
            done = await Promise.race([
                operation.wait({ timeoutMs: remaining(), signal: waiting.signal }),
                completed.then(exit => {
                    // A failed physical target cannot finish this original
                    // operation. Preserve that first cause, cancel only the
                    // read-only wait, and join the original worker in cleanup.
                    if (observingCompletion && (live || exit.code !== 0 || exit.signal !== null || exit.error !== undefined)) {
                        targetFailed = true;
                        phase = "independent-physical-target";
                        report.target_early_exit = { code: exit.code ?? null, signal: exit.signal ?? null,
                            error: exit.error ?? null };
                        throw Error("original-physical-target-exited-before-completion");
                    }
                    return new Promise(() => {});
                }),
            ]);
        } finally { observingCompletion = false; if (targetFailed) waiting.abort(); }
        assert.equal(done.snapshot.state, "completed");
        const moved = done.value;
        admittedConnectionId = moved.connection.id;
        assert.deepEqual(moved.retired, [intent.connectionId]);
        assert.deepEqual(moved.retained.map(v => v.id), [intent.retainedConnectionId]);
        report.original_root_completed = true;
        phase = "retained-current-observation";
        const retained = await moved.retained[0].refresh({ timeoutMs: 5000 });
        assert.equal(retained.id, intent.retainedConnectionId); assert.equal(retained.generation, intent.retainedGeneration);
        assert.equal(moved.binding.deviceId, intent.target.deviceId);
        assert.ok(id(moved.connection.id)); assert.notEqual(moved.connection.id, intent.connectionId);
        phase = "original-retirement-observation";
        const until = performance.now() + 5000;
        let retiredOriginal;
        for (;;) {
            const remaining = Math.floor(until - performance.now()); assert.ok(remaining > 0);
            const original = await connection.refresh({ timeoutMs: remaining });
            assert.equal(original.id, intent.connectionId);
            if (original.state === "retired") { assert.ok(original.generation > intent.originalGeneration); retiredOriginal = original; break; }
            await new Promise(done => setTimeout(done, Math.min(50, remaining)));
        }
        phase = "original-reference-recovery";
        const restored = controller.operations.restore(reference, controller.operationKinds.connection_replace);
        assert.deepEqual(restored.referenceJson(), reference);
        const recovered = await restored.wait({ timeoutMs: 8000 });
        assert.equal(recovered.snapshot.state, "completed");
        // SDK handles carry methods, not captured snapshot facts. Compare the
        // original result's identities/revisions/binding, then read current facts.
        assert.deepEqual(movementFacts(recovered.value), movementFacts(moved));
        report.original_reference_preserved = true;
        phase = "original-source-owner-join";
        await retireOriginal({ reference: structuredClone(reference), original: retiredOriginal,
            movement: structuredClone(movementFacts(moved)) });
        report.original_source_owner_joined = true;
        if (live) {
            phase = "target-live-owner-observation";
            const readyUntil = performance.now() + 8000;
            let ready;
            for (;;) {
                assert.equal(exited, undefined, "physical target exited before ownership transfer");
                try {
                    const info = await stat(readyPath);
                    assert.equal(info.mode & 0o077, 0); assert.ok(info.size > 0 && info.size <= 65536);
                    ready = JSON.parse(await readFile(readyPath, "utf8"));
                    break;
                } catch (error) { if (error.code !== "ENOENT") throw error; }
                assert.ok(performance.now() < readyUntil, "physical owner readiness deadline");
                await new Promise(done => setTimeout(done, 20));
            }
            assert.equal(ready.format, "connect/native-replacement-owner-ready/1");
            assert.equal(ready.operation_id, intent.operationId);
            assert.equal(ready.connection_id, moved.connection.id);
            assert.equal(ready.video_enabled, Boolean(intent.source.video));
            assert.equal(ready.operation_completed_inferred, false);
            assert.equal(ready.decoded_media_verified, false);
            verifyReplacementStreamIdentities(ready.negotiated_streams, intent);
            report.target_owner_ready_sha256 = hash(await readFile(readyPath));
            await save(join(directory, "admission.json"), { ...report, movement: movementFacts(moved) });
            assert.equal(exited, undefined, "physical target exited during ownership transfer");
            transferred = true;
            return Object.freeze({ connection: moved.connection,
                movement: structuredClone(movementFacts(moved)), reference: structuredClone(reference),
                // These methods never submit a service mutation. A subsequent
                // move owns the original operation and this owner's retirement.
                join: finish,
                stopAndJoin: async () => { await requestStop(); return finish(); },
            });
        }
        return await finish();
    } catch (error) { recordFailure(error); throw error; }
    finally { if (!transferred && !finishPromise) await cleanup(); }
}
