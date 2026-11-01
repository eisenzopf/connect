// Communications use public installed NodeClient handles only. No service,
// Store, provider, internal helper protocol or model implementation is imported.
import { randomUUID } from "node:crypto";
import { isDeepStrictEqual, inspect } from "node:util";

const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const kinds = Object.freeze({ "conversation.create": "conversation_create", "message.send": "message_send",
    "session.invite": "session_invite", "session.accept": "session_accept", "session.reject": "session_reject",
    "session.cancel": "session_cancel", "session.end": "session_end", "connection.update": "connection_update",
    "connection.offer": "connection_offer", "connection.end": "connection_end",
    "stream.subscribe": "stream_subscribe", "stream.unsubscribe": "stream_unsubscribe" });
const fail = code => { const error = Error(code); error.code = code; throw error; };
const copy = value => structuredClone(value);
function closed(value, keys) {
    if (!value || Array.isArray(value) || Object.getPrototypeOf(value) !== Object.prototype ||
        Object.keys(value).sort().join() !== [...keys].sort().join()) fail("invalid-app-request");
}
function request(value) {
    if (!Object.hasOwn(kinds, value?.type)) fail("unsupported-app-action");
    switch (value.type) {
        case "conversation.create": closed(value, ["type", "recipients"]); break;
        case "message.send": closed(value, ["type", "conversationId", "recipients", "text"]);
            if (typeof value.text !== "string" || !value.text.trim() || Buffer.byteLength(value.text) > 8192) fail("invalid-app-request"); break;
        case "session.invite": closed(value, ["type", "conversationId", "recipients", "medium"]);
            if (!["text-chat", "voice", "voice+video"].includes(value.medium)) fail("unsupported-app-action"); break;
        case "connection.update": closed(value, ["type", "conversationId", "sessionId", "connectionId", "action"]);
            if (!["mute", "unmute", "hold", "resume"].includes(value.action)) fail("unsupported-app-action"); break;
        case "connection.offer": closed(value, ["type", "conversationId", "sessionId", "medium"]);
            if (!["voice", "voice+video"].includes(value.medium)) fail("unsupported-app-action"); break;
        case "connection.end": closed(value, ["type", "conversationId", "sessionId", "connectionId"]); break;
        case "stream.subscribe": case "stream.unsubscribe":
            closed(value, ["type", "conversationId", "sessionId", "connectionId", ...(value.type === "stream.subscribe" ? ["peer"] : [])]);
            if (value.type === "stream.subscribe" && !id(value.peer)) fail("invalid-app-request"); break;
        default: closed(value, ["type", "conversationId", "sessionId"]);
    }
    for (const key of ["conversationId", "sessionId", "connectionId"])
        if (key in value && !id(value[key])) fail("invalid-app-request");
    if ("recipients" in value && (!Array.isArray(value.recipients) || !value.recipients.length ||
        value.recipients.length > 4 || value.recipients.some(v => !id(v)) || new Set(value.recipients).size !== value.recipients.length))
        fail("invalid-app-request");
    return copy(value);
}
function savedReference(saved) {
    return saved.reference ?? { schema: "connect-operation-reference/1", operation_id: saved.operationId,
        command: saved.intent.command, scope: Object.fromEntries(Object.entries(saved.intent.scope)
            .map(([key, value]) => [key.replace(/[A-Z]/g, c => "_" + c.toLowerCase()), value])) };
}
export function publicError(error) {
    const local = new Set(["invalid-app-request", "unsupported-app-action", "different-original-intent", "unknown-app-actor",
        "unknown-app-operation", "app-request-capacity", "app-intent-capacity", "app-snapshot-capacity", "app-resync-required", "app-reconnecting"]);
    if (local.has(error?.code)) return { code: error.code };
    if (error?.kind === "authentication" || error?.problem?.category === "forbidden") return { code: "permission-denied" };
    if (error?.certainty === "refused") return { code: "action-refused" };
    if (["validation", "configuration", "unsupported", "conflict", "resync-required"].includes(error?.kind)) return { code: error.kind };
    return { code: "offline-or-uncertain" };
}

/** App-owned routing/presentation. Journal hints never grant membership or
 * capture authority; every read and action goes through the current SDK actor. */
export class AppGateway {
    #journal; #actors; #factory; #onError; #clients = new Map(); #flights = new Map(); #reconnects = new Map(); #closed = false; #closing;
    constructor({ journal, actors, createClient, onError = () => {} }) {
        if (!journal || typeof createClient !== "function" || !actors || Object.keys(actors).length > 5) fail("invalid-app-request");
        for (const [role, actor] of Object.entries(actors))
            if (!["owner", "assistant", "booker", "organizer", "observer"].includes(role) ||
                !id(actor.identityId) || typeof actor.config !== "string" || typeof actor.label !== "string") fail("invalid-app-request");
        if (typeof onError !== "function") fail("invalid-app-request");
        this.#journal = journal; this.#actors = copy(actors); this.#factory = createClient; this.#onError = onError;
    }
    [inspect.custom]() { return "AppGateway(private)"; }
    toJSON() { return { kind: "app-gateway" }; }
    #actor(role) {
        if (this.#closed || !Object.hasOwn(this.#actors, role)) fail("unknown-app-actor");
        return this.#actors[role];
    }
    #client(role) {
        const actor = this.#actor(role);
        if (this.#reconnects.has(role)) fail("app-reconnecting");
        if (!this.#clients.has(role)) this.#clients.set(role, this.#factory(actor));
        return this.#clients.get(role);
    }
    #options() { return { timeoutMs: 5000, signal: AbortSignal.timeout(5000) }; }
    contacts(role) {
        this.#actor(role);
        return Object.entries(this.#actors).filter(([name]) => name !== role)
            .map(([name, actor]) => ({ role: name, label: actor.label }));
    }
    progress(role) { this.#actor(role); return this.#journal.progress(role); }
    lookupKey(role, clientKey) {
        this.#actor(role); const saved = this.#journal.lookup(clientKey);
        if (!saved || saved.intent.actor !== role) return null;
        return this.progress(role).find(value => value.operationId === saved.operationId);
    }
    async capabilities(role) {
        const value = await this.#client(role).capabilities.refresh(this.#options());
        return { transport: value.selectedTransport, features: [...value.features] };
    }
    async #raw(role, type, payload, conversationId, options = this.#options()) {
        const client = this.#client(role);
        const envelope = { v: 2, id: "app_read_" + randomUUID(), ts: new Date().toISOString(), type, payload,
            ...(conversationId ? { cid: conversationId } : {}) };
        return (await client.raw.request(client.raw.validate(envelope), options)).map(item => item.value);
    }
    async conversations(role, cursor) {
        if (cursor !== undefined && (typeof cursor !== "string" || cursor.length > 2048)) fail("invalid-app-request");
        const frames = await this.#raw(role, "conversation.list", { limit: 20, ...(cursor ? { cursor } : {}) });
        const ack = frames.find(frame => frame.type === "ack"), page = ack?.payload?.details?.page;
        if (!page || !Array.isArray(page.conversations) || page.conversations.length > 20) fail("app-resync-required");
        return { conversations: page.conversations.map(value => {
            if (!id(value.id) || typeof value.closed !== "boolean") fail("app-resync-required");
            return { id: value.id, closed: value.closed };
        }), cursor: page.next_cursor ?? null };
    }
    async #topology(role, cid, options) {
        let page, cursor, revision, watermark; const items = [];
        // Let the server select a page within its configured limit. The wire
        // maximum (127) can exceed an installation's page size (100 by default).
        // Even a one-item page keeps the same total ceiling and shared deadline.
        for (let count = 0; count < 2048; count++) {
            const frames = await this.#raw(role, "conversation.get", page ? { page } : {}, cid, options);
            const snapshot = frames.find(frame => frame.type === "conversation.snapshot")?.payload;
            if (!snapshot || !Array.isArray(snapshot.items) || typeof snapshot.has_more !== "boolean") fail("app-resync-required");
            if (revision === undefined) { revision = snapshot.revision; watermark = snapshot.cursor; }
            else if (snapshot.revision !== revision || snapshot.cursor !== watermark) fail("app-resync-required");
            items.push(...snapshot.items);
            if (items.length > 2048) fail("app-snapshot-capacity");
            if (!snapshot.has_more) return items;
            if (snapshot.items.length === 0) fail("app-resync-required");
            cursor = snapshot.next_page;
            if (typeof cursor !== "string" || cursor === page) fail("app-resync-required");
            page = cursor;
        }
        fail("app-snapshot-capacity");
    }
    async view(role, cid, historyCursor) {
        if (historyCursor !== undefined && historyCursor !== null &&
            (typeof historyCursor !== "string" || historyCursor.length > 2048)) fail("invalid-app-request");
        const client = this.#client(role), options = this.#options();
        const conversation = await client.conversations.byId(cid).refresh(options);
        const items = await this.#topology(role, cid, options);
        const own = conversation.participants.find(p => p.identityId === this.#actor(role).identityId && p.active);
        if (!own) fail("app-resync-required");
        const history = await this.#raw(role, "message.history", { limit: 50, include_attachments: false,
            ...(historyCursor ? { cursor: historyCursor } : {}) }, cid, options);
        const label = identity => Object.values(this.#actors).find(actor => actor.identityId === identity)?.label ?? "Participant";
        const participants = conversation.participants.filter(p => p.active).map(p =>
            ({ id: p.id, label: label(p.identityId), role: Object.entries(this.#actors).find(([, a]) => a.identityId === p.identityId)?.[0] ?? null }));
        const messages = history.filter(frame => frame.type === "message.send").map(frame => {
            const message = frame.payload;
            const text = message.content_type === "text/plain" ? (message.body_encoding === "base64" ?
                new TextDecoder("utf-8", { fatal: true }).decode(Buffer.from(message.body, "base64")) : message.body) : "[Attachment or non-text message]";
            return { id: message.msg_id, from: participants.find(p => p.id === message.from)?.label ?? "Former participant", text };
        });
        const sessions = items.filter(item => item.kind === "session").map(item => item.data);
        const appearances = items.filter(item => item.kind === "session_participant" &&
            item.data.participant_id === own.id && item.data.membership_epoch === own.membershipEpoch).map(item => item.data);
        return { conversation: { id: cid, closed: conversation.closed }, participants, messages,
            historyCursor: history.find(frame => frame.type === "ack")?.payload?.next_cursor ?? null,
            sessions: sessions.map(session => ({ id: session.id, state: session.state, media: session.media,
                appearance: appearances.find(p => p.session_id === session.id)?.state ?? "unavailable" })),
            connections: items.filter(item => item.kind === "connection" && item.data.participant_id === own.id)
                .map(item => ({ id: item.data.connection_id, sessionId: item.data.session_id, state: item.data.state,
                    peers: participants.filter(peer => peer.id !== own.id && peer.role && items.some(v =>
                        v.kind === "session_participant" && v.data.session_id === item.data.session_id &&
                        v.data.participant_id === peer.id && v.data.state === "present" && v.data.membership_epoch ===
                            conversation.participants.find(p => p.id === peer.id)?.membershipEpoch) && items.some(v =>
                        v.kind === "connection" && v.data.session_id === item.data.session_id && v.data.participant_id === peer.id &&
                        ["ready", "active"].includes(v.data.state))).map(peer => ({ role: peer.role, label: peer.label })) })),
            sampledAt: Date.now() };
    }
    #recipients(role, names) {
        return names.map(name => { if (name === role) fail("invalid-app-request"); return this.#actor(name).identityId; });
    }
    async #prepare(role, action) {
        const client = this.#client(role), options = this.#options(), scope = {};
        if (action.type === "conversation.create") return { scope, sdk: {
            participants: this.#recipients(role, action.recipients), policy: "persistent" } };
        scope.conversationId = action.conversationId;
        const conversation = client.conversations.byId(action.conversationId);
        const current = await conversation.refresh(options);
        if (current.closed) fail("invalid-app-request");
        if (["message.send", "session.invite"].includes(action.type)) {
            const recipients = this.#recipients(role, action.recipients).map(identity => {
                const member = current.participants.find(p => p.identityId === identity && p.active);
                if (!member) fail("invalid-app-request"); return member.id;
            });
            if (action.type === "message.send") return { scope, sdk: { messageId: "msg_app_" + randomUUID(),
                to: { first: recipients[0], rest: recipients.slice(1) }, text: action.text } };
            const sessionId = "sid_app_" + randomUUID();
            return { scope: { ...scope, sessionId }, sdk: { sessionId, to: recipients, medium: action.medium,
                intent: "Reference app invitation", capabilities: action.medium === "text-chat" ?
                    { data: ["reliable-ordered"] } : { audio: ["opus"], ...(action.medium === "voice+video" ? { video: ["vp8"] } : {}) } } };
        }
        scope.sessionId = action.sessionId;
        const session = await conversation.sessions.byId(action.sessionId).refresh(options);
        if (["connection.offer", "connection.update", "connection.end", "stream.subscribe", "stream.unsubscribe"].includes(action.type)) {
            const own = current.participants.find(p => p.identityId === this.#actor(role).identityId && p.active);
            if (!own) fail("app-resync-required");
            const items = await this.#topology(role, action.conversationId, options);
            if (action.type === "connection.offer") {
                const media = action.medium === "voice" ? ["audio"] : ["audio", "video"];
                const present = items.some(v => v.kind === "session_participant" && v.data.session_id === action.sessionId &&
                    v.data.participant_id === own.id && v.data.membership_epoch === own.membershipEpoch && v.data.state === "present");
                if (session.state !== "accepted" || !present || !media.every(v => session.media.includes(v))) fail("invalid-app-request");
                if (items.some(v => v.kind === "connection" && v.data.session_id === action.sessionId &&
                    v.data.participant_id === own.id && v.data.state !== "retired")) fail("invalid-app-request");
                scope.connectionId = "conn_app_" + randomUUID();
                const streams = media.map(kind => ({ id: "stream_app_" + randomUUID(), kind, direction: "sendrecv",
                    codecs: { first: kind === "audio" ? "opus" : "vp8", rest: [] } }));
                return { scope, sdk: { connectionId: scope.connectionId, streams: { first: streams[0], rest: streams.slice(1) } } };
            }
            scope.connectionId = action.connectionId;
            if (!items.some(v => v.kind === "connection" && v.data.connection_id === action.connectionId &&
                v.data.session_id === action.sessionId && v.data.participant_id === own.id && ["ready", "active"].includes(v.data.state))) fail("invalid-app-request");
            const connection = await conversation.sessions.byId(action.sessionId).connections.byId(action.connectionId).refresh(options);
            if (action.type === "connection.end") return { scope, sdk: { reasonCode: 200, reason: "Reference app connection ended" } };
            if (connection.streamRevision === undefined) fail("app-resync-required");
            // Audio selection changes the mix routing on an existing duplex
            // binding. Unsubscribe names remote source Streams, not invented
            // local recvonly bindings. Resolve current visible source records
            // under this same Session; SDK/server admission still authorizes IO.
            const audioSources = participant => [...new Set(items.filter(v => v.kind === "stream" &&
                v.data.session_id === action.sessionId && v.data.medium === "audio" && v.data.state !== "ended" &&
                v.data.participant_id !== own.id && (!participant || v.data.participant_id === participant) &&
                items.some(source => source.kind === "connection" && source.data.connection_id === v.data.connection_id &&
                    source.data.session_id === action.sessionId && source.data.participant_id === v.data.participant_id &&
                    ["ready", "active"].includes(source.data.state))).map(v => v.data.stream_id))];
            if (action.type === "stream.unsubscribe") {
                const negotiation = await conversation.sessions.byId(action.sessionId).connections.byId(action.connectionId).media.refresh(options);
                const streams = audioSources();
                if (!streams.length) fail("app-resync-required");
                if (streams.length > 32) fail("app-snapshot-capacity");
                return { scope, sdk: { streams: { first: streams[0], rest: streams.slice(1) }, expectedStreamRevision: negotiation.streamRevision } };
            }
            if (action.type === "stream.subscribe") {
                const identity = this.#recipients(role, [action.peer])[0];
                const peer = current.participants.find(p => p.identityId === identity && p.active);
                if (!peer || !items.some(v => v.kind === "session_participant" && v.data.session_id === action.sessionId &&
                    v.data.participant_id === peer.id && v.data.membership_epoch === peer.membershipEpoch && v.data.state === "present")) fail("invalid-app-request");
                const streams = audioSources(peer.id);
                if (!streams.length) fail("app-resync-required");
                if (streams.length > 32) fail("app-snapshot-capacity");
                const selectors = streams.map(stream => ({ stream }));
                return { scope, sdk: { selectors: { first: selectors[0], rest: selectors.slice(1) }, expectedStreamRevision: connection.streamRevision } };
            }
            let streams = [], revision = connection.streamRevision;
            if (["mute", "unmute"].includes(action.action)) {
                // Snapshot Stream participant_id describes the containing
                // Connection; it does not distinguish its incoming bindings.
                const negotiation = await conversation.sessions.byId(action.sessionId).connections.byId(action.connectionId).media.refresh(options);
                streams = negotiation.streams.filter(v => v.medium === "audio" && ["sendonly", "sendrecv"].includes(v.direction)).map(v => v.id);
                revision = negotiation.streamRevision;
                if (!streams.length) fail("app-resync-required");
            }
            const update = action.action === "hold" ? { action: "hold", direction: "inactive" } :
                ["mute", "unmute"].includes(action.action) ? { action: action.action, streams: { first: streams[0], rest: streams.slice(1) } } : { action: "resume" };
            return { scope, sdk: { update, expectedRevision: revision } };
        }
        return { scope, sdk: action.type === "session.accept" ? { capabilities: session.capabilitiesOffer ?? {},
            expectedTopologyRevision: session.topologyRevision } : { reasonCode: action.type === "session.reject" ? 486 : 200,
            reason: "Reference app decision", expectedTopologyRevision: session.topologyRevision } };
    }
    #restore(client, saved) { return client.operations.restore(savedReference(saved), client.operationKinds[kinds[saved.intent.command]]); }
    async #submit(client, saved) {
        const { command, scope, input } = saved.intent, options = { ...this.#options(), operationId: saved.operationId }, sdk = input.sdk;
        if (command === "conversation.create") return client.conversations.create(sdk, options);
        const conversation = client.conversations.byId(scope.conversationId);
        if (command === "message.send") return conversation.send(sdk, options);
        if (command === "session.invite") return conversation.sessions.invite(sdk, options);
        const session = conversation.sessions.byId(scope.sessionId);
        if (command === "connection.offer") return session.connections.offer(sdk, options);
        if (command === "connection.end") return session.connections.byId(scope.connectionId).end(sdk, options);
        if (command === "stream.subscribe" || command === "stream.unsubscribe")
            return session.connections.byId(scope.connectionId).streams[command === "stream.subscribe" ? "subscribe" : "unsubscribe"](sdk, options);
        if (command === "connection.update") return session.connections.byId(scope.connectionId).update(sdk.update, sdk.expectedRevision, options);
        return session[({ "session.accept": "accept", "session.reject": "reject", "session.cancel": "cancelInvitation", "session.end": "end" })[command]](sdk, options);
    }
    async #observe(client, saved, operation = this.#restore(client, saved)) {
        const options = this.#options(), reference = operation.referenceJson();
        const snapshot = await operation.status(options);
        this.#journal.observe(saved.operationId, reference, snapshot);
        if (snapshot.state === "completed") {
            const completion = await operation.wait(options);
            this.#journal.observe(saved.operationId, reference, completion.snapshot);
            const value = completion.value, scope = saved.intent.scope;
            const result = saved.intent.command === "conversation.create" ? { conversationId: value.id } :
                saved.intent.command === "message.send" ? { conversationId: scope.conversationId, messageId: value.messageId } :
                saved.intent.command.startsWith("connection.") || saved.intent.command.startsWith("stream.") ?
                    { ...scope, connectionId: value.connection?.id ?? value.id ?? scope.connectionId } :
                { ...scope, sessionId: value.session?.id ?? value.id ?? scope.sessionId };
            this.#journal.rememberResult(saved.operationId, result);
        }
        const current = this.#journal.read(saved.operationId);
        return { operation: this.#journal.progress(saved.intent.actor).find(v => v.operationId === saved.operationId), result: current.result };
    }
    command(role, clientKey, input) {
        this.#actor(role); const action = request(input);
        if (!id(clientKey)) fail("invalid-app-request");
        const key = role + ":" + clientKey, existing = this.#flights.get(key);
        if (existing) { if (!isDeepStrictEqual(existing.action, action)) fail("different-original-intent"); return existing.work; }
        if (this.#flights.size >= 16) fail("app-request-capacity");
        const work = this.#execute(role, clientKey, action).finally(() => this.#flights.delete(key));
        this.#flights.set(key, { action, work }); return work;
    }
    /** The UI saves the returned operation ID before separately asking to
     * submit. This phase performs SDK reads/validation but no mutation. */
    prepareIntent(role, clientKey, input) {
        this.#actor(role); const action = request(input);
        if (!id(clientKey)) fail("invalid-app-request");
        const key = "prepare:" + role + ":" + clientKey, flight = this.#flights.get(key);
        if (flight) { if (!isDeepStrictEqual(flight.action, action)) fail("different-original-intent"); return flight.work; }
        if (this.#flights.size >= 16) fail("app-request-capacity");
        const work = (async () => {
            let saved = this.#journal.lookup(clientKey);
            if (saved && (saved.intent.actor !== role || !isDeepStrictEqual(saved.intent.input.request, action))) fail("different-original-intent");
            if (!saved) {
                const prepared = await this.#prepare(role, action);
                saved = this.#journal.prepare(clientKey, { actor: role, command: action.type,
                    scope: prepared.scope, input: { request: action, sdk: prepared.sdk } });
            }
            this.#restore(this.#client(role), saved);
            return { operation: this.progress(role).find(value => value.operationId === saved.operationId) };
        })().finally(() => this.#flights.delete(key));
        this.#flights.set(key, { action, work }); return work;
    }
    async #execute(role, clientKey, action) {
        let saved = this.#journal.lookup(clientKey), submitted = false;
        if (saved && (saved.intent.actor !== role || !isDeepStrictEqual(saved.intent.input.request, action))) fail("different-original-intent");
        const client = this.#client(role);
        if (!saved) {
            const prepared = await this.#prepare(role, action);
            saved = this.#journal.prepare(clientKey, { actor: role, command: action.type,
                scope: prepared.scope, input: { request: action, sdk: prepared.sdk } });
        }
        try {
            this.#restore(client, saved); // Public SDK validates the original reference before dispatch.
            if (saved.phase === "refused") return { operation: this.#journal.progress(role).find(v => v.operationId === saved.operationId), result: null };
            if (saved.phase !== "prepared") return await this.#observe(client, saved);
            this.#journal.attempt(saved.operationId);
            const operation = await this.#submit(client, saved); submitted = true;
            return await this.#observe(client, saved, operation);
        } catch (error) {
            this.#onError(error);
            const current = this.#journal.read(saved.operationId);
            if (current.phase === "prepared" && !submitted && error?.certainty === "not-submitted") this.#journal.notSubmitted(saved.operationId);
            if (current.phase === "attempted") {
                if (!submitted && ["not-submitted", "refused"].includes(error?.certainty)) this.#journal.refused(saved.operationId);
                else this.#journal.uncertain(saved.operationId);
            }
            return { error: publicError(error), operation: this.#journal.progress(role).find(v => v.operationId === saved.operationId), result: null };
        }
    }
    async recover(role, operationId) {
        const saved = this.#journal.read(operationId);
        if (saved.intent.actor !== role) fail("unknown-app-operation");
        return this.command(role, saved.clientKey, saved.intent.input.request);
    }
    reconnect(role) {
        this.#actor(role);
        const existing = this.#reconnects.get(role); if (existing) return existing;
        const current = this.#clients.get(role);
        const work = (async () => {
            if (current) await current.close({ timeoutMs: 5000 });
            this.#clients.delete(role);
            const actor = this.#actor(role); // Closing the gateway fences replacement after the old owner joins.
            const client = this.#factory(actor); this.#clients.set(role, client);
            const value = await client.capabilities.refresh(this.#options());
            return { transport: value.selectedTransport, features: [...value.features] };
        })().finally(() => this.#reconnects.delete(role));
        this.#reconnects.set(role, work); return work;
    }
    close() {
        if (this.#closing) return this.#closing;
        this.#closed = true;
        const flights = [...this.#flights.values()].map(value => value.work).concat([...this.#reconnects.values()]);
        this.#closing = (async () => {
            const settled = await Promise.allSettled([...this.#clients.values()].map(client => client.close({ timeoutMs: 5000 })));
            await Promise.allSettled(flights);
            this.#clients.clear();
            if (settled.some(value => value.status === "rejected")) fail("app-cleanup-incomplete");
        })();
        return this.#closing;
    }
}
