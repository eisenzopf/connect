import { BrowserTransportClient, } from "./browser_transport_client.js";
import { CanonicalError, } from "./canonical_error.js";
import { ConnectError } from "./errors.js";
import { canonicalJson, parseCanonicalJson, jsonObject, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export class BrowserSdkError extends CanonicalError {
    constructor(kind, certainty, operation, problem) {
        super(kind, certainty, operation, problem);
        this.name = "BrowserSdkError";
    }
}
function invalid() {
    throw new BrowserSdkError("validation", "not-submitted");
}
export function sdkId(value) {
    if (!identifier(value))
        invalid();
    return value;
}
export function sdkRevision(value) {
    if (!Number.isSafeInteger(value) || value < 0)
        invalid();
    return value;
}
function object(v) {
    return jsonObject(v);
}
function freeze(v) {
    if (v && typeof v === "object") {
        for (const child of Object.values(v))
            freeze(child);
        Object.freeze(v);
    }
    return v;
}
function copy(v) {
    return parseCanonicalJson(new TextEncoder().encode(canonicalJson(v, 65536)), 65536);
}
function boundedText(v, max = 256, empty = false) {
    return (typeof v === "string" &&
        (empty || v.length > 0) &&
        new TextEncoder().encode(v).length <= max &&
        !/[\u0000-\u001f\u007f-\u009f]/.test(v));
}
const states = new Set([
    "accepted",
    "preparing",
    "ready",
    "committing",
    "completed",
    "cancelled",
    "failed",
    "interrupted",
    "unknown",
]);
const kinds = new WeakMap();
function kind(command, scope, decode) {
    const k = Object.freeze({ command });
    kinds.set(k, { command, scope, decode });
    return k;
}
function numeric(v) {
    return Number.isSafeInteger(v) && Number(v) >= 0;
}
function sessionView(v, cid, malformed) {
    if (!object(v) ||
        !identifier(v.id) ||
        v.conversation_id !== cid ||
        !boundedText(v.state, 64) ||
        !Array.isArray(v.media) ||
        v.media.length > 4 ||
        v.media.some((m) => typeof m !== "string" ||
            !["audio", "video", "screen", "data"].includes(m)) ||
        new Set(v.media).size !== v.media.length ||
        !(v.intent === null || boundedText(v.intent)) ||
        !(v.capabilities_offer === null || object(v.capabilities_offer)) ||
        !Array.isArray(v.codec_preferences) ||
        v.codec_preferences.length > 32 ||
        v.codec_preferences.some((c) => !boundedText(c, 128)) ||
        typeof v.is_main !== "boolean" ||
        !numeric(v.revision) ||
        !numeric(v.topology_revision) ||
        !numeric(v.created_at) ||
        !numeric(v.updated_at) ||
        !(v.ended_reason === null || boundedText(v.ended_reason, 256, true)))
        return malformed();
    return freeze({
        id: v.id,
        conversationId: cid,
        state: v.state,
        media: v.media,
        intent: v.intent,
        capabilitiesOffer: v.capabilities_offer,
        codecPreferences: v.codec_preferences,
        isMain: v.is_main,
        revision: v.revision,
        topologyRevision: v.topology_revision,
        createdAt: v.created_at,
        updatedAt: v.updated_at,
        endedReason: v.ended_reason,
    });
}
function sessionResult(value, client, key) {
    const bad = () => current(client).malformed(key, "recorded");
    if (!object(value) ||
        value.kind !== "session_control" ||
        !object(value.data))
        return bad();
    const d = value.data;
    if (d.operation_id !== key.operationId ||
        d.action !== key.command ||
        !object(d.session))
        return bad();
    const view = sessionView(d.session, key.scope.conversationId, bad);
    if (view.id !== key.scope.sessionId)
        return bad();
    return new BrowserSession(client, view.conversationId, view.id, view);
}
function decisionResult(value, client, key) {
    const bad = () => current(client).malformed(key, "recorded");
    if (!object(value) ||
        value.kind !== "session_decision" ||
        !object(value.data))
        return bad();
    const d = value.data;
    if (d.operation_id !== key.operationId ||
        d.conversation_id !== key.scope.conversationId ||
        d.session_id !== key.scope.sessionId ||
        d.state !==
            (key.command === "session.reject" ? "rejected" : "cancelled") ||
        !numeric(d.revision) ||
        !boundedText(d.reason, 256, true) ||
        !Number.isInteger(d.reason_code) ||
        Number(d.reason_code) < 0 ||
        Number(d.reason_code) > 65535)
        return bad();
    return freeze({
        session: client.conversations
            .byId(d.conversation_id)
            .sessions.byId(d.session_id),
        state: d.state,
        topologyRevision: d.revision,
    });
}
export const browserOperationKinds = Object.freeze({
    conversationCreate: kind("create", "root", (v, c, k) => {
        if (!object(v) ||
            v.kind !== "conversation" ||
            !object(v.data) ||
            !identifier(v.data.id) ||
            !object(v.data.receipt) ||
            v.data.receipt.operation_id !== k.operationId)
            return current(c).malformed(k, "recorded");
        return c.conversations.byId(v.data.id);
    }),
    sessionInvite: kind("session.invite", "session", sessionResult),
    sessionAccept: kind("session.accept", "session", sessionResult),
    sessionEnd: kind("session.end", "session", sessionResult),
    sessionReject: kind("session.reject", "session", decisionResult),
    sessionCancelInvitation: kind("session.cancel", "session", decisionResult),
});
function descriptor(k) {
    const d = kinds.get(k);
    if (!d)
        invalid();
    return d;
}
function reference(saved, k) {
    const d = descriptor(k);
    if (!object(saved) ||
        Object.keys(saved).sort().join() !==
            "command,operation_id,schema,scope" ||
        saved.schema !== "connect-operation-reference/1" ||
        saved.command !== d.command ||
        !identifier(saved.operation_id) ||
        !object(saved.scope) ||
        Object.keys(saved.scope).some((v) => !["conversation_id", "session_id", "connection_id"].includes(v)))
        invalid();
    const s = saved.scope;
    if (d.scope === "root"
        ? Object.keys(s).length !== 0
        : !identifier(s.conversation_id) ||
            !identifier(s.session_id) ||
            s.connection_id !== undefined)
        invalid();
    return freeze({
        operationId: saved.operation_id,
        command: d.command,
        scope: {
            ...(s.conversation_id === undefined
                ? {}
                : { conversationId: s.conversation_id }),
            ...(s.session_id === undefined
                ? {}
                : { sessionId: s.session_id }),
        },
    });
}
function referenceJson(key) {
    if (!key || !object(key.scope))
        invalid();
    return {
        schema: "connect-operation-reference/1",
        operation_id: key.operationId,
        command: key.command,
        scope: {
            ...(key.scope.conversationId === undefined
                ? {}
                : { conversation_id: key.scope.conversationId }),
            ...(key.scope.sessionId === undefined
                ? {}
                : { session_id: key.scope.sessionId }),
            ...(key.scope.connectionId === undefined
                ? {}
                : { connection_id: key.scope.connectionId }),
        },
    };
}
class Budget {
    until;
    signal;
    constructor(options) {
        if (!options ||
            !Number.isFinite(options.timeoutMs) ||
            options.timeoutMs < 1 ||
            options.timeoutMs > 60000)
            invalid();
        this.until = performance.now() + options.timeoutMs;
        this.signal = options.signal;
        this.options();
    }
    options() {
        if (this.signal?.aborted)
            throw new BrowserSdkError("cancelled", "read-only");
        const left = this.until - performance.now();
        if (left <= 0)
            throw new BrowserSdkError("deadline", "read-only");
        return { timeoutMs: left, signal: this.signal };
    }
    async delay() {
        const wait = Math.min(50, this.options().timeoutMs);
        await new Promise((resolve, reject) => {
            const done = () => {
                clearTimeout(timer);
                this.signal?.removeEventListener("abort", abort);
                resolve();
            }, abort = () => {
                clearTimeout(timer);
                this.signal?.removeEventListener("abort", abort);
                reject(new BrowserSdkError("cancelled", "read-only"));
            };
            const timer = setTimeout(done, wait);
            this.signal?.addEventListener("abort", abort, { once: true });
            if (this.signal?.aborted)
                abort();
        });
    }
}
function translated(error, certainty, key) {
    if (error instanceof BrowserSdkError) {
        if (key && !error.operation)
            return new BrowserSdkError(error.kind, certainty, key, error.problem);
        return error;
    }
    if (error instanceof ConnectError) {
        const kinds = {
            timeout: "deadline",
            cancelled: "cancelled",
            protocol: "protocol",
            authentication: "authentication",
            forbidden: "authentication",
            unauthorized: "authentication",
            "invalid-request": "validation",
            configuration: "configuration",
            trust: "trust",
            conflict: "conflict",
            unsupported: "unsupported",
            "unsupported-feature": "unsupported",
            "resync-required": "resync-required",
        };
        const kind = error.code === "refused"
            ? error.problem?.code === 401 || error.problem?.code === 403
                ? "authentication"
                : "application"
            : (kinds[error.code] ?? "transport-uncertain");
        return new BrowserSdkError(kind, ["refused", "forbidden", "unauthorized"].includes(error.code)
            ? "refused"
            : certainty, key, error.problem);
    }
    return new BrowserSdkError("protocol", certainty, key);
}
class State {
    owner;
    tenant;
    actor;
    hints = [];
    hintBytes = 0;
    wake;
    reader = false;
    stopEvents;
    stopClose;
    constructor(owner, tenant, actor) {
        this.owner = owner;
        this.tenant = tenant;
        this.actor = actor;
    }
    get peer() {
        try {
            return this.owner.client;
        }
        catch (e) {
            throw translated(e, "read-only");
        }
    }
    malformed(key, certainty = "read-only") {
        this.peer.close(new ConnectError("protocol", "Invalid typed browser response"));
        throw new BrowserSdkError("protocol", certainty, key);
    }
    observe(peer, epoch) {
        if (peer.closed ||
            this.owner.status.physicalEpoch !== epoch ||
            this.peer !== peer)
            throw new BrowserSdkError("cancelled", "read-only");
    }
    watch() {
        this.stopEvents?.();
        this.stopClose?.();
        this.hints = [];
        this.hintBytes = 0;
        const peer = this.peer, epoch = this.owner.status.physicalEpoch;
        this.stopClose = peer.onClose(() => {
            this.hints = [];
            this.hintBytes = 0;
            this.wake?.();
        });
        this.stopEvents = peer.onEvent((frame) => {
            if (frame.type !== "session.invite")
                return;
            const bytes = new TextEncoder().encode(canonicalJson(frame)).length;
            if (this.hints.length >= 64 || this.hintBytes + bytes > 1048576)
                this.malformed();
            this.hints.push({ frame, epoch });
            this.hintBytes += bytes;
            this.wake?.();
        });
    }
    async request(type, payload, scope, options, key) {
        const peer = this.peer, epoch = this.owner.status.physicalEpoch;
        try {
            const frames = await peer.request(type, payload, scope, options);
            this.observe(peer, epoch);
            return frames;
        }
        catch (e) {
            throw translated(e, key ? "uncertain" : "read-only", key);
        }
    }
    async snapshot(cid, options) {
        const budget = new Budget(options), peer = this.peer, epoch = this.owner.status.physicalEpoch;
        let page, cursor, revision, bytes = 0;
        const facts = [], pages = new Set();
        for (let i = 0; i < 256; i++) {
            let batch;
            try {
                batch = await this.owner.recoverConversation({ conversationId: cid, page, limit: 63 }, budget.options());
            }
            catch (e) {
                throw translated(e, "read-only");
            }
            this.observe(peer, epoch);
            if (batch.kind !== "snapshot" || batch.frames.length !== 1)
                this.malformed();
            const body = batch.frames[0].payload;
            if (revision !== undefined &&
                (revision !== body.revision || cursor !== batch.cursor))
                throw new BrowserSdkError("resync-required", "read-only");
            revision = Number(body.revision);
            cursor = batch.cursor;
            for (const item of body.items) {
                if (!object(item) ||
                    typeof item.kind !== "string" ||
                    !object(item.data))
                    this.malformed();
                bytes += new TextEncoder().encode(canonicalJson(item)).length;
                if (bytes > 2 * 1048576)
                    throw new BrowserSdkError("resync-required", "read-only");
                facts.push(item);
            }
            if (!batch.hasMore)
                break;
            if (i === 255 || !batch.nextPage || pages.has(batch.nextPage))
                this.malformed();
            page = batch.nextPage;
            pages.add(page);
        }
        const participants = [], sessions = new Map(), appearances = new Map();
        let conversation;
        const memberIds = new Set();
        for (const item of facts) {
            const d = item.data;
            if (item.kind === "conversation") {
                if (conversation ||
                    d.conversation_id !== cid ||
                    d.revision !== revision ||
                    typeof d.closed !== "boolean")
                    this.malformed();
                conversation = d;
            }
            else if (item.kind === "participant") {
                if (d.conversation_id !== cid ||
                    !identifier(d.participant_id) ||
                    !identifier(d.identity_id) ||
                    memberIds.has(d.participant_id) ||
                    !["human", "ai", "system", "external"].includes(String(d.kind)) ||
                    !boundedText(d.role, 128) ||
                    !numeric(d.membership_epoch) ||
                    d.membership_epoch < 1 ||
                    typeof d.active !== "boolean" ||
                    !(d.removal_reason === null ||
                        boundedText(d.removal_reason)))
                    this.malformed();
                memberIds.add(d.participant_id);
                participants.push(freeze({
                    id: d.participant_id,
                    conversationId: cid,
                    identityId: d.identity_id,
                    kind: d.kind,
                    role: d.role,
                    membershipEpoch: d.membership_epoch,
                    active: d.active,
                    removalReason: d.removal_reason,
                }));
            }
            else if (item.kind === "session") {
                const s = sessionView(d, cid, () => this.malformed());
                if (sessions.has(s.id))
                    this.malformed();
                sessions.set(s.id, s);
            }
            else if (item.kind === "session_participant") {
                if (!identifier(d.session_id) ||
                    !identifier(d.participant_id) ||
                    !numeric(d.membership_epoch) ||
                    !boundedText(d.role, 128) ||
                    ![
                        "invited",
                        "joining",
                        "present",
                        "left",
                        "removed",
                    ].includes(String(d.state)))
                    this.malformed();
                const id = d.session_id + ":" + d.participant_id;
                if (appearances.has(id))
                    this.malformed();
                appearances.set(id, freeze(copy(d)));
            }
        }
        if (!conversation || revision === undefined || cursor === undefined)
            this.malformed();
        budget.options();
        this.observe(peer, epoch);
        return {
            conversation: freeze({
                id: cid,
                revision: revision,
                cursor,
                closed: conversation.closed,
                participants,
            }),
            sessions,
            appearances,
        };
    }
}
const clients = new WeakMap();
function current(client) {
    const s = clients.get(client);
    if (!s)
        invalid();
    return s;
}
export class BrowserOperation {
    client;
    kind;
    reference;
    #record;
    #revision = -1;
    /** @internal Instances are supplied by SDK-owned mutation kinds. */
    constructor(client, ref, kind) {
        this.client = client;
        this.kind = kind;
        this.reference = reference(referenceJson(ref), kind);
        Object.freeze(this);
    }
    referenceJson() {
        return copy(referenceJson(this.reference));
    }
    async observation(options) {
        const s = current(this.client);
        let value;
        try {
            value = await s.owner.recoverOperation(this.reference, new Budget(options).options());
        }
        catch (e) {
            throw translated(e, "read-only", this.reference);
        }
        if (typeof value.record_id !== "string" ||
            !identifier(value.record_id) ||
            (this.#record !== undefined && value.record_id !== this.#record) ||
            Number(value.revision) < this.#revision)
            s.malformed(this.reference);
        this.#record = value.record_id;
        this.#revision = Number(value.revision);
        return {
            snapshot: freeze({
                key: this.reference,
                state: value.state,
                revision: Number(value.revision),
                recordId: value.record_id,
                ...(value.problem === undefined
                    ? {}
                    : { problem: copy(value.problem) }),
            }),
            ...(value.result === undefined
                ? {}
                : { result: copy(value.result) }),
        };
    }
    async status(options) {
        return (await this.observation(options)).snapshot;
    }
    async wait(options) {
        const budget = new Budget(options);
        for (;;) {
            const o = await this.observation(budget.options());
            if (o.snapshot.state === "completed") {
                if (o.result === undefined)
                    throw new BrowserSdkError("receipt-unavailable", "recorded", this.reference);
                const value = descriptor(this.kind).decode(o.result, this.client, this.reference);
                budget.options();
                return Object.freeze({ snapshot: o.snapshot, value });
            }
            if (["failed", "cancelled"].includes(o.snapshot.state))
                throw new BrowserSdkError("application", "recorded", this.reference);
            await budget.delay();
        }
    }
    async waitUntilReady(options) {
        const budget = new Budget(options);
        for (;;) {
            const s = await this.status(budget.options());
            if (["ready", "committing", "completed"].includes(s.state))
                return s;
            if (["failed", "cancelled"].includes(s.state))
                throw new BrowserSdkError("application", "recorded", this.reference);
            await budget.delay();
        }
    }
}
const submit = Symbol("submit");
export class BrowserSdkClient {
    operationKinds = browserOperationKinds;
    conversations = {
        byId: (id) => {
            sdkId(id);
            return new BrowserConversation(this, id);
        },
        create: (input, options) => {
            if (!input ||
                !["persistent", "ephemeral"].includes(input.policy) ||
                !Array.isArray(input.participants) ||
                input.participants.length > 128)
                invalid();
            const identities = new Set();
            const participants = input.participants.map((p) => {
                sdkId(p.identityId);
                if (identities.has(p.identityId) || !boundedText(p.role, 64))
                    invalid();
                identities.add(p.identityId);
                return { identity_id: p.identityId, role: p.role };
            });
            const idle = input.idleCloseSeconds ??
                (input.policy === "ephemeral" ? 60 : undefined);
            if (idle !== undefined &&
                (!Number.isInteger(idle) || idle < 1 || idle > 604800))
                invalid();
            return this[submit](browserOperationKinds.conversationCreate, {
                tenant_id: current(this).tenant,
                policy: input.policy,
                initial_participants: participants,
                ...(idle === undefined ? {} : { idle_close_secs: idle }),
                ...(input.metadata === undefined
                    ? {}
                    : { metadata: input.metadata }),
            }, {}, options);
        },
    };
    operations = {
        restore: (saved, k) => new BrowserOperation(this, reference(copy(saved), k), k),
        recover: (ref) => {
            if (!ref)
                invalid();
            const k = Object.values(browserOperationKinds).find((k) => k.command === ref.command);
            if (!k)
                invalid();
            return new BrowserOperation(this, ref, k);
        },
    };
    constructor(s) {
        clients.set(this, s);
        s.watch();
    }
    static async connect(options) {
        sdkId(options.tenantId);
        const tenant = options.tenantId, actor = options.identityId;
        try {
            return new BrowserSdkClient(new State(await BrowserTransportClient.connect(options), tenant, actor));
        }
        catch (error) {
            throw translated(error, "not-submitted");
        }
    }
    get transport() {
        return current(this).owner.status;
    }
    async reconnect(options = {}) {
        const s = current(this);
        try {
            await s.owner.reconnect(options);
            s.watch();
        }
        catch (error) {
            throw translated(error, "not-submitted");
        }
    }
    close() {
        const s = current(this);
        s.owner.close();
        s.stopEvents?.();
        s.stopClose?.();
        s.hints = [];
        s.hintBytes = 0;
        s.wake?.();
    }
    async [submit](k, body, scope, options) {
        const d = descriptor(k);
        if (!options || !identifier(options.operationId))
            invalid();
        const key = reference(referenceJson({
            operationId: options.operationId,
            command: d.command,
            scope: {
                ...(scope.cid === undefined
                    ? {}
                    : { conversationId: scope.cid }),
                ...(scope.sid === undefined
                    ? {}
                    : { sessionId: scope.sid }),
            },
        }), k);
        const budget = new Budget(options);
        const captured = copy(body);
        try {
            const frames = await current(this).request(d.command === "create" ? "conversation.create" : d.command, captured, { ...scope, operation_id: key.operationId }, budget.options(), key);
            const updates = frames.filter((f) => f.type === "operation.updated");
            if (d.scope === "session") {
                if (updates.length !== 1 ||
                    updates[0].cid !== scope.cid ||
                    updates[0].sid !== scope.sid ||
                    updates[0].connid !== undefined ||
                    updates[0].payload.operation_id !== key.operationId ||
                    !states.has(String(updates[0].payload.state)) ||
                    !numeric(updates[0].payload.revision))
                    current(this).malformed(key, "uncertain");
            }
            else {
                const ack = frames.find((f) => f.type === "ack"), details = ack?.payload.details;
                if (!object(details) ||
                    !object(details.conversation) ||
                    !identifier(details.conversation.id) ||
                    !object(details.conversation.receipt) ||
                    details.conversation.receipt.operation_id !==
                        key.operationId)
                    current(this).malformed(key, "uncertain");
            }
            budget.options();
            return new BrowserOperation(this, key, k);
        }
        catch (error) {
            throw translated(error, "uncertain", key);
        }
    }
    incomingInvitations(options) {
        return incoming(this, options);
    }
}
export class BrowserParticipant {
    client;
    conversationId;
    id;
    constructor(client, conversationId, id) {
        this.client = client;
        this.conversationId = conversationId;
        this.id = id;
        Object.freeze(this);
    }
    async refresh(options) {
        const snapshot = await current(this.client).snapshot(this.conversationId, options), p = snapshot.conversation.participants.find((p) => p.id === this.id);
        if (!p)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return p;
    }
}
export class BrowserConversation {
    client;
    id;
    participants = {
        byId: (id) => {
            sdkId(id);
            return new BrowserParticipant(this.client, this.id, id);
        },
        byIdentity: async (id, options) => {
            sdkId(id);
            const s = await this.refresh(options), p = s.participants.find((p) => p.identityId === id && p.active);
            if (!p)
                throw new BrowserSdkError("receipt-unavailable", "read-only");
            return new BrowserParticipant(this.client, this.id, p.id);
        },
    };
    sessions = {
        byId: (id) => {
            sdkId(id);
            return new BrowserSession(this.client, this.id, id);
        },
        invite: (input, options) => {
            if (!input ||
                !identifier(input.sessionId) ||
                !Array.isArray(input.to) ||
                input.to.length > 128 ||
                input.to.some((p) => !identifier(p)) ||
                new Set(input.to).size !== input.to.length ||
                ![
                    "voice",
                    "video",
                    "voice+video",
                    "screen-share",
                    "text-chat",
                    "mixed",
                ].includes(input.medium) ||
                !boundedText(input.intent) ||
                !object(input.capabilities) ||
                new TextEncoder().encode(canonicalJson(input.capabilities))
                    .length > 16384)
                invalid();
            return this.client[submit](browserOperationKinds.sessionInvite, {
                to: [...input.to],
                medium: input.medium,
                intent: input.intent,
                capabilities_offer: input.capabilities,
            }, { cid: this.id, sid: input.sessionId }, options);
        },
    };
    constructor(client, id) {
        this.client = client;
        this.id = id;
        Object.freeze(this);
    }
    async refresh(options) {
        return (await current(this.client).snapshot(this.id, options))
            .conversation;
    }
}
export class BrowserSession {
    client;
    conversationId;
    id;
    receiptSnapshot;
    constructor(client, conversationId, id, receiptSnapshot) {
        this.client = client;
        this.conversationId = conversationId;
        this.id = id;
        this.receiptSnapshot = receiptSnapshot;
        Object.freeze(this);
    }
    async refresh(options) {
        const snapshot = await current(this.client).snapshot(this.conversationId, options), s = snapshot.sessions.get(this.id);
        if (!s)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return s;
    }
    accept(input, options) {
        if (!input ||
            !object(input.capabilities) ||
            new TextEncoder().encode(canonicalJson(input.capabilities)).length >
                16384)
            invalid();
        return this.client[submit](browserOperationKinds.sessionAccept, { capabilities_answer: input.capabilities }, {
            cid: this.conversationId,
            sid: this.id,
            expected_revision: sdkRevision(input.expectedTopologyRevision),
        }, options);
    }
    decide(k, input, options) {
        if (!input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            !boundedText(input.reason, 256, true))
            invalid();
        return this.client[submit](k, { reason_code: input.reasonCode, reason: input.reason }, {
            cid: this.conversationId,
            sid: this.id,
            expected_revision: sdkRevision(input.expectedTopologyRevision),
        }, options);
    }
    reject(input, options) {
        return this.decide(browserOperationKinds.sessionReject, input, options);
    }
    cancelInvitation(input, options) {
        return this.decide(browserOperationKinds.sessionCancelInvitation, input, options);
    }
    end(input, options) {
        return this.decide(browserOperationKinds.sessionEnd, input, options);
    }
}
function incoming(client, options) {
    new Budget(options);
    const s = current(client);
    if (s.reader)
        throw new BrowserSdkError("conflict", "read-only");
    s.reader = true;
    const captured = { timeoutMs: options.timeoutMs, signal: options.signal }, stop = new AbortController();
    let closed = false, busy = false;
    const reader = {
        [Symbol.asyncIterator]() {
            return reader;
        },
        async next() {
            if (closed)
                return { done: true, value: undefined };
            if (busy)
                throw new BrowserSdkError("conflict", "read-only");
            const combined = AbortSignal.any([
                stop.signal,
                ...(captured.signal ? [captured.signal] : []),
            ]), budget = new Budget({ ...captured, signal: combined });
            busy = true;
            try {
                for (;;) {
                    budget.options();
                    const peer = s.peer, epoch = s.owner.status.physicalEpoch;
                    const queued = s.hints.shift();
                    if (!queued) {
                        await new Promise((resolve, reject) => {
                            let timer;
                            const finish = () => {
                                clearTimeout(timer);
                                combined.removeEventListener("abort", cancel);
                                if (s.wake === finish)
                                    s.wake = undefined;
                                resolve();
                            }, cancel = () => {
                                clearTimeout(timer);
                                combined.removeEventListener("abort", cancel);
                                if (s.wake === finish)
                                    s.wake = undefined;
                                reject(new BrowserSdkError("cancelled", "read-only"));
                            };
                            s.wake = finish;
                            timer = setTimeout(finish, budget.options().timeoutMs);
                            combined.addEventListener("abort", cancel, {
                                once: true,
                            });
                            if (combined.aborted)
                                cancel();
                        });
                        continue;
                    }
                    s.hintBytes -= new TextEncoder().encode(canonicalJson(queued.frame)).length;
                    if (queued.epoch !== epoch)
                        continue;
                    const f = queued.frame, p = f.payload;
                    if (f.connid !== undefined ||
                        f.in_reply_to !== undefined ||
                        f.expected_revision !== undefined ||
                        !identifier(f.cid) ||
                        !identifier(f.sid) ||
                        !identifier(f.operation_id) ||
                        !identifier(p.from) ||
                        !Array.isArray(p.to) ||
                        p.to.length !== 1 ||
                        !identifier(p.to[0]) ||
                        p.from === p.to[0] ||
                        ![
                            "voice",
                            "video",
                            "voice+video",
                            "screen-share",
                            "text-chat",
                            "mixed",
                        ].includes(String(p.medium)) ||
                        !boundedText(p.intent) ||
                        !object(p.capabilities_offer))
                        s.malformed();
                    const to = p.to[0];
                    const snapshot = await s.snapshot(f.cid, budget.options());
                    s.observe(peer, epoch);
                    const target = snapshot.conversation.participants.find((v) => v.id === to &&
                        v.identityId === s.actor &&
                        v.active), view = snapshot.sessions.get(f.sid), appearance = snapshot.appearances.get(f.sid + ":" + to);
                    if (snapshot.conversation.closed ||
                        !target ||
                        !view ||
                        ["ended", "rejected", "cancelled", "failed"].includes(view.state) ||
                        !appearance ||
                        appearance.membership_epoch !==
                            target.membershipEpoch ||
                        appearance.state !== "invited")
                        continue;
                    return {
                        done: false,
                        value: freeze({
                            session: new BrowserSession(client, f.cid, f.sid),
                            from: p.from,
                            to: to,
                            operationId: f.operation_id,
                            source: {
                                kind: "session.invite",
                                correlationId: f.operation_id,
                            },
                            medium: p.medium,
                            intent: p.intent,
                            capabilities: copy(p.capabilities_offer),
                        }),
                    };
                }
            }
            catch (e) {
                if (closed)
                    return { done: true, value: undefined };
                throw translated(e, "read-only");
            }
            finally {
                busy = false;
            }
        },
        async close() {
            if (closed)
                return;
            closed = true;
            s.reader = false;
            stop.abort();
            s.wake?.();
        },
        async return() {
            await reader.close();
            return { done: true, value: undefined };
        },
    };
    return reader;
}
