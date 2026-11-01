// Application routing/failure tests. The fixture models the public SDK shape;
// actual installed-package/server/browser qualification remains separate.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { AppIntents } from "./intents.mjs";
import { AppGateway, publicError } from "./gateway.mjs";
const root = fileURLToPath(new URL("../../../.local/gate-6/reference-app/", import.meta.url));
await mkdir(root, { recursive: true, mode: 0o700 });
const frozen = JSON.parse(await readFile(new URL("../../../contracts/schemas/uctp-envelope-2.schema.json", import.meta.url), "utf8"));
const actors = { owner: { identityId: "id-owner", config: "private-owner.toml", label: "Owner" },
    organizer: { identityId: "id-organizer", config: "private-organizer.toml", label: "Organizer" } };
const commands = ["conversation.create", "message.send", "session.invite", "session.accept", "session.reject", "session.cancel", "session.end", "connection.update"];
function fixture() {
    const service = { records: new Map(), calls: [], reads: [], clients: [], lost: false, allowed: true,
        membershipEpoch: 1, topology: 3, streamRevision: 8 };
    const permitted = () => { if (!service.allowed) throw Object.assign(Error("private refusal reason"), { kind: "authentication", certainty: "read-only" }); };
    service.createClient = actor => {
        const current = { actor, closed: false, closes: 0 }; service.clients.push(current);
        const operation = reference => ({
            referenceJson: () => structuredClone(reference),
            status: async () => { permitted(); service.reads.push(reference.operation_id);
                const value = service.records.get(reference.operation_id);
                if (!value) throw Object.assign(Error("not found"), { kind: "application", certainty: "read-only" });
                return structuredClone(value.snapshot); },
            wait: async () => { permitted(); const value = service.records.get(reference.operation_id);
                return structuredClone({ snapshot: value.snapshot, value: value.value }); },
        });
        const mutate = async (command, scope, sdk, options, value) => {
            permitted(); service.calls.push({ command, sdk: structuredClone(sdk), operationId: options.operationId });
            const reference = { schema: "connect-operation-reference/1", operation_id: options.operationId,
                command, scope: Object.fromEntries(Object.entries(scope).map(([k, v]) => [k.replace(/[A-Z]/g, c => "_" + c.toLowerCase()), v])) };
            service.records.set(options.operationId, { snapshot: { key: { operationId: options.operationId, command, scope },
                state: "completed", revision: 1 }, value });
            if (service.lost) { service.lost = false; throw Object.assign(Error("private transport credential"), { kind: "transport-uncertain", certainty: "uncertain" }); }
            return operation(reference);
        };
        const conversation = cid => ({
            refresh: async () => { permitted(); return { id: cid, closed: false, participants: Object.entries(actors).map(([role, value]) =>
                ({ id: "participant-" + role, identityId: value.identityId, active: true, membershipEpoch: service.membershipEpoch })) }; },
            send: (sdk, options) => mutate("message.send", { conversationId: cid }, sdk, options, { messageId: sdk.messageId }),
            sessions: {
                invite: (sdk, options) => mutate("session.invite", { conversationId: cid, sessionId: sdk.sessionId }, sdk, options, { id: sdk.sessionId }),
                byId: sid => {
                    const session = { refresh: async () => ({ id: sid, topologyRevision: service.topology,
                        capabilitiesOffer: { data: ["reliable-ordered"] } }), connections: { byId: connectionId => ({
                        refresh: async () => ({ id: connectionId, streamRevision: service.streamRevision }),
                        update: (update, revision, options) => mutate("connection.update", { conversationId: cid, sessionId: sid, connectionId },
                            { update, revision }, options, { connection: { id: connectionId } }),
                    }) } };
                    for (const [method, command] of Object.entries({ accept: "session.accept", reject: "session.reject", cancelInvitation: "session.cancel", end: "session.end" }))
                        session[method] = (sdk, options) => mutate(command, { conversationId: cid, sessionId: sid }, sdk, options,
                            ["session.reject", "session.cancel"].includes(command) ? { session: { id: sid } } : { id: sid });
                    return session;
                },
            },
        });
        return { operationKinds: Object.fromEntries(commands.map(command => [command.replaceAll(".", "_"), { command }])),
            operations: { restore: (reference, kind) => { assert.equal(kind.command, reference.command); return operation(reference); } },
            capabilities: { refresh: async () => { permitted(); return { selectedTransport: "quic", features: new Set(["original-id-operations"]) }; } },
            conversations: { byId: conversation, create: (sdk, options) => mutate("conversation.create", {}, sdk, options, { id: "conversation-created" }) },
            raw: { validate: value => {
                if (value.type === "conversation.get") {
                    const limit = frozen.$defs.ConversationGet.properties.limit;
                    assert.ok(Number.isInteger(value.payload.limit) && value.payload.limit >= limit.minimum && value.payload.limit <= limit.maximum,
                        "Application topology requests must satisfy the frozen public wire limit");
                }
                return { value };
            }, request: async checked => {
                permitted(); const envelope = checked.value, frame = (type, payload) => ({ value: { type, cid: envelope.cid, payload } });
                if (envelope.type === "conversation.list") return [frame("ack", { details: { page: { conversations: [{ id: "conversation-created", closed: false }], next_cursor: null } } })];
                if (envelope.type === "message.history") return [frame("message.send", { msg_id: "message-1", from: "participant-owner", body: "visible text", content_type: "text/plain", body_encoding: "utf8" }), frame("ack", {})];
                assert.equal(envelope.type, "conversation.get");
                return [frame("conversation.snapshot", { revision: 1, cursor: "watermark", has_more: false, items: [
                    { kind: "session", data: { id: "session-1", state: "invited", media: ["data"] } },
                    { kind: "session_participant", data: { session_id: "session-1", participant_id: "participant-organizer", membership_epoch: service.membershipEpoch, state: "invited" } },
                    { kind: "stream", data: { connection_id: "connection-1", stream_id: "stream-1" } },
                ] })];
            } },
            close: async () => { current.closes++; current.closed = true; },
        };
    };
    return service;
}
async function setup(service = fixture()) {
    const directory = await mkdtemp(join(root, "gateway-")), journal = await AppIntents.open(directory);
    const gateway = new AppGateway({ journal, actors, createClient: service.createClient });
    return { directory, journal, gateway, service, close: async () => { await gateway.close(); journal.close(); } };
}

test("concurrent duplicate UI deliveries dispatch one prepared original and never expose backend config", async () => {
    const f = await setup();
    const action = { type: "conversation.create", recipients: ["organizer"] };
    const [first, duplicate] = await Promise.all([f.gateway.command("owner", "button-one", action), f.gateway.command("owner", "button-one", action)]);
    assert.deepEqual(first, duplicate); assert.equal(f.service.calls.length, 1);
    assert.deepEqual(f.service.calls[0].sdk, { participants: ["id-organizer"], policy: "persistent" });
    assert.equal(first.result.conversationId, "conversation-created");
    assert.deepEqual(f.gateway.contacts("owner"), [{ role: "organizer", label: "Organizer" }]);
    assert.ok(!JSON.stringify(f.gateway).includes("private-owner")); await f.close();
});

test("lost submitted reply recovers original server record after journal restart without another mutation", async () => {
    const f = await setup(); f.service.lost = true;
    const action = { type: "message.send", conversationId: "conversation-created", recipients: ["organizer"], text: "private app text" };
    const uncertain = await f.gateway.command("owner", "message-button", action);
    assert.equal(uncertain.operation.needsRecovery, true); assert.equal(uncertain.error.code, "offline-or-uncertain");
    assert.ok(!JSON.stringify(uncertain).includes("private transport credential"));
    const originalId = uncertain.operation.operationId; await f.close();
    const journal = await AppIntents.open(f.directory), gateway = new AppGateway({ journal, actors, createClient: f.service.createClient });
    const recovered = await gateway.recover("owner", originalId);
    assert.equal(recovered.operation.operationId, originalId); assert.equal(recovered.operation.state, "completed");
    assert.equal(f.service.calls.length, 1); assert.equal(journal.discovery()[0].result.messageId, f.service.calls[0].sdk.messageId);
    await gateway.close(); journal.close();
});

test("changed input under original UI key conflicts and another actor cannot recover its operation", async () => {
    const f = await setup(), action = { type: "conversation.create", recipients: ["organizer"] };
    const first = await f.gateway.command("owner", "stable-button", action);
    await assert.rejects(f.gateway.command("owner", "stable-button", { ...action, recipients: ["owner"] }), { code: "different-original-intent" });
    await assert.rejects(f.gateway.recover("organizer", first.operation.operationId), { code: "unknown-app-operation" });
    assert.equal(f.service.calls.length, 1); await f.close();
});

test("invitations and decisions resolve participant IDs and capture one topology revision through SDK handles", async () => {
    const f = await setup();
    const invited = await f.gateway.command("owner", "invite-button", { type: "session.invite", conversationId: "conversation-created", recipients: ["organizer"], medium: "text-chat" });
    assert.deepEqual(f.service.calls[0].sdk.to, ["participant-organizer"]);
    const sessionId = invited.result.sessionId;
    await f.gateway.command("organizer", "accept-button", { type: "session.accept", conversationId: "conversation-created", sessionId });
    assert.equal(f.service.calls[1].sdk.expectedTopologyRevision, 3);
    f.service.topology = 4;
    await f.gateway.command("organizer", "accept-button", { type: "session.accept", conversationId: "conversation-created", sessionId });
    assert.equal(f.service.calls.length, 2); await f.close();
});

test("timeline and invitation recovery use current authorized SDK snapshots rather than a cached role view", async () => {
    const f = await setup();
    const view = await f.gateway.view("organizer", "conversation-created");
    assert.equal(view.messages[0].text, "visible text"); assert.equal(view.sessions[0].appearance, "invited");
    f.service.allowed = false;
    await assert.rejects(f.gateway.view("organizer", "conversation-created"), { kind: "authentication" });
    assert.deepEqual(publicError(Error("private body must not be echoed")), { code: "offline-or-uncertain" });
    await f.close();
});

test("negotiated connection controls capture original stream revision and use the public update handle", async () => {
    const f = await setup();
    const result = await f.gateway.command("owner", "mute-button", { type: "connection.update", conversationId: "conversation-created",
        sessionId: "session-1", connectionId: "connection-1", action: "mute" });
    assert.equal(result.operation.state, "completed");
    assert.deepEqual(f.service.calls[0].sdk, { update: { action: "mute", streams: { first: "stream-1", rest: [] } }, revision: 8 });
    await f.close();
});

test("reconnect is explicit and closes the original client; malformed actions perform no SDK work", async () => {
    const f = await setup();
    assert.throws(() => f.gateway.command("owner", "bad", { type: "conversation.create", recipients: ["organizer"], credential: "private" }), { code: "invalid-app-request" });
    assert.equal(f.service.clients.length, 0);
    await f.gateway.capabilities("owner"); await f.gateway.capabilities("owner");
    assert.equal(f.service.clients.length, 1);
    await f.gateway.reconnect("owner"); assert.equal(f.service.clients[0].closes, 1); assert.equal(f.service.clients.length, 2);
    await f.close(); assert.equal(f.service.clients[1].closes, 1);
});

test("UI preparation survives a lost observation without submitting until an explicit second action", async () => {
    const f = await setup(), action = { type: "conversation.create", recipients: ["organizer"] };
    const prepared = await f.gateway.prepareIntent("owner", "two-step", action);
    assert.equal(prepared.operation.submission, "prepared"); assert.equal(f.service.calls.length, 0);
    assert.deepEqual(f.gateway.lookupKey("owner", "two-step"), prepared.operation);
    assert.equal(f.gateway.lookupKey("organizer", "two-step"), null);
    assert.equal((await f.gateway.prepareIntent("owner", "two-step", action)).operation.operationId, prepared.operation.operationId);
    assert.equal(f.service.calls.length, 0);
    const submitted = await f.gateway.recover("owner", prepared.operation.operationId);
    assert.equal(submitted.operation.state, "completed"); assert.equal(f.service.calls.length, 1);
    await f.close();
});
