// Actual public Node SDK/native helper scenario. No Store, media or provider IO.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { readFile, writeFile, mkdir, stat, realpath, open, readdir, readlink } from "node:fs/promises";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { NodeClient, SdkBinaryData } from "../sdk/typescript/dist/index.js";

const root = fileURLToPath(new URL("../", import.meta.url));
const inputPath = resolve(process.argv[2] ?? ""), output = resolve(process.argv[3] ?? "");
const dataWorkload = process.argv[4] === "--data-workload";
assert.ok(process.argv.length === 4 || (process.argv.length === 5 && dataWorkload));
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const encode = value => JSON.stringify(value, null, 2) + "\n";
const privateRoot = await realpath(join(root, ".local"));
assert.ok((await realpath(inputPath)).startsWith(privateRoot + "/") && output.startsWith(privateRoot + "/"));
assert.equal((await stat(inputPath)).mode & 0o077, 0);
const input = JSON.parse(await readFile(inputPath, "utf8"));
assert.equal(input.format, "connect/transport-conversation-input/1");
assert.ok(["EdDSA", "ES256"].includes(input.signing_algorithm));
assert.deepEqual(Object.keys(input.actors).sort(), ["participant-1", "participant-2", "participant-3", "participant-4", "replacement-1"]);
assert.equal(hash(await readFile(input.executable)), input.binary_sha256);
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
    for (const field of ["connection", "stream", "offer", "select", "data", "message", "end", "accept"]) plan[prefix + "-" + field] = "workload_" + randomUUID();
}
const planFile = await open(join(privateDirectory, "plan.json"), "wx", 0o600);
try { await planFile.writeFile(encode(plan)); await planFile.sync(); } finally { await planFile.close(); }
const directory = await open(privateDirectory, "r"); try { await directory.sync(); } finally { await directory.close(); }
const options = { timeoutMs: 8000 };
const mutation = name => ({ ...options, operationId: plan[name] });
const clients = new Map();
const observedHelpers = new Set();
const report = { format: "connect/transport-conversation-control/1", passed: false, checks: [],
    source_revision: input.source_revision, binary_sha256: input.binary_sha256, signing_algorithm: input.signing_algorithm, history_page_counts: [],
    node: process.version, plan_sha256: hash(await readFile(join(privateDirectory, "plan.json"))),
    input_sha256: hash(await readFile(inputPath)), sdk_config_hashes: {},
    maximum_active_members_observed: 0, concurrent_session_intents_observed: 0,
    actual_connection_workload_observed: false, live_media_upgrade_observed: false,
    replacement_media_ownership_observed: false, actual_topology_verified: false,
    cell_qualified: false, task_complete: false, whole_gate_qualified: false,
    workload_mode: dataWorkload ? "six-native-data-connections" : "control-only",
    scope: "One source-bound Ubuntu/local/native signing cohort public Node SDK membership/audience/Session control scenario. Optional workload opens six actual native data Connections across four participants/two Sessions and verifies 64KiB reliable payloads. Audio/video duplex, live upgrade, replacement media ownership, packet capture, browser, remote topology, SIP, TURN, model/provider and complete matrix cells remain unqualified." };
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
    const pids = execFileSync("ps", ["-o", "pid=", "--ppid", String(process.pid)], { encoding: "utf8" }).trim().split(/\s+/).filter(Boolean);
    for (const pid of pids) {
        try { if (hash(await readFile(`/proc/${pid}/exe`)) === input.binary_sha256) observedHelpers.add(Number(pid)); }
        catch (error) { if (!["ENOENT", "ESRCH"].includes(error.code)) throw error; }
    }
}
try {
    for (const [role, actor] of Object.entries(input.actors)) {
        assert.ok((await realpath(actor.config)).startsWith(privateRoot + "/"));
        assert.equal((await stat(actor.config)).mode & 0o077, 0);
        report.sdk_config_hashes[role] = hash(await readFile(actor.config));
        const client = new NodeClient({ config: actor.config, executable: input.executable });
        clients.set(role, client);
        assert.equal((await client.capabilities.refresh(options)).selectedTransport, "quic");
    }
    await helpers(); assert.equal(observedHelpers.size, 5);
    check("five-independent-public-sdk-helpers-bound-to-production-executable");
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
    const caps = { data: ["reliable-ordered"], audio: ["g.711-mu"] };
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
    await refused(() => complete(sessions[0].update({ action: "medium-upgrade", media: { first: "video", rest: [] } }, upgraded.topologyRevision, mutation("unsupported"))), 501, "unsupported_feature");
    assert.deepEqual(await sessions[0].refresh(options), upgraded);
    check("additive-session-capability-intent-and-unoffered-video-refusal-without-state-change");
    if (dataWorkload) {
        phase = "six-native-data-connection-offers";
        report.native_offers_completed = 0;
        const connections = [], flows = [];
        for (const [sid, roles] of sessionRoles.entries()) for (const role of roles) {
            phase = "native-data-offer-session-" + sid + "-" + role;
            const prefix = sid + "-" + role, source = clients.get(role).conversations.byId(cid).sessions.byId(plan["session" + sid]);
            const connection = await complete(source.connections.offer({ connectionId: plan[prefix + "-connection"],
                streams: { first: { id: plan[prefix + "-stream"], kind: "data", direction: "sendrecv", codecs: { first: "binary", rest: [] } }, rest: [] } }, mutation(prefix + "-offer")));
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
            await complete(target.connection.streams.subscribe({ selectors: { first: { stream: plan[source.prefix + "-stream"] }, rest: [] },
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
        const accepted = await Promise.all(flows.map(flow => complete(flow.source.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.sendData(flow.intent, mutation(flow.source.prefix + "-data")))));
        const received = await Promise.all(flows.map(flow => flow.target.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.receiveData(options)));
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
        assert.equal(process.platform, "linux"); assert.equal(process.arch, "x64");
        assert.equal(hash(await readFile(`/proc/${input.server_pid}/exe`)), input.binary_sha256);
        const serverNamespace = await readlink(`/proc/${input.server_pid}/ns/net`);
        for (const pid of observedHelpers) assert.equal(await readlink(`/proc/${pid}/ns/net`), serverNamespace);
        const inodes = new Set();
        for (const fd of await readdir(`/proc/${input.server_pid}/fd`)) {
            try { const value = await readlink(`/proc/${input.server_pid}/fd/${fd}`); const match = /^socket:\[(\d+)\]$/.exec(value); if (match) inodes.add(match[1]); }
            catch (error) { if (error.code !== "ENOENT") throw error; }
        }
        const nativePort = Number(input.native_endpoint.split(":").at(-1));
        const udp = (await readFile(`/proc/${input.server_pid}/net/udp`, "utf8")).trim().split("\n").slice(1).map(line => line.trim().split(/\s+/));
        assert.ok(udp.some(row => row[1].split(":")[0] === "0100007F" && parseInt(row[1].split(":")[1], 16) === nativePort && inodes.has(row[9])));
        report.local_topology = { same_network_namespace_observed: true, server_owned_loopback_udp_bind_observed: true,
            helpers_in_server_namespace: observedHelpers.size, server_executable_sha256: input.binary_sha256, namespace_sha256: hash(serverNamespace) };
        report.actual_topology_verified = true;
        check("actual-server-loopback-listener-and-original-helper-network-namespace-observed");
        phase = "native-data-original-replay";
        const flow = flows[0], sender = flow.source.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media;
        assert.deepEqual(await complete(sender.sendData(flow.intent, mutation(flow.source.prefix + "-data"))), accepted[0]);
        await assert.rejects(flow.target.connection.streams.byId(plan[flow.source.prefix + "-stream"]).media.receiveData({ timeoutMs: 120 }), error => error.kind === "deadline" && error.certainty === "read-only");
        check("native-data-original-replay-recovers-receipt-without-second-delivery");
        phase = "six-native-connections-original-end";
        for (const item of connections) await complete(item.connection.end({ reasonCode: 200, reason: "Synthetic workload complete" }, mutation(item.prefix + "-end")));
        const retired = await Promise.all(connections.map(v => v.connection.refresh(options)));
        assert.ok(retired.every(v => v.state === "retired"));
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
    await owner.close({ timeoutMs: 5000 });
    clients.delete("participant-1");
    const replacement = clients.get("replacement-1");
    const restored = await complete(replacement.operations.restore(originalReference, replacement.operationKinds.conversation_create));
    assert.equal(restored.id, cid);
    assert.equal((await restored.refresh(options)).participants.filter(p => p.active).length, 4);
    assert.deepEqual(await history(replacement), [plan.beforeMessage, plan.includedMessage, plan.from4Message, plan.afterMessage].sort());
    check("independent-device-recovers-original-create-and-history-after-original-helper-close");
    report.passed = true;
} catch (error) {
    report.failure = { phase, name: error.name, kind: error.kind ?? null, certainty: error.certainty ?? null };
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
    for (const client of clients.values()) {
        try { await client.close({ timeoutMs: 5000 }); }
        catch { report.passed = false; report.helper_close_failed = true; }
    }
    for (const pid of observedHelpers) {
        try { await stat(`/proc/${pid}`); report.passed = false; report.helper_survived = true; }
        catch (error) { if (error.code !== "ENOENT") throw error; }
    }
    report.observed_helpers = observedHelpers.size;
    report.all_observed_helpers_joined = !report.helper_survived && !report.helper_close_failed;
    for (const [role, actor] of Object.entries(input.actors))
        if (hash(await readFile(actor.config)) !== report.sdk_config_hashes[role]) report.passed = false;
    if (hash(await readFile(input.executable)) !== input.binary_sha256) report.passed = false;
    await writeFile(output, encode(report), { flag: "wx", mode: 0o600 });
}
process.stdout.write(JSON.stringify({ conversation_control_passed: report.passed, checks: report.checks.length, cell_qualified: false }) + "\n");
if (!report.passed) process.exitCode = 1;
