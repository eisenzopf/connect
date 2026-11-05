import { nativeMatrixLifecycle } from "./lib/native_matrix_lifecycle.mjs";
import { waitForProgrammeOffset } from "./lib/transport_programme_clock.mjs";
import { liveVideoProgramFromManifest } from "./lib/transport_live_video_program.mjs";
// Actual public Node SDK/native helper scenario. No Store or provider IO.
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { readFile, writeFile, mkdir, stat, realpath, open } from "node:fs/promises";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { NodeClient, SdkBinaryData } from "../sdk/typescript/dist/index.js";
import { nativeHost, executableHash, childPids, processExists, observeLocalTopology, observeRemoteTopology, observeMacLinuxTopology } from "./lib/transport_host.mjs";
import { connectionSnapshots } from "./lib/transport_snapshot.mjs";
import { nativeAv } from "./lib/transport_av.mjs";
import { operationFailure } from "./lib/transport_failure.mjs";
import { liveSessionUpgrade } from "./lib/transport_live_upgrade.mjs";
import { deviceMovementWorkload } from "./lib/transport_movement_workload.mjs";
import { observeMovementSessions } from "./lib/transport_movement_sessions.mjs";
import { physicalMovementSegments } from "./lib/transport_movement_observation.mjs";
import { TOTAL_MOVES } from "./lib/transport_movement_profile.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const inputPath = resolve(process.argv[2] ?? ""), output = resolve(process.argv[3] ?? "");
const mediaWorkload = process.argv[4] === "--media-workload";
const dataWorkload = mediaWorkload || process.argv[4] === "--data-workload";
assert.ok(process.argv.length === 4 || (process.argv.length === 5 && dataWorkload));
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const encode = value => JSON.stringify(value, null, 2) + "\n";
const privateRoot = await realpath(join(root, ".local"));
assert.ok((await realpath(inputPath)).startsWith(privateRoot + "/") && output.startsWith(privateRoot + "/"));
assert.equal((await stat(inputPath)).mode & 0o077, 0);
const input = JSON.parse(await readFile(inputPath, "utf8"));
const matrixLifecycle = mediaWorkload && input.matrix_resource_directory
    ? nativeMatrixLifecycle(input.matrix_resource_directory) : undefined;
assert.equal(input.format, "connect/transport-conversation-input/1");
assert.ok(["EdDSA", "ES256"].includes(input.signing_algorithm));
assert.equal(input.client_platform ?? input.server_platform, nativeHost().platform);
assert.ok(input.server_platform===nativeHost().platform || input.server_platform==="macos-arm64" && input.network_profile==="remote" && input.server_ownership?.kind==="macos-linux-guest");
assert.ok(["local", "remote"].includes(input.network_profile));
const mediaRoles = ["participant-1", "participant-2", "participant-3", "participant-4", "replacement-1"];
const controllerRoles = ["controller-1", "controller-2"];
assert.deepEqual(Object.keys(input.actors).sort(), [...mediaRoles, ...(mediaWorkload ? controllerRoles : [])].sort());
assert.equal(hash(await readFile(input.executable)), input.binary_sha256);
if (mediaWorkload) {
    assert.ok((await realpath(input.physical_peer_executable)).startsWith(privateRoot + "/"));
    assert.equal(hash(await readFile(input.physical_peer_executable)), input.physical_peer_sha256);
    assert.equal(new Set(Object.values(input.actors).map(actor => actor.device_id)).size, 7);
    assert.ok([...controllerRoles, "replacement-1"].every(role => input.actors[role].identity_id === input.actors["participant-1"].identity_id));
}
const privateDirectory = join(dirname(inputPath), "conversation-control");
await mkdir(privateDirectory, { mode: 0o700 });
const plan = Object.fromEntries([
    "create", "before", "included", "from4", "after", "add", "update", "stale", "remove", "readd",
    "session0", "session1", "invite0", "invite1", "accept0", "accept1", "upgrade", "unsupported", "end0", "end1",
].map(name => [name, "transport_" + randomUUID()]));
for (const name of ["before", "included", "from4", "after"]) plan[name + "Message"] = "message_" + randomUUID();
const sessionRoles = dataWorkload ? [["participant-1", "participant-2", "participant-3", "participant-4"], ["participant-1", "participant-2"]] : [["participant-1", "participant-2"], ["participant-1", "participant-3"]];
if (dataWorkload) for (const [sid, roles] of sessionRoles.entries()) for (const role of roles) {
    const prefix = sid + "-" + role;
    for (const field of ["connection", "stream", "audio", "video", "offer", "select", "data", "message", "end", "accept"]) plan[prefix + "-" + field] = "workload_" + randomUUID();
}
if (mediaWorkload) for (const sid of [0, 1]) for (const role of ["participant-1", "participant-2"]) {
    const prefix = "live-" + sid + "-" + role;
    for (const field of ["connection", "stream", "offer", "select", "before", "after", "before-message", "after-message", "end"])
        plan[prefix + "-" + field] = "workload_" + randomUUID();
}
const planFile = await open(join(privateDirectory, "plan.json"), "wx", 0o600);
try { await planFile.writeFile(encode(plan)); await planFile.sync(); } finally { await planFile.close(); }
const directory = await open(privateDirectory, "r"); try { await directory.sync(); } finally { await directory.close(); }
const options = { timeoutMs: 8000 };
const mutation = name => ({ ...options, operationId: plan[name] });
const clients = new Map();
const observedHelpers = new Set();
const dataFailures = new WeakMap();
const ownerNativeMessageIds = [];
const report = { format: mediaWorkload ? "connect/transport-conversation-control/2" : "connect/transport-conversation-control/1", passed: false, checks: [],
    source_revision: input.source_revision, binary_sha256: input.binary_sha256, server_binary_sha256: input.server_binary_sha256 ?? input.binary_sha256, signing_algorithm: input.signing_algorithm, history_page_counts: [],
    physical_peer_sha256: mediaWorkload ? input.physical_peer_sha256 : null,
    node: process.version, host: nativeHost(), network_profile: input.network_profile, plan_sha256: hash(await readFile(join(privateDirectory, "plan.json"))),
    input_sha256: hash(await readFile(inputPath)), sdk_config_hashes: {},
    maximum_active_members_observed: 0, concurrent_session_intents_observed: 0,
    actual_connection_workload_observed: false, live_media_upgrade_observed: false,
    replacement_media_ownership_observed: false, actual_topology_verified: false,
    cell_qualified: false, task_complete: false, whole_gate_qualified: false,
    workload_mode: mediaWorkload ? "six-native-opus-vp8-data-connections" : dataWorkload ? "six-native-data-connections" : "control-only",
    scope: "One source-bound native OS/signing/network fixture public Node SDK membership/audience/Session control scenario. Optional workload opens six actual native data Connections across four participants/two Sessions and verifies 64KiB reliable payloads. Optional media workload adds six native Opus/VP8 sources and RTP/source-clock reception; independent decoding is checked by the parent. Paired AV-sync, live upgrade/replacement ownership, browser, complete remote transport cells, SIP, TURN, model/provider and complete matrix cells remain unqualified." };
let phase = "client-setup";
function check(name) { report.checks.push(name); phase = name; }
async function complete(operation) {
    const result = await (await operation).wait(options);
    assert.equal(result.snapshot.state, "completed");
    return result.value;
}
async function refused(call, code, reason) {
    let observed = false;
    try { await call(); } catch (error) {
        assert.equal(error.problem?.code, code);
        assert.equal(error.problem?.reason, reason);
        assert.notEqual(error.certainty, "uncertain");
        observed = true;
    }
    assert.equal(observed, true);
}
async function helpers() {
    const live = new Set();
    for (const pid of childPids(process.pid)) {
        try { if (await executableHash(pid) === input.binary_sha256) { observedHelpers.add(pid); live.add(pid); } }
        catch (error) { if (!["ENOENT", "ESRCH"].includes(error.code)) throw error; }
    }
    return live;
}
try {
    if (matrixLifecycle) report.resource_baseline = await matrixLifecycle.baseline();
    for (const [role, actor] of Object.entries(input.actors)) {
        assert.ok((await realpath(actor.config)).startsWith(privateRoot + "/"));
        assert.equal((await stat(actor.config)).mode & 0o077, 0);
        report.sdk_config_hashes[role] = hash(await readFile(actor.config));
        // Enrolled controllers start only at the movement boundary. Their
        // otherwise unused QUIC transports would hit the fixed idle deadline.
        if (controllerRoles.includes(role)) continue;
        const client = new NodeClient({ config: actor.config, executable: input.executable });
        clients.set(role, client);
        assert.equal((await client.capabilities.refresh(options)).selectedTransport, "quic");
    }
    await helpers(); assert.equal(observedHelpers.size, 5);
    check("five-independent-public-sdk-helpers-bound-to-production-executable");
    if (input.network_profile === "remote") {
        report.topology = await (input.server_ownership?.kind==="macos-linux-guest" ? observeMacLinuxTopology(input.server_ownership, observedHelpers, input.native_endpoint, input.binary_sha256, input.client_ip, input.source_revision, input.server_binary_sha256) : observeRemoteTopology(input.server_pid, observedHelpers, input.native_endpoint, input.binary_sha256, input.client_ip));
        report.actual_topology_verified = true;
        check("actual-systemd-server-listener-and-distinct-ordinary-user-client-network-namespace-observed");
    }
    const owner = clients.get("participant-1"), late = clients.get("participant-4");
    const others = ["participant-2", "participant-3"].map(role => input.actors[role].identity_id);
    const create = await owner.conversations.create({ participants: others, policy: "persistent" }, mutation("create"));
    const originalReference = create.referenceJson();
    const conversation = await complete(create);
    const cid = conversation.id;
    await writeFile(join(privateDirectory, "original-operation.json"), encode(originalReference), { flag: "wx", mode: 0o600 });
    let view = await conversation.refresh(options);
    assert.equal(view.participants.filter(p => p.active).length, 3);
    const member = (snapshot, role) => {
        const result = snapshot.participants.filter(p => p.identityId === input.actors[role].identity_id);
        assert.equal(result.length, 1); return result[0];
    };
    const recipients = snapshot => snapshot.participants.filter(p => p.active && p.identityId !== input.actors["participant-1"].identity_id).map(p => p.id);
    const send = (client, room, name, targets) => complete(client.conversations.byId(room).send({
        messageId: plan[name + "Message"], to: { first: targets[0], rest: targets.slice(1) }, text: "Synthetic " + name,
    }, mutation(name)));
    const history = async client => {
        const started = performance.now(), cursors = new Set(), messages = new Set();
        let cursor;
        for (let page = 0; page < 8; page++) {
            const remaining = Math.floor(options.timeoutMs - (performance.now() - started));
            assert.ok(remaining > 0);
            const frames = await client.raw.request(client.raw.validate({ v: 2, id: "history_" + randomUUID(),
                ts: new Date().toISOString(), type: "message.history", cid,
                payload: { limit: 50, ...(cursor ? { cursor } : {}) } }), { timeoutMs: remaining });
            const ack = frames.filter(f => f.value.type === "ack"); assert.equal(ack.length, 1);
            for (const frame of frames.filter(f => f.value.type === "message.send")) {
                const id = frame.value.payload.msg_id;
                assert.equal(typeof id, "string"); assert.equal(messages.has(id), false); messages.add(id);
            }
            cursor = ack[0].value.payload.next_cursor ?? null;
            if (cursor === null) {
                report.history_page_counts.push({ pages: page + 1, messages: messages.size });
                return [...messages].sort();
            }
            assert.equal(typeof cursor, "string"); assert.ok(cursor.length > 0);
            assert.equal(cursors.has(cursor), false); cursors.add(cursor);
        }
        throw Error("Bounded history did not reach its terminal page");
    };
    await send(owner, cid, "before", recipients(view));
    await refused(() => late.conversations.byId(cid).refresh(options), 403, "forbidden");
    check("scoped-native-conversation-and-nonmember-current-access-refusal");
    view = await conversation.refresh(options);
    await complete(conversation.participants.add({ identityId: input.actors["participant-4"].identity_id,
        kind: "human", role: "participant", expectedRevision: view.revision }, mutation("add")));
    view = await conversation.refresh(options);
    assert.equal(view.participants.filter(p => p.active).length, 4);
    report.maximum_active_members_observed = 4;
    const firstMembership = member(view, "participant-4");
    phase = "added-member-old-history";
    assert.deepEqual(await history(late), []);
    phase = "included-message-send";
    await send(owner, cid, "included", recipients(view));
    phase = "added-member-current-history";
    assert.deepEqual(await history(late), [plan.includedMessage]);
    check("four-active-participants-and-addition-does-not-expand-older-message-audience");
    view = await conversation.refresh(options);
    const roleInput = { role: "reviewer", expectedConversationRevision: view.revision };
    await complete(conversation.participants.byId(firstMembership.id).update(roleInput, mutation("update")));
    const updated = await conversation.refresh(options);
    assert.equal(member(updated, "participant-4").role, "reviewer");
    await complete(conversation.participants.byId(firstMembership.id).update(roleInput, mutation("update")));
    assert.equal((await conversation.refresh(options)).revision, updated.revision);
    await refused(() => complete(conversation.participants.byId(firstMembership.id).update({ ...roleInput, role: "other" }, mutation("update"))), 409, "operation_conflict");
    await refused(() => complete(conversation.participants.byId(firstMembership.id).update(roleInput, mutation("stale"))), 409, "operation_conflict");
    assert.equal((await conversation.refresh(options)).revision, updated.revision);
    check("membership-update-original-replay-changed-intent-and-stale-revision-guards");
    const fromLate = await late.conversations.byId(cid).send({ messageId: plan.from4Message,
        to: { first: member(updated, "participant-1").id, rest: [] }, text: "Synthetic from4" }, mutation("from4"));
    const oldSendReference = fromLate.referenceJson(); await complete(fromLate);
    view = await conversation.refresh(options);
    await complete(conversation.participants.byId(firstMembership.id).remove({ expectedConversationRevision: view.revision }, mutation("remove")));
    await refused(() => late.conversations.byId(cid).refresh(options), 403, "forbidden");
    await refused(() => late.operations.restore(oldSendReference, late.operationKinds.message_send).wait(options), 403, "forbidden");
    phase = "removed-member-current-snapshot";
    view = await conversation.refresh(options);
    assert.equal(view.participants.filter(p => p.active).length, 3);
    assert.equal(view.participants.some(p => p.identityId === input.actors["participant-4"].identity_id), false);
    await complete(conversation.participants.add({ identityId: input.actors["participant-4"].identity_id,
        kind: "human", role: "participant", expectedRevision: view.revision }, mutation("readd")));
    phase = "readmitted-member-epoch";
    view = await conversation.refresh(options);
    const nextMembership = member(view, "participant-4");
    assert.equal(nextMembership.id, firstMembership.id);
    assert.equal(nextMembership.membershipEpoch, firstMembership.membershipEpoch + 1);
    assert.deepEqual(await history(late), []);
    await refused(() => late.operations.restore(oldSendReference, late.operationKinds.message_send).wait(options), 403, "forbidden");
    await send(owner, cid, "after", recipients(view));
    assert.deepEqual(await history(late), [plan.afterMessage]);
    check("remove-and-readmit-preserve-participant-id-and-fence-old-epoch-history-and-replay");
    const caps = mediaWorkload ? { data: ["reliable-ordered"], audio: ["opus"], video: ["vp8"] } : { data: ["reliable-ordered"], audio: ["g.711-mu"] };
    for (const [index, roles] of sessionRoles.entries()) {
        const session = await complete(conversation.sessions.invite({ sessionId: plan["session" + index],
            to: roles.slice(1).map(role => member(view, role).id), medium: "text-chat", intent: "synchronous-engagement", capabilities: caps }, mutation("invite" + index)));
        for (const role of roles.slice(1)) {
            const peerSession = clients.get(role).conversations.byId(cid).sessions.byId(session.id);
            const before = await peerSession.refresh(options);
            await complete(peerSession.accept({ capabilities: caps, expectedTopologyRevision: before.topologyRevision }, mutation(dataWorkload ? index + "-" + role + "-accept" : "accept" + index)));
        }
    }
    const sessions = [0, 1].map(index => conversation.sessions.byId(plan["session" + index]));
    const simultaneous = await Promise.all(sessions.map(session => session.refresh(options)));
    assert.ok(simultaneous.every(s => s.state === "accepted" && s.media.includes("data")));
    assert.equal(simultaneous.filter(s => s.isMain).length, 1);
    report.concurrent_session_intents_observed = 2;
    check("two-current-accepted-session-intents-coexist-with-one-main-session");
    const initialSession = simultaneous[0];
    await complete(sessions[0].update({ action: "medium-upgrade", media: { first: "audio", rest: [] } }, initialSession.topologyRevision, mutation("upgrade")));
    const upgraded = await sessions[0].refresh(options);
    assert.deepEqual([...upgraded.media].sort(), ["audio", "data"]);
    await refused(() => complete(sessions[0].update({ action: "medium-upgrade", media: { first: mediaWorkload ? "screen" : "video", rest: [] } }, upgraded.topologyRevision, mutation("unsupported"))), 501, "unsupported_feature");
    assert.deepEqual(await sessions[0].refresh(options), upgraded);
    check(mediaWorkload ? "additive-session-capability-intent-and-unoffered-screen-refusal-without-state-change" : "additive-session-capability-intent-and-unoffered-video-refusal-without-state-change");
    if (mediaWorkload) {
        phase = "live-original-data-connections-across-session-av-upgrade";
        const live = await liveSessionUpgrade({ clients, sessions, plan, options, complete, refused,
            participants: Object.fromEntries(["participant-1", "participant-2"].map(role => [role, member(view, role).id])),
            observe: observation => { report.live_data_session_upgrade_observation = observation; } });
        ownerNativeMessageIds.push(...live.messages.filter(v => v.source === "participant-1" || v.target === "participant-1").map(v => v.messageId));
        report.live_data_session_upgrade = live.observation;
        check("original-data-connections-carry-identified-content-before-and-after-both-session-av-upgrades-and-retire-before-main-workload");
    }
    if (dataWorkload) {
        phase = "six-native-data-connection-offers";
        report.native_offers_completed = 0;
        const connections = [], flows = [];
        const priorHistory = new Map();
        for (const role of ["participant-1", "participant-2", "participant-3", "participant-4"])
            priorHistory.set(role, await history(clients.get(role)));
        for (const [sid, roles] of sessionRoles.entries()) for (const role of roles) {
            phase = "native-data-offer-session-" + sid + "-" + role;
            const prefix = sid + "-" + role, source = clients.get(role).conversations.byId(cid).sessions.byId(plan["session" + sid]);
            const connection = await complete(source.connections.offer({ connectionId: plan[prefix + "-connection"],
                streams: { first: { id: plan[prefix + "-stream"], kind: "data", direction: "sendrecv", codecs: { first: "binary", rest: [] } }, rest: mediaWorkload ? [
                    { id: plan[prefix + "-audio"], kind: "audio", direction: "sendrecv", codecs: { first: "opus", rest: [] } },
                    { id: plan[prefix + "-video"], kind: "video", direction: "sendonly", codecs: { first: "vp8", rest: [] } },
                ] : [] } }, mutation(prefix + "-offer")));
            connections.push({ sid, role, prefix, connection });
            report.native_offers_completed = connections.length;
        }
        assert.equal(connections.length, 6);
        phase = "six-native-data-subscriptions";
        for (const target of connections) {
            const roles = sessionRoles[target.sid], previous = roles[(roles.indexOf(target.role) + roles.length - 1) % roles.length];
            const source = connections.find(v => v.sid === target.sid && v.role === previous);
            const current = await target.connection.refresh(options);
            assert.ok(current.streamRevision > 0);
            await complete(target.connection.streams.subscribe({ selectors: { first: { stream: plan[source.prefix + "-stream"] }, rest: mediaWorkload ? [
                { stream: plan[source.prefix + "-audio"] }, { stream: plan[source.prefix + "-video"] },
            ] : [] },
                expectedStreamRevision: current.streamRevision }, mutation(target.prefix + "-select")));
            const negotiated = await target.connection.media.refresh(options);
            assert.ok(negotiated.streams.some(s => s.id === plan[target.prefix + "-stream"] && s.medium === "data"));
            assert.ok(negotiated.streams.some(s => s.id === plan[source.prefix + "-stream"] && s.medium === "data"));
            const payload = Buffer.from(Array.from({ length: 65536 }, (_, offset) => (offset * 13 + connections.indexOf(source) * 29) % 256));
            flows.push({ source, target, payload, intent: { messageId: plan[source.prefix + "-message"],
                to: { first: member(view, target.role).id, rest: [] }, payload: new SdkBinaryData(payload) } });
        }
        const heldStart = performance.now();
        const before = await Promise.all(connections.map(v => v.connection.refresh(options)));
        assert.ok(before.every(v => ["ready", "active"].includes(v.state)));
        assert.equal(new Set(before.map(v => v.id)).size, 6);
        phase = "six-native-reliable-data-flow";
        report.native_data_submissions = report.native_data_completions = report.native_data_receives = 0;
        const accepted = await Promise.all(flows.map(async flow => {
            let step = "submission";
            try {
                const operation = await flow.source.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.sendData(flow.intent, mutation(flow.source.prefix + "-data"));
                report.native_data_submissions++;
                step = "completion";
                const result = await complete(operation);
                report.native_data_completions++;
                return result;
            } catch (error) {
                dataFailures.set(error, { step, source_role: flow.source.role, session_index: flow.source.sid });
                throw error;
            }
        }));
        const received = await Promise.all(flows.map(async flow => {
            try {
                const result = await flow.target.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.receiveData(options);
                report.native_data_receives++;
                return result;
            } catch (error) {
                dataFailures.set(error, { step: "receive", target_role: flow.target.role, session_index: flow.target.sid });
                throw error;
            }
        }));
        for (const [index, flow] of flows.entries()) {
            const value = received[index];
            assert.equal(accepted[index].boundary, "core-queue-admission");
            assert.equal(accepted[index].connectionId, flow.source.connection.id);
            assert.equal(value.messageId, flow.intent.messageId);
            assert.equal(value.operationId, plan[flow.source.prefix + "-data"]);
            assert.equal(value.fromParticipant, member(view, flow.source.role).id);
            assert.equal(value.payload.kind, "binary");
            assert.deepEqual(Buffer.from(value.payload.value), flow.payload);
        }
        const after = await Promise.all(connections.map(v => v.connection.refresh(options)));
        assert.ok(after.every((v, index) => ["ready", "active"].includes(v.state) && v.id === before[index].id && v.generation === before[index].generation));
        const liveSessions = await Promise.all(sessions.map(v => v.refresh(options)));
        assert.ok(liveSessions.every(v => !["ended", "failed", "cancelled", "rejected"].includes(v.state)));
        assert.equal(liveSessions.filter(v => v.isMain).length, 1);
        report.actual_connection_workload_observed = true;
        report.data_workload = { connections: 6, participants: 4, sessions: 2, connection_counts_by_session: [4, 2],
            before_states: before.map(v => v.state), after_states: after.map(v => v.state),
            verified_payloads: 6, bytes_per_payload: 65536, total_received_bytes: 6 * 65536,
            held_observation_milliseconds: Math.round(performance.now() - heldStart),
            payload_sha256: flows.map(v => hash(v.payload)), audio_video_workload_verified: false };
        check("six-live-native-data-connections-carry-six-independent-full-size-payloads-across-four-participants-two-sessions");
        phase = "owned-local-topology-observation";
        report.topology = input.network_profile === "local" ?
            await observeLocalTopology(input.server_pid, observedHelpers, input.native_endpoint, input.binary_sha256) :
            await (input.server_ownership?.kind==="macos-linux-guest" ? observeMacLinuxTopology(input.server_ownership, observedHelpers, input.native_endpoint, input.binary_sha256, input.client_ip, input.source_revision, input.server_binary_sha256) : observeRemoteTopology(input.server_pid, observedHelpers, input.native_endpoint, input.binary_sha256, input.client_ip));
        report.actual_topology_verified = true;
        check(input.network_profile === "local" ? "actual-server-loopback-listener-and-original-helper-network-namespace-observed" : "actual-server-owned-veth-listener-and-distinct-original-client-network-namespace-observed");
        phase = "native-data-original-replay";
        const flow = flows[0], sender = flow.source.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media;
        assert.deepEqual(await complete(sender.sendData(flow.intent, mutation(flow.source.prefix + "-data"))), accepted[0]);
        await assert.rejects(flow.target.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.receiveData({ timeoutMs: 120 }), error => error.kind === "deadline" && error.certainty === "read-only");
        check("native-data-original-replay-recovers-receipt-without-second-delivery");
        // Native admission shares the durable addressed-message history. Check
        // exact sender/recipient IDs rather than hiding binary journal entries
        // from the history assertions once actual publication succeeds.
        for (const [role, previous] of priorHistory) {
            const nativeIds = flows.filter(v => v.source.role === role || v.target.role === role).map(v => v.intent.messageId);
            assert.deepEqual(await history(clients.get(role)), [...previous, ...nativeIds].sort());
            if (role === "participant-1") ownerNativeMessageIds.push(...nativeIds);
        }
        report.data_workload.frozen_message_history_audiences_verified = true;
        check("native-data-history-preserves-exact-original-sender-and-addressed-recipient-audiences");
        if (mediaWorkload) {
            // The reserve Device has no workload. Do not hold its fixed SDK
            // owner unused across the native transport's30second idle bound.
            phase = "reserved-device-initial-helper-close-before-native-av";
            const beforeClose = await helpers();
            await clients.get("replacement-1").close({ timeoutMs: 5000 });
            clients.delete("replacement-1");
            const afterClose = await helpers();
            const retired = [...beforeClose].filter(pid => !afterClose.has(pid));
            assert.equal(beforeClose.size, 5); assert.equal(afterClose.size, 4);
            assert.equal(retired.length, 1); assert.equal(await processExists(retired[0]), false);
            report.replacement_activation = { initial_helper_joined_before_workload: true,
                original_device_config_sha256: report.sdk_config_hashes["replacement-1"], new_helper_created: false,
                original_create_reference_preserved: false, mutation_replayed: false };
            check("reserved-device-initial-helper-joins-before-sixty-second-workload");
            phase = "six-native-opus-vp8-sixty-second-workload";
            const controllers = [];
            for (const role of controllerRoles) {
                const client = new NodeClient({ config: input.actors[role].config, executable: input.executable });
                clients.set(role, client); controllers.push(client);
                assert.equal((await client.capabilities.refresh(options)).selectedTransport, "quic");
            }
            await helpers(); assert.equal(observedHelpers.size, 7);
            const sourceManifest = JSON.parse(await readFile(join(input.media_source_directory, "manifest.json"), "utf8"));
            const sourcePrograms = {};
            for (const index of [0, 4]) {
                const bytes = await readFile(join(input.media_source_directory, "source-" + index + ".json"));
                assert.equal(hash(bytes), sourceManifest.sources[index].input_sha256);
                const source = JSON.parse(bytes);
                sourcePrograms[index] = { audioPackets: source.audio.map(packet => [...Buffer.from(packet, "base64")]),
                    videoProgram: liveVideoProgramFromManifest(input.media_source_directory, sourceManifest, index) };
            }
            report.native_av = await nativeAv(connections, sessionRoles, input.media_source_directory, input.media_output_directory, plan,
                observation => { report.native_av_observation = observation; }, async workload => {
                    const observed = await observeMovementSessions({ workload, primarySessions: sessions,
                        controllerSessions: controllers.map((client, index) => client.conversations.byId(cid).sessions.byId(sessions[index].id)),
                        run: async current => {
                            await waitForProgrammeOffset(current, 2000);
                            return deviceMovementWorkload({ workload: current, controllers, conversationId: cid, sessions,
                            connections, streamIds: connections.map(item => ({ audio: plan[item.prefix + "-audio"],
                                video: plan[item.prefix + "-video"], data: plan[item.prefix + "-stream"] })),
                            dataClients: connections.map(item => clients.get(item.role)),
                            participantIds: connections.map(item => member(view, item.role).id),
                            targets: ["participant-1", "replacement-1"].map(role => ({
                                endpointId: input.actors[role].media_endpoint_id, deviceId: input.actors[role].device_id,
                                nativeConfig: input.actors[role].native_config })), sourcePrograms, privateRoot,
                            directory: join(privateDirectory, "movement"), executable: input.physical_peer_executable,
                            executableSha256: input.physical_peer_sha256,
                            currentEnds: connections.map((item, index) => ({ index, operationId: plan[item.prefix + "-end"] })),
                            });
                        } });
                    report.current_control_session_observation = observed.observation;
                    report.device_movement = observed.value.observation;
                    return observed.value.observation;
                });
            report.replacement_media_ownership_observed = report.device_movement.original_moves_completed === TOTAL_MOVES;
            report.data_workload.current_owner_end = report.device_movement.current_owner_end;
            phase = "original-and-physical-capture-custody";
            const normalized = await physicalMovementSegments({ directory: join(privateDirectory, "movement"),
                movement: report.device_movement,
                plan: JSON.parse(await readFile(join(privateDirectory, "movement/plan.json"), "utf8")),
                packetWorkersJoined: report.native_av.original_receive_tasks_joined,
                conversationId: cid, sessionIds: sessions.map(session => session.id),
                connectionIds: connections.map(item => item.connection.id),
                streamIds: connections.map(item => ({ audio: plan[item.prefix + "-audio"], video: plan[item.prefix + "-video"], data: plan[item.prefix + "-stream"] })),
                executableSha256: input.physical_peer_sha256 });
            const normalizedPath = join(input.media_output_directory, "movement-segments.json");
            await writeFile(normalizedPath, encode(normalized), { flag: "wx", mode: 0o600 });
            report.native_movement_capture = { format: normalized.format, sha256: hash(await readFile(normalizedPath)),
                original_capture_sha256: await Promise.all(connections.map(async (_, index) =>
                    hash(await readFile(join(input.media_output_directory, "received-" + index + ".json"))))),
                original_move_records_bound: normalized.original_move_records_bound,
                reliable_data_exchanges_bound: normalized.reliable_data_exchanges_bound };
            assert.equal(report.device_movement.replacement_data_io_verified, true);
            const movementMessages = report.device_movement.replacement_data_exchanges.flatMap(exchange => exchange.messages);
            assert.equal(movementMessages.length, TOTAL_MOVES * 2);
            assert.equal(new Set(movementMessages.map(message => message.message_id)).size, TOTAL_MOVES * 2);
            for (const [role, previous] of priorHistory) {
                const participant = member(view, role).id;
                const additional = movementMessages.filter(message => [message.from_participant, message.to_participant].includes(participant))
                    .map(message => message.message_id);
                const baseline = flows.filter(flow => flow.source.role === role || flow.target.role === role).map(flow => flow.intent.messageId);
                assert.deepEqual(await history(clients.get(role)), [...previous, ...baseline, ...additional].sort());
                if (role === "participant-1") ownerNativeMessageIds.push(...additional);
            }
            report.data_workload.replacement_data_history_audiences_verified = true;
            for (const role of controllerRoles) {
                await clients.get(role).close({ timeoutMs: 5000 }); clients.delete(role);
            }
            check("six-native-routes-four-scheduled-original-moves-and-six-current-owner-ends-with-original-programme-clock");
        }
        phase = "six-native-connections-original-end";
        // Media ends already belong to four retained SDKs and two final peers.
        // Never submit another end against the moved originals or foreign peers.
        for (const item of mediaWorkload ? [] : connections) {
            const originalEnd = await item.connection.end({ reasonCode: 200, reason: "Synthetic workload complete" },
                mutation(item.prefix + "-end"));
            await complete(originalEnd);
        }
        report.data_workload.original_end_operations_completed = mediaWorkload ? 4 : connections.length;
        report.data_workload.physical_owner_end_operations_completed = mediaWorkload ? 2 : 0;
        report.data_workload.original_retired_by_replacement = mediaWorkload ? 2 : 0;
        // End completion proves local engine termination. Its separate durable
        // retirement snapshot may follow later (the public end contract). Read
        // only the original IDs under one ordinary SDK observation deadline.
        const retirementStarted = performance.now(), retirementUntil = retirementStarted + options.timeoutMs;
        report.data_workload.retirement_observation_budget_ms = options.timeoutMs;
        report.data_workload.retirement_observations = 0;
        report.data_workload.retirement_observer = "single-current-authorized-conversation-snapshot";
        for (;;) {
            const remaining = Math.floor(retirementUntil - performance.now());
            assert.ok(remaining > 0);
            const snapshot = await connectionSnapshots(owner, cid, connections.map(v => ({
                id: v.connection.id, sessionId: plan["session" + v.sid],
            })), { timeoutMs: remaining });
            const retired = snapshot.connections;
            report.data_workload.retirement_observations++;
            report.data_workload.retirement_snapshot_pages = snapshot.pages;
            report.data_workload.retirement_snapshot_revision = snapshot.revision;
            report.data_workload.after_end_states = retired.map(v => v.state);
            if (retired.every((v, index) => v.id === connections[index].connection.id && v.state === "retired" && v.generation > before[index].generation)) break;
            await new Promise(done => setTimeout(done, Math.min(50, Math.max(0, retirementUntil - performance.now()))));
        }
        report.data_workload.retirement_observation_elapsed_ms = Math.round(performance.now() - retirementStarted);
        report.data_workload.original_connections_retired = 6;
        for (const role of ["participant-1", "participant-2", "participant-3", "participant-4"])
            assert.equal((await clients.get(role).media.dataStats(options)).queuedBytes, 0);
        check("all-six-original-native-connections-retire-and-owned-data-queues-drain");
    }
    for (const [index, session] of sessions.entries()) {
        const current = await session.refresh(options);
        await complete(session.end({ expectedTopologyRevision: current.topologyRevision, reasonCode: 200, reason: "Synthetic fixture complete" }, mutation("end" + index)));
        assert.equal((await session.refresh(options)).state, "ended");
    }
    check("both-original-session-intents-end-through-public-sdk");
    phase = "original-owner-sdk-close-before-independent-device-recovery";
    await owner.close({ timeoutMs: 5000 });
    clients.delete("participant-1");
    let replacement = clients.get("replacement-1");
    if (mediaWorkload) {
        phase = "independent-device-explicit-current-sdk-activation";
        const actor = input.actors["replacement-1"];
        assert.equal(hash(await readFile(actor.config)), report.replacement_activation.original_device_config_sha256);
        replacement = new NodeClient({ config: actor.config, executable: input.executable });
        clients.set("replacement-1", replacement);
        assert.equal((await replacement.capabilities.refresh(options)).selectedTransport, "quic");
        await helpers(); assert.equal(observedHelpers.size, 8);
        report.replacement_activation.new_helper_created = true;
        check("independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins");
    }
    phase = "independent-device-original-create-receipt-recovery";
    const originalRecovery = replacement.operations.restore(originalReference, replacement.operationKinds.conversation_create);
    assert.deepEqual(originalRecovery.referenceJson(), originalReference);
    if (mediaWorkload) report.replacement_activation.original_create_reference_preserved = true;
    const restored = await complete(originalRecovery);
    assert.equal(restored.id, cid);
    phase = "independent-device-current-membership-read";
    assert.equal((await restored.refresh(options)).participants.filter(p => p.active).length, 4);
    phase = "independent-device-original-history-audience-read";
    assert.deepEqual(await history(replacement), [plan.beforeMessage, plan.includedMessage, plan.from4Message, plan.afterMessage, ...ownerNativeMessageIds].sort());
    check("independent-device-recovers-original-create-and-history-after-original-helper-close");
    report.passed = true;
} catch (error) {
    report.failure = { phase, name: error.name, kind: error.kind ?? null, certainty: error.certainty ?? null, ...operationFailure(error) };
    if (dataFailures.has(error)) report.failure.data_context = dataFailures.get(error);
    if (["message.send", "operation.get", "connection.offer", "stream.subscribe"].includes(error.operation?.command))
        report.failure.command = error.operation.command;
    if (Number.isInteger(error.problem?.code) && error.problem.code >= 100 && error.problem.code <= 599
        && /^[a-z-]{1,32}$/.test(error.problem.category) && /^[a-z_]{1,64}$/.test(error.problem.reason))
        report.failure.problem = { code: error.problem.code, category: error.problem.category, reason: error.problem.reason };
    if (dataWorkload && error.operation?.command === "connection.offer") {
        const role = [...clients.keys()].find(v => phase.endsWith("-" + v));
        if (role) try {
            const client = clients.get(role), key = error.operation;
            const frames = await client.raw.request(client.raw.validate({ v: 2, id: "status_" + randomUUID(), ts: new Date().toISOString(),
                type: "operation.get", cid: key.scope.conversationId, sid: key.scope.sessionId, connid: key.scope.connectionId,
                payload: { operation_id: key.operationId } }), options);
            const states = frames.filter(v => v.value.type === "operation.updated").map(v => v.value.payload.state);
            assert.ok(states.every(v => ["accepted", "preparing", "ready", "committing", "completed", "failed", "cancelled", "unknown", "interrupted"].includes(v)));
            report.failure.original_offer_status = states;
        } catch (statusError) { report.failure.original_status_read_kind = statusError.kind ?? statusError.name; }
    }
    const position = /scripts\/check-transport-conversation\.mjs:(\d+):(\d+)/.exec(String(error.stack));
    if (position) report.failure.source = { file: "scripts/check-transport-conversation.mjs", line: Number(position[1]), column: Number(position[2]) };
    report.failure.source_stack = [...String(error.stack).matchAll(/scripts\/check-transport-conversation\.mjs:(\d+):(\d+)/g)].slice(0, 4).map(p => ({ line: Number(p[1]), column: Number(p[2]) }));
    // Describe assertion shapes without uploading identifiers, bodies or keys.
    const shape = value => Array.isArray(value) ? { type: "array", length: value.length, elements: [...new Set(value.map(v => typeof v))] } : { type: typeof value };
    if (error.name === "AssertionError") report.failure.assertion = { actual: shape(error.actual), expected: shape(error.expected) };
    await writeFile(join(privateDirectory, "failure.log"), String(error.stack), { mode: 0o600 });
} finally {
    if (matrixLifecycle) await matrixLifecycle.terminal();
    for (const client of clients.values()) {
        try { await client.close({ timeoutMs: 5000 }); }
        catch { report.passed = false; report.helper_close_failed = true; }
    }
    for (const pid of observedHelpers) {
        if (await processExists(pid)) { report.passed = false; report.helper_survived = true; }
    }
    report.observed_helpers = observedHelpers.size;
    report.all_observed_helpers_joined = !report.helper_survived && !report.helper_close_failed;
    if (matrixLifecycle) {
        const observed = await matrixLifecycle.retired({observedHelpers: observedHelpers.size,
            helpersJoined: report.all_observed_helpers_joined, closeFailed: report.helper_close_failed === true});
        report.retirement = observed.retirement;
        report.native_idle_observation = observed.idle_observer;
        report.passed = Boolean(report.passed && observed.retirement.passed && observed.idle_observer.passed);
    }
    for (const [role, actor] of Object.entries(input.actors))
        if (hash(await readFile(actor.config)) !== report.sdk_config_hashes[role]) report.passed = false;
    if (hash(await readFile(input.executable)) !== input.binary_sha256) report.passed = false;
    if (mediaWorkload && hash(await readFile(input.physical_peer_executable)) !== input.physical_peer_sha256) report.passed = false;
    await writeFile(output, encode(report), { flag: "wx", mode: 0o600 });
}
process.stdout.write(JSON.stringify({ conversation_control_passed: report.passed, checks: report.checks.length, cell_qualified: false }) + "\n");
if (!report.passed) process.exitCode = 1;
