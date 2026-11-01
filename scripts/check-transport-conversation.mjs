// Actual public Node SDK/native helper scenario. No Store, media or provider IO.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { readFile, writeFile, mkdir, stat, realpath, open } from "node:fs/promises";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { NodeClient } from "../sdk/typescript/dist/node_sdk.js";

const root = fileURLToPath(new URL("../", import.meta.url));
const inputPath = resolve(process.argv[2] ?? ""), output = resolve(process.argv[3] ?? "");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const encode = value => JSON.stringify(value, null, 2) + "\n";
const privateRoot = await realpath(join(root, ".local"));
assert.ok((await realpath(inputPath)).startsWith(privateRoot + "/") && output.startsWith(privateRoot + "/"));
assert.equal((await stat(inputPath)).mode & 0o077, 0);
const input = JSON.parse(await readFile(inputPath, "utf8"));
assert.equal(input.format, "connect/transport-conversation-input/1");
assert.deepEqual(Object.keys(input.actors).sort(), ["participant-1", "participant-2", "participant-3", "participant-4", "replacement-1"]);
assert.equal(hash(await readFile(input.executable)), input.binary_sha256);
const privateDirectory = join(dirname(inputPath), "conversation-control");
await mkdir(privateDirectory, { mode: 0o700 });
const plan = Object.fromEntries([
    "create", "before", "included", "from4", "after", "add", "update", "stale", "remove", "readd",
    "session0", "session1", "invite0", "invite1", "accept0", "accept1", "upgrade", "unsupported", "end0", "end1",
].map(name => [name, "transport_" + randomUUID()]));
for (const name of ["before", "included", "from4", "after"]) plan[name + "Message"] = "message_" + randomUUID();
const planFile = await open(join(privateDirectory, "plan.json"), "wx", 0o600);
try { await planFile.writeFile(encode(plan)); await planFile.sync(); } finally { await planFile.close(); }
const directory = await open(privateDirectory, "r"); try { await directory.sync(); } finally { await directory.close(); }
const options = { timeoutMs: 8000 };
const mutation = name => ({ ...options, operationId: plan[name] });
const clients = new Map();
const observedHelpers = new Set();
const report = { format: "connect/transport-conversation-control/1", passed: false, checks: [],
    source_revision: input.source_revision, binary_sha256: input.binary_sha256,
    node: process.version, plan_sha256: hash(await readFile(join(privateDirectory, "plan.json"))),
    input_sha256: hash(await readFile(inputPath)), sdk_config_hashes: {},
    maximum_active_members_observed: 0, concurrent_session_intents_observed: 0,
    actual_connection_workload_observed: false, live_media_upgrade_observed: false,
    replacement_media_ownership_observed: false, actual_topology_verified: false,
    cell_qualified: false, task_complete: false, whole_gate_qualified: false,
    scope: "One actual Ubuntu/local/native EdDSA public Node SDK control scenario: membership/audiences, two accepted Session intents, additive capability intent/refusal and original-operation recovery across an independent Device. No six-Connection workload, media ownership/duplex, packet capture, browser, remote topology, SIP, TURN, model or provider execution." };
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
        const frames = await client.raw.request(client.raw.validate({ v: 2, id: "history_" + randomUUID(),
            ts: new Date().toISOString(), type: "message.history", cid, payload: { limit: 50 } }), options);
        const ack = frames.filter(f => f.value.type === "ack"); assert.equal(ack.length, 1);
        assert.equal(ack[0].value.payload.next_cursor ?? null, null);
        return frames.filter(f => f.value.type === "message.send").map(f => f.value.payload.msg_id).sort();
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
    view = await conversation.refresh(options);
    assert.equal(member(view, "participant-4").active, false);
    await complete(conversation.participants.add({ identityId: input.actors["participant-4"].identity_id,
        kind: "human", role: "participant", expectedRevision: view.revision }, mutation("readd")));
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
    for (const [index, role] of ["participant-2", "participant-3"].entries()) {
        const session = await complete(conversation.sessions.invite({ sessionId: plan["session" + index],
            to: [member(view, role).id], medium: "text-chat", intent: "synchronous-engagement", capabilities: caps }, mutation("invite" + index)));
        const peerSession = clients.get(role).conversations.byId(cid).sessions.byId(session.id);
        const before = await peerSession.refresh(options);
        await complete(peerSession.accept({ capabilities: caps, expectedTopologyRevision: before.topologyRevision }, mutation("accept" + index)));
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
    const position = /scripts\/check-transport-conversation\.mjs:(\d+):(\d+)/.exec(String(error.stack));
    if (position) report.failure.source = { file: "scripts/check-transport-conversation.mjs", line: Number(position[1]), column: Number(position[2]) };
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
