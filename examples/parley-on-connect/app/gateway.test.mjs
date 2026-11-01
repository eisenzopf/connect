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
const commands = ["conversation.create", "message.send", "session.invite", "session.accept", "session.reject", "session.cancel", "session.end", "connection.update",
    "connection.offer", "connection.end", "stream.subscribe", "stream.unsubscribe"];
function fixture() {
    const service = { records: new Map(), calls: [], reads: [], clients: [], lost: false, allowed: true,
        membershipEpoch: 1, topology: 3, streamRevision: 8, pageSize: 100, topologyPages: 0,
        connectionPresent: true, connectionId: "connection-1", sessionState: "accepted", media: ["audio"], peerState: "invited" };
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
                        state: service.sessionState, media: service.media, capabilitiesOffer: { data: ["reliable-ordered"] } }), connections: {
                        offer: (sdk, options) => {
                            service.connectionPresent = true; service.connectionId = sdk.connectionId;
                            return mutate("connection.offer", { conversationId: cid, sessionId: sid, connectionId: sdk.connectionId }, sdk, options, { id: sdk.connectionId });
                        }, byId: connectionId => ({
                        refresh: async () => ({ id: connectionId, streamRevision: service.streamRevision, state: "ready" }),
                        update: (update, revision, options) => mutate("connection.update", { conversationId: cid, sessionId: sid, connectionId },
                            { update, revision }, options, { connection: { id: connectionId } }),
                        end: (sdk, options) => mutate("connection.end", { conversationId: cid, sessionId: sid, connectionId }, sdk, options, { id: connectionId }),
                        streams: Object.fromEntries(["subscribe", "unsubscribe"].map(action => [action, (sdk, options) =>
                            mutate("stream." + action, { conversationId: cid, sessionId: sid, connectionId }, sdk, options, { connection: { id: connectionId } })])),
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
                if (value.type === "conversation.get" && value.payload.limit !== undefined) {
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
                assert.ok((envelope.payload.limit ?? service.pageSize) <= service.pageSize, "Configured server page limits also apply");
                const items = [
                    { kind: "session", data: { id: "session-1", state: service.sessionState, media: service.media } },
                    { kind: "session_participant", data: { session_id: "session-1", participant_id: "participant-organizer", membership_epoch: service.membershipEpoch, state: service.peerState } },
                    { kind: "session_participant", data: { session_id: "session-1", participant_id: "participant-owner", membership_epoch: service.membershipEpoch, state: "present" } },
                    ...(service.connectionPresent ? [{ kind: "connection", data: { connection_id: service.connectionId, session_id: "session-1", participant_id: "participant-owner", state: "ready" } }] : []),
                    { kind: "stream", data: { connection_id: service.connectionId, stream_id: "stream-1", participant_id: "participant-owner", medium: "audio", session_id: "session-1" } },
                    ...(service.projectForeignStream ? [{ kind: "stream", data: { connection_id: service.connectionId,
                        stream_id: "foreign-stream", participant_id: "participant-organizer", medium: "audio", session_id: "session-1" } }] : []),
                ];
                const offset = envelope.payload.page ? Number(envelope.payload.page.replace("page_", "")) : 0;
                assert.ok(Number.isInteger(offset) && offset >= 0 && offset < items.length);
                service.topologyPages++;
                const end = Math.min(offset + service.pageSize, items.length);
                return [frame("conversation.snapshot", { revision: 1, cursor: "watermark", has_more: end < items.length,
                    ...(end < items.length ? { next_page: "page_" + end } : {}), items: items.slice(offset, end) })];
            } },
            close: async () => { current.closes++; if (service.closeBarrier) await service.closeBarrier; current.closed = true; },
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

test("topology follows the server's small configured pages and preserves one coherent invitation view", async () => {
    const service = fixture(); service.pageSize = 1;
    const f = await setup(service), view = await f.gateway.view("organizer", "conversation-created");
    assert.equal(service.topologyPages, 5); assert.equal(view.sessions.length, 1);
    assert.equal(view.sessions[0].appearance, "invited"); assert.equal(view.messages[0].text, "visible text");
    await f.close();
});

test("negotiated connection controls capture original stream revision and use the public update handle", async () => {
    const f = await setup();
    f.service.projectForeignStream = true;
    const result = await f.gateway.command("owner", "mute-button", { type: "connection.update", conversationId: "conversation-created",
        sessionId: "session-1", connectionId: "connection-1", action: "mute" });
    assert.equal(result.operation.state, "completed");
    assert.deepEqual(f.service.calls[0].sdk, { update: { action: "mute", streams: { first: "stream-1", rest: [] } }, revision: 8 });
    await f.close();
});

test("media offer saves original Connection and Stream IDs before submission and recovers a lost reply without another offer", async () => {
    const service = fixture(); service.connectionPresent = false; service.media = ["audio", "video"];
    const f = await setup(service), action = { type: "connection.offer", conversationId: "conversation-created", sessionId: "session-1", medium: "voice+video" };
    const prepared = await f.gateway.prepareIntent("owner", "join-media", action), saved = f.journal.lookup("join-media");
    assert.equal(service.calls.length, 0);
    assert.match(saved.intent.scope.connectionId, /^conn_app_/);
    assert.equal(saved.intent.input.sdk.connectionId, saved.intent.scope.connectionId);
    const streams = [saved.intent.input.sdk.streams.first, ...saved.intent.input.sdk.streams.rest];
    assert.deepEqual(streams.map(v => [v.kind, v.direction, v.codecs.first]), [["audio", "sendrecv", "opus"], ["video", "sendrecv", "vp8"]]);
    assert.equal(new Set(streams.map(v => v.id)).size, 2);
    service.lost = true;
    const uncertain = await f.gateway.command("owner", "join-media", action);
    assert.equal(uncertain.operation.operationId, prepared.operation.operationId); assert.equal(uncertain.operation.needsRecovery, true);
    await f.close();
    const journal = await AppIntents.open(f.directory), gateway = new AppGateway({ journal, actors, createClient: service.createClient });
    const recovered = await gateway.recover("owner", prepared.operation.operationId);
    assert.deepEqual(recovered.result, { conversationId: action.conversationId, sessionId: action.sessionId, connectionId: saved.intent.scope.connectionId });
    assert.equal(service.calls.length, 1); assert.deepEqual(service.calls[0].sdk, saved.intent.input.sdk);
    await gateway.close(); journal.close();
});

test("media offers refuse unavailable Session media, unaccepted Sessions and existing original Connections before mutation", async () => {
    for (const change of [service => { service.media = ["data"]; service.connectionPresent = false; },
        service => { service.sessionState = "invited"; service.connectionPresent = false; }, service => {}]) {
        const service = fixture(); change(service); const f = await setup(service);
        await assert.rejects(f.gateway.prepareIntent("owner", "unavailable-media", {
            type: "connection.offer", conversationId: "conversation-created", sessionId: "session-1", medium: "voice",
        }), { code: "invalid-app-request" });
        assert.equal(service.calls.length, 0); assert.equal(f.journal.lookup("unavailable-media"), null);
        await f.close();
    }
});

test("Connection end retains its original Session scope and refuses another participant's Connection", async () => {
    const f = await setup(), action = { type: "connection.end", conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
    await assert.rejects(f.gateway.prepareIntent("organizer", "foreign-end", action), { code: "invalid-app-request" });
    assert.equal(f.service.calls.length, 0);
    const result = await f.gateway.command("owner", "original-end", action);
    assert.deepEqual(result.result, { conversationId: action.conversationId, sessionId: action.sessionId, connectionId: action.connectionId });
    assert.equal(f.service.calls[0].command, "connection.end");
    assert.deepEqual(f.service.calls[0].sdk, { reasonCode: 200, reason: "Reference app connection ended" });
    assert.deepEqual(await f.gateway.recover("owner", result.operation.operationId), result);
    assert.equal(f.service.calls.length, 1); await f.close();
});

test("Stream selection uses current present membership and the negotiated revision with original result scope", async () => {
    const f = await setup(), action = { type: "stream.subscribe", conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1", peer: "organizer" };
    await assert.rejects(f.gateway.prepareIntent("owner", "not-present", action), { code: "invalid-app-request" });
    assert.equal(f.service.calls.length, 0);
    f.service.peerState = "present";
    const selected = await f.gateway.command("owner", "listen", action);
    assert.deepEqual(f.service.calls[0].sdk, { selectors: { first: { participant: "participant-organizer" }, rest: [] }, expectedStreamRevision: 8 });
    assert.deepEqual(selected.result, { conversationId: action.conversationId, sessionId: action.sessionId, connectionId: action.connectionId });
    f.service.streamRevision = 17;
    await f.gateway.command("owner", "stop-listening", { ...action, type: "stream.unsubscribe" });
    assert.equal(f.service.calls[1].sdk.expectedStreamRevision, 17);
    assert.equal(f.service.calls[1].command, "stream.unsubscribe");
    f.service.allowed = false;
    await assert.rejects(f.gateway.prepareIntent("owner", "revoked-listen", action), { kind: "authentication" });
    assert.equal(f.service.calls.length, 2); await f.close();
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

test("reconnect joins one original owner and refuses concurrent work until its replacement is ready", async () => {
    const f = await setup(); await f.gateway.capabilities("owner");
    let release; f.service.closeBarrier = new Promise(done => { release = done; });
    const first = f.gateway.reconnect("owner"), duplicate = f.gateway.reconnect("owner");
    assert.equal(first, duplicate); assert.equal(f.service.clients[0].closes, 1);
    await assert.rejects(f.gateway.capabilities("owner"), { code: "app-reconnecting" });
    await assert.rejects(f.gateway.command("owner", "during-reconnect", { type: "conversation.create", recipients: ["organizer"] }), { code: "app-reconnecting" });
    assert.equal(f.service.clients.length, 1); assert.equal(f.service.calls.length, 0);
    release(); f.service.closeBarrier = null; await first;
    assert.equal(f.service.clients[0].closed, true); assert.equal(f.service.clients.length, 2);
    assert.equal((await f.gateway.capabilities("owner")).transport, "quic");
    await f.close(); assert.equal(f.service.clients[1].closes, 1);
});

test("shutdown during reconnect joins the old owner without creating a replacement client", async () => {
    const f = await setup(); await f.gateway.capabilities("owner");
    let release; f.service.closeBarrier = new Promise(done => { release = done; });
    const reconnect = f.gateway.reconnect("owner");
    const observed = assert.rejects(reconnect, { code: "unknown-app-actor" });
    const closing = f.gateway.close(); release(); f.service.closeBarrier = null;
    await observed; await closing;
    assert.equal(f.service.clients.length, 1); assert.equal(f.service.clients[0].closed, true);
    f.journal.close();
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
