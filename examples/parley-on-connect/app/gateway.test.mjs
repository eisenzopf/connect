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
            refresh: async () => { permitted(); service.conversationRefreshReads = (service.conversationRefreshReads ?? 0) + 1;
                service.readEntered?.(); if (service.readBarrier) await service.readBarrier; return { id: cid, closed: false, participants: Object.entries(actors).map(([role, value]) =>
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
                        refresh: async () => { permitted(); service.signedConnectionReads ??= []; service.signedConnectionReads.push(current);
                            if (service.failSupervision && current !== service.clients[0]) { current.fenced = true;
                                throw { kind: "deadline", certainty: "read-only" }; }
                            return { id: connectionId, generation: service.connectionGeneration ?? 1, streamRevision: service.streamRevision, state: "ready" }; },
                        update: (update, revision, options) => mutate("connection.update", { conversationId: cid, sessionId: sid, connectionId },
                            { update, revision }, options, { connection: { id: connectionId } }),
                        end: (sdk, options) => mutate("connection.end", { conversationId: cid, sessionId: sid, connectionId }, sdk, options, { id: connectionId }),
                        media: { refresh: async () => { permitted(); service.mediaDescribeReads = (service.mediaDescribeReads ?? 0) + 1; return { binding: { conversationId: cid, sessionId: sid, connectionId }, sourceClock: true, streamRevision: service.streamRevision, streams: [
                            { id: "stream-1", medium: "audio", direction: "sendrecv" },
                            ...(service.projectForeignStream ? [{ id: "foreign-stream", medium: "audio", direction: "recvonly" }] : []),
                            ...(service.videoReceive ? [{ id: "peer-video", medium: "video", direction: "recvonly" },
                                { id: "own-camera", medium: "video", direction: "sendrecv" }] : []),
                        ] }; }, streamById: streamId => ({
                            sendRtp: async (packet, options) => { permitted(); service.packetCalls ??= []; service.packetCalls.push(packet);
                                return { boundary: "local-media-queue", datagramSequence: packet.sequence }; },
                            receivePacket: async () => { permitted(); return { kind: "gap", droppedPackets: 0n }; },
                        }) },
                        streams: Object.fromEntries(["subscribe", "unsubscribe"].map(action => [action, (sdk, options) => {
                            // The installed public methods intentionally have
                            // different shapes: selectors versus Stream IDs.
                            assert.deepEqual(Object.keys(sdk).sort(), ["expectedStreamRevision", action === "subscribe" ? "selectors" : "streams"].sort());
                            if (action === "unsubscribe") assert.ok(["peer-audio", "peer-video"].includes(sdk.streams.first));
                            service.receiving = action === "subscribe";
                            return mutate("stream." + action, { conversationId: cid, sessionId: sid, connectionId }, sdk, options, { connection: { id: connectionId } });
                        }])),
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
            raw: { read: async envelope => {
                if (envelope.type === "conversation.get" && envelope.payload.limit !== undefined) {
                    const limit = frozen.$defs.ConversationGet.properties.limit;
                    assert.ok(Number.isInteger(envelope.payload.limit) && envelope.payload.limit >= limit.minimum && envelope.payload.limit <= limit.maximum,
                        "Application topology requests must satisfy the frozen public wire limit");
                }
                permitted(); const frame = (type, payload) => ({ value: { type, cid: envelope.cid, payload } });
                if (envelope.type === "conversation.list") return [frame("ack", { details: { page: { conversations: [{ id: "conversation-created", closed: false }], next_cursor: null } } })];
                if (envelope.type === "message.history") return [frame("message.send", { msg_id: "message-1", from: "participant-owner", body: "visible text", content_type: "text/plain", body_encoding: "utf8" }), frame("ack", {})];
                assert.equal(envelope.type, "conversation.get");
                service.snapshotReaders ??= []; service.snapshotReaders.push(current);
                assert.ok((envelope.payload.limit ?? service.pageSize) <= service.pageSize, "Configured server page limits also apply");
                const items = [
                    { kind: "conversation", data: { conversation_id: envelope.cid, closed: false } },
                    ...Object.entries(actors).map(([role, actor]) => ({ kind: "participant", data: {
                        participant_id: "participant-" + role, identity_id: actor.identityId, conversation_id: envelope.cid,
                        active: true, membership_epoch: service.membershipEpoch } })),
                    { kind: "session", data: { id: "session-1", state: service.sessionState, media: service.media } },
                    { kind: "session_participant", data: { session_id: "session-1", participant_id: "participant-organizer", membership_epoch: service.membershipEpoch, state: service.peerState } },
                    { kind: "session_participant", data: { session_id: "session-1", participant_id: "participant-owner", membership_epoch: service.membershipEpoch, state: "present" } },
                    ...(service.connectionPresent ? [{ kind: "connection", data: { connection_id: service.connectionId, session_id: "session-1", participant_id: "participant-owner", membership_epoch: service.connectionEpoch ?? service.membershipEpoch,
                        generation: service.connectionGeneration ?? 1, state: "ready" } }] : []),
                    ...(service.peerConnectionPresent ? [
                        { kind: "connection", data: { connection_id: "peer-connection", session_id: "session-1", participant_id: "participant-organizer", state: "ready" } },
                        { kind: "stream", data: { connection_id: "peer-connection", stream_id: "peer-audio", participant_id: "participant-organizer", medium: "audio", session_id: "session-1", state: "active" } },
                    ] : []),
                    { kind: "stream", data: { connection_id: service.connectionId, stream_id: "stream-1", participant_id: "participant-owner", medium: "audio", session_id: "session-1" } },
                    ...(service.projectForeignStream ? [{ kind: "stream", data: { connection_id: service.connectionId,
                        stream_id: "foreign-stream", participant_id: "participant-owner", medium: "audio", session_id: "session-1" } }] : []),
                ];
                if (service.duplicateSnapshotParticipant) items.push(structuredClone(items.find(v => v.kind === "participant" && v.data.identity_id === actors.owner.identityId)));
                const offset = envelope.payload.page ? Number(envelope.payload.page.replace("page_", "")) : 0;
                assert.ok(Number.isInteger(offset) && offset >= 0 && offset < items.length);
                service.topologyPages++;
                const end = Math.min(offset + service.pageSize, items.length);
                return [frame("conversation.snapshot", { revision: service.changeSnapshotRevision && offset > 0 ? 2 : 1, cursor: "watermark", has_more: end < items.length,
                    ...(end < items.length ? { next_page: "page_" + end } : {}), items: items.slice(offset, end) })];
            } },
            close: async () => { current.closes++; service.closeStarted?.(); if (service.closeBarrier) await service.closeBarrier; current.closed = true; },
        };
    };
    return service;
}
async function setup(service = fixture()) {
    const directory = await mkdtemp(join(root, "gateway-")), journal = await AppIntents.open(directory);
    const gateway = new AppGateway({ journal, actors, createClient: service.createClient, createRtpPacket: (sequence, bytes) => ({ sequence, bytes: Buffer.from(bytes) }) });
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
    assert.equal(service.topologyPages, 8); assert.equal(view.sessions.length, 1);
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
    const journal = await AppIntents.open(f.directory), gateway = new AppGateway({ journal, actors, createClient: service.createClient, createRtpPacket: (sequence, bytes) => ({ sequence, bytes: Buffer.from(bytes) }) });
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
    const f = await setup(), action = { type: "stream.subscribe", conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1", peer: "organizer", medium: "audio" };
    await assert.rejects(f.gateway.prepareIntent("owner", "not-present", action), { code: "invalid-app-request" });
    assert.equal(f.service.calls.length, 0);
    f.service.peerState = "present";
    await assert.rejects(f.gateway.prepareIntent("owner", "no-source", action), { code: "app-resync-required" });
    assert.equal(f.service.calls.length, 0);
    f.service.peerConnectionPresent = true;
    const selected = await f.gateway.command("owner", "listen", action);
    assert.deepEqual(f.service.calls[0].sdk, { selectors: { first: { stream: "peer-audio" }, rest: [] }, expectedStreamRevision: 8 });
    assert.deepEqual(selected.result, { conversationId: action.conversationId, sessionId: action.sessionId, connectionId: action.connectionId });
    f.service.streamRevision = 17;
    const { peer, ...connectionScope } = action;
    await f.gateway.command("owner", "stop-listening", { ...connectionScope, type: "stream.unsubscribe" });
    assert.equal(f.service.calls[1].sdk.expectedStreamRevision, 17);
    assert.deepEqual(f.service.calls[1].sdk.streams, { first: "peer-audio", rest: [] });
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
    const closeStarted = new Promise(done => { f.service.closeStarted = done; });
    const first = f.gateway.reconnect("owner"), duplicate = f.gateway.reconnect("owner");
    assert.equal(first, duplicate); await closeStarted; assert.equal(f.service.clients[0].closes, 1);
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


test("media bridge opens only the current own present Session and fences SDK IO on authority loss and reconnect", async t => {
    const f = await setup(); t.after(f.close); const owner = {};
    const binding = { conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
    await assert.rejects(f.gateway.mediaOpen("owner", owner, { ...binding, connectionId: "foreign" }), { code: "app-resync-required" });
    f.service.sessionState = "ended";
    await assert.rejects(f.gateway.mediaOpen("owner", owner, binding), { code: "app-resync-required" });
    f.service.sessionState = "accepted";
    const lease = await f.gateway.mediaOpen("owner", owner, binding);
    const send = { leaseId: lease.leaseId, streamId: "stream-1", sequence: 1,
        packet: { kind: "rtp", base64: Buffer.from([128,111,0,1,0,0,3,192,0,0,0,7,1]).toString("base64") } };
    assert.equal((await f.gateway.mediaSend("owner", owner, send)).boundary, "local-media-queue");
    f.service.allowed = false;
    await assert.rejects(f.gateway.mediaSend("owner", owner, send), { kind: "authentication" });
    assert.equal(f.service.packetCalls.length, 1);
    f.service.allowed = true;
    await f.gateway.reconnect("owner");
    await assert.rejects(f.gateway.mediaSend("owner", owner, send), { code: "app-media-stopped" });
    assert.equal(f.service.clients[0].closed, true);
    const replacement = await f.gateway.mediaOpen("owner", owner, binding);
    assert.notEqual(replacement.leaseId, lease.leaseId);
});


test("media opening uses current coherent membership/physical facts and refuses epoch or duplicate ambiguity", async t => {
    const binding = { conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
    for (const patch of [{ connectionEpoch: 0 }, { connectionGeneration: 0 }, { duplicateSnapshotParticipant: true }]) {
        const service = fixture(); Object.assign(service, patch); const f = await setup(service); t.after(f.close);
        await assert.rejects(f.gateway.mediaOpen("owner", {}, binding), { code: "app-resync-required" });
        assert.equal(service.mediaDescribeReads ?? 0, 0);
        assert.ok(service.clients.slice(1).every(v => v.closed && v.closes === 1));
    }
    const service = fixture(); service.pageSize = 1; const f = await setup(service); t.after(f.close);
    const result = await f.gateway.mediaOpen("owner", {}, binding);
    assert.equal(result.generation, 1); assert.equal(service.mediaDescribeReads, 1);
    assert.ok(service.snapshotReaders.every(v => v === service.clients[1]));
    assert.equal(service.conversationRefreshReads ?? 0, 0);
    assert.equal(service.topologyPages, 8);
});

test("media opening refuses a changed snapshot revision across pages before any physical describe", async t => {
    const service = fixture(); service.pageSize = 1; service.changeSnapshotRevision = true;
    const f = await setup(service); t.after(f.close);
    await assert.rejects(f.gateway.mediaOpen("owner", {}, {
        conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" }), { code: "app-resync-required" });
    assert.equal(service.mediaDescribeReads ?? 0, 0);
});

test("media supervision owns an isolated read-only client and joins it without retiring the original control owner", async t => {
    const f = await setup(); t.after(() => f.close());
    const owner = {}, binding = { conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
    const lease = await f.gateway.mediaOpen("owner", owner, binding);
    assert.equal(f.service.clients.length, 2);
    const [original, supervisor] = f.service.clients;
    f.service.failSupervision = true;
    await new Promise(done => setTimeout(done, 550));
    assert.deepEqual(f.service.signedConnectionReads, [supervisor]);
    assert.equal(supervisor.fenced, true);
    assert.equal(supervisor.closed, true);
    assert.equal(supervisor.closes, 1);
    assert.equal(original.closed, false);
    assert.equal(original.fenced, undefined);
    await assert.rejects(f.gateway.mediaReceive("owner", owner, { leaseId: lease.leaseId, streamId: "stream-1" }),
        { code: "app-media-stopped" });
    assert.equal((await f.gateway.capabilities("owner")).transport, "quic");
    assert.equal(f.service.clients.length, 2);
    await f.gateway.reconnect("owner");
    assert.equal(original.closed, true);
    assert.equal(supervisor.closes, 1);
});

test("video selection uses the current peer selector and unsubscribes only negotiated incoming video with original recovery", async t => {
    const f = await setup(); t.after(() => f.close());
    const action = { type: "stream.subscribe", conversationId: "conversation-created", sessionId: "session-1",
        connectionId: "connection-1", peer: "organizer", medium: "video" };
    f.service.media = ["audio", "video"]; f.service.peerState = "present";
    await assert.rejects(f.gateway.prepareIntent("owner", "no-live-peer", action), { code: "invalid-app-request" });
    f.service.peerConnectionPresent = true;
    const result = await f.gateway.command("owner", "watch", action);
    assert.deepEqual(f.service.calls[0].sdk, { selectors: { first: { participant: "participant-organizer",
        kinds: { first: "video", rest: [] } }, rest: [] }, expectedStreamRevision: 8 });
    await f.gateway.recover("owner", result.operation.operationId);
    assert.equal(f.service.calls.length, 1);
    const { peer, ...scope } = action;
    await assert.rejects(f.gateway.prepareIntent("owner", "no-receiver", { ...scope, type: "stream.unsubscribe" }),
        { code: "app-resync-required" });
    f.service.videoReceive = true; f.service.streamRevision = 19;
    const ended = await f.gateway.command("owner", "stop-video", { ...scope, type: "stream.unsubscribe" });
    assert.deepEqual(f.service.calls[1].sdk, { streams: { first: "peer-video", rest: [] }, expectedStreamRevision: 19 });
    assert.deepEqual(ended.result, result.result);
    assert.equal(f.service.calls.length, 2);
    assert.throws(() => f.gateway.prepareIntent("owner", "changed-medium", { ...action, medium: "screen" }),
        { code: "unsupported-app-action" });
});

test("concurrent identical actor reads share only their active SDK work and never cache authorization or cross actors", async t => {
    const f = await setup(); t.after(() => f.close());
    const cid = "conversation-created";
    const [one, duplicate, other] = await Promise.all([
        f.gateway.view("owner", cid), f.gateway.view("owner", cid), f.gateway.view("organizer", cid),
    ]);
    assert.deepEqual(one, duplicate);
    assert.equal(one.connections.length, 1); assert.equal(other.connections.length, 0);
    assert.equal(f.service.conversationRefreshReads, 2);
    assert.equal(f.service.topologyPages, 2);
    await f.gateway.view("owner", cid);
    assert.equal(f.service.conversationRefreshReads, 3); // Completed data is not retained.
    f.service.allowed = false;
    await assert.rejects(f.gateway.view("owner", cid), { kind: "authentication" });
    await assert.rejects(f.gateway.view("organizer", cid), { kind: "authentication" });
});

test("read sharing fences late retired-owner results and shutdown joins the original read", async t => {
    const f = await setup(); let release = () => {};
    t.after(async () => { release(); await f.close(); });
    const block = () => {
        let entered;
        const started = new Promise(resolve => { entered = resolve; });
        f.service.readEntered = entered;
        f.service.readBarrier = new Promise(resolve => { release = resolve; });
        return started;
    };
    const cid = "conversation-created", entered = block();
    const read = f.gateway.view("owner", cid);
    const refused = assert.rejects(read, { code: "app-resync-required" });
    await entered; await f.gateway.reconnect("owner");
    release(); await refused;
    f.service.readBarrier = null; f.service.readEntered = null;
    assert.equal((await f.gateway.view("owner", cid)).conversation.id, cid);
    const enteredAgain = block(), late = f.gateway.view("owner", cid);
    const stopped = assert.rejects(late, { code: "app-resync-required" });
    await enteredAgain;
    let joined = false; const closing = f.gateway.close().then(() => { joined = true; });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(joined, false);
    release(); await stopped; await closing;
    assert.equal(joined, true);
});

test("media admission and new Connection preparation accept current ready/active Sessions and refuse terminal or unconsented states", async t => {
    for (const state of ["accepted", "ready", "active"]) {
        const f = await setup(); t.after(() => f.close()); f.service.sessionState = state;
        const owner = {}, binding = { conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
        const lease = await f.gateway.mediaOpen("owner", owner, binding);
        assert.equal((await f.gateway.mediaReceive("owner", owner, { leaseId: lease.leaseId, streamId: "stream-1" })).kind, "gap");
        await f.gateway.mediaRetire("owner", owner, { leaseId: lease.leaseId });
        f.service.connectionPresent = false;
        const prepared = await f.gateway.prepareIntent("owner", "join-" + state,
            { type: "connection.offer", conversationId: binding.conversationId, sessionId: binding.sessionId, medium: "voice" });
        assert.equal(prepared.operation.command, "connection.offer");
        assert.equal(f.service.calls.length, 0);
    }
    for (const state of ["inviting", "ending", "ended", "failed"]) {
        const f = await setup(); t.after(() => f.close()); f.service.sessionState = state;
        const binding = { conversationId: "conversation-created", sessionId: "session-1", connectionId: "connection-1" };
        await assert.rejects(f.gateway.mediaOpen("owner", {}, binding), { code: "app-resync-required" });
        assert.equal(f.service.mediaDescribeReads ?? 0, 0);
        f.service.connectionPresent = false;
        await assert.rejects(f.gateway.prepareIntent("owner", "refuse-" + state,
            { type: "connection.offer", conversationId: binding.conversationId, sessionId: binding.sessionId, medium: "voice" }),
            { code: "invalid-app-request" });
    }
});
