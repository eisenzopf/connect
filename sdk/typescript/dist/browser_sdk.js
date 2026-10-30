import { createBrowserRaw, } from "./browser_sdk_raw.js";
import { BrowserTasks } from "./browser_tasks.js";
import { handoffIntent, moveIntent, movementCompletion, } from "./browser_sdk_movements.js";
import { transitionCommand, transitionContinuation, transitionObservation, consultationCompletion, consultBody, } from "./browser_sdk_consultations.js";
import { operationDecisionResult, } from "./browser_sdk_decisions.js";
import { BrowserApplicationReads, } from "./browser_application_reads.js";
import { identityKind, membershipRole, membershipRevision, participantView, membershipResult, } from "./browser_sdk_membership.js";
import { messageBody, messageResult, } from "./browser_sdk_messages.js";
import { createBrowserEvents, isBrowserCursor, } from "./browser_sdk_events.js";
export function browserCursor(value) {
    if (!isBrowserCursor(value))
        invalid();
    return value;
}
import { BrowserInvitationInbox } from "./browser_sdk_inbox.js";
import { fields, nonEmpty, sessionInviteBody, sessionUpdateBody, sessionCapabilities, offeredStreams, } from "./browser_sdk_sessions.js";
import { subscribeBody, unsubscribeBody, updateBody, selectionResult, updateResult, } from "./browser_sdk_streams.js";
import { BrowserRtcConnection, } from "./browser_sdk_webrtc.js";
import { browserIceConfiguration } from "./browser_ice.js";
import { BrowserTransportClient, } from "./browser_transport_client.js";
import { CanonicalError, } from "./canonical_error.js";
import { BrowserMediaPeer, } from "./browser_media.js";
import { BrowserMediaCapture, } from "./browser_media_capture.js";
import { BrowserMediaRenderer } from "./browser_media_renderer.js";
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
    return parseCanonicalJson(new TextEncoder().encode(canonicalJson(v, 1048576)), 1048576);
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
        (key.command === "session.update"
            ? ![
                "participant-leave",
                "role-change",
                "medium-upgrade",
                "codec-renegotiate",
            ].includes(String(d.action))
            : d.action !== key.command) ||
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
function connectionResult(value, client, key) {
    const bad = () => current(client).malformed(key, "recorded");
    if (!object(value))
        return bad();
    let revision;
    if (key.command === "connection.offer") {
        if (!numeric(value.stream_revision) ||
            value.stream_revision < 1 ||
            !Array.isArray(value.streams) ||
            !value.streams.length ||
            value.streams.length > 32 ||
            value.streams.some((v) => !identifier(v)) ||
            new Set(value.streams).size !== value.streams.length)
            return bad();
        revision = value.stream_revision;
        const s = current(client), local = s.local.get(key.scope.connectionId);
        if (local &&
            local.offerOperation === key.operationId &&
            local.cid === key.scope.conversationId &&
            local.sid === key.scope.sessionId &&
            local.epoch === s.owner.status.physicalEpoch)
            local.revision = Math.max(local.revision ?? 0, revision);
    }
    else {
        if (value.ended !== true ||
            value.connection_id !== key.scope.connectionId)
            return bad();
        current(client).mediaPeer?.retireLocalConnection(key.scope.connectionId);
        current(client).local.get(key.scope.connectionId)?.rtc?.close();
        current(client).local.delete(key.scope.connectionId);
    }
    return new BrowserConnection(client, key.scope.conversationId, key.scope.sessionId, key.scope.connectionId, revision);
}
function controlConnection(value, client, key) {
    if (!object(value) ||
        !numeric(value.stream_revision) ||
        value.stream_revision < 1)
        return current(client).malformed(key, "recorded");
    return new BrowserConnection(client, key.scope.conversationId, key.scope.sessionId, key.scope.connectionId, value.stream_revision);
}
function observedControl(value, client, key) {
    const s = current(client), local = s.local.get(key.scope.connectionId);
    if (local &&
        local.cid === key.scope.conversationId &&
        local.sid === key.scope.sessionId &&
        local.epoch === s.owner.status.physicalEpoch &&
        local.generation === value.connectionGeneration)
        local.revision = Math.max(local.revision ?? 0, value.streamRevision);
    return value;
}
function participantResult(v, c, key) {
    const result = membershipResult(v, key, () => current(c).malformed(key, "recorded"));
    return new BrowserParticipant(c, key.scope.conversationId, result.snapshot.id, result.snapshot, result.revision);
}
function transitionTopology(client, key, value) {
    const connection = (id) => new BrowserConnection(client, key.scope.conversationId, key.scope.sessionId, id);
    return freeze({
        retained: value.retained.map(connection),
        admitted: value.admitted.map(connection),
        retired: value.retired,
        topologyRevision: value.topologyRevision,
        ...(value.conversationRevision === undefined
            ? {}
            : { conversationRevision: value.conversationRevision }),
    });
}
function consultationResult(value, client, key) {
    const observed = consultationCompletion(value, key, () => current(client).malformed(key, "recorded"));
    return freeze({
        session: new BrowserSession(client, key.scope.conversationId, key.scope.sessionId),
        ...transitionTopology(client, key, observed),
    });
}
function consultationReturnResult(value, client, key) {
    const observed = consultationCompletion(value, key, () => current(client).malformed(key, "recorded"));
    return new BrowserSession(client, key.scope.conversationId, key.scope.sessionId, undefined, freeze({
        consultationRecordId: observed.consultationRecordId,
        remoteContinuing: observed.remoteContinuing,
        topology: transitionTopology(client, key, observed),
    }));
}
function handoffResult(value, client, key) {
    const bad = () => current(client).malformed(key, "recorded"), observed = movementCompletion(value, key, bad);
    if (observed.kind !== "handoff")
        return bad();
    return freeze({
        ...transitionTopology(client, key, observed),
        ...(observed.roleChange === undefined
            ? {}
            : { roleChange: observed.roleChange }),
    });
}
function moveResult(value, client, key) {
    const bad = () => current(client).malformed(key, "recorded"), observed = movementCompletion(value, key, bad);
    if (observed.kind !== "replacement")
        return bad();
    return freeze({
        connection: new BrowserConnection(client, key.scope.conversationId, key.scope.sessionId, observed.connectionId),
        ...transitionTopology(client, key, observed),
        binding: observed.binding,
    });
}
export const browserOperationKinds = Object.freeze({
    sessionHandoff: kind("session.handoff", "connection", handoffResult),
    connectionReplace: kind("connection.replace", "connection", moveResult),
    sessionConsult: kind("session.consult", "connection", consultationResult),
    sessionConsultReturn: kind("session.consult.return", "connection", consultationReturnResult),
    operationCancel: kind("operation.cancel", "decision", (v, c, key) => operationDecisionResult(v, key, () => current(c).malformed(key, "recorded"))),
    operationReconcile: kind("operation.reconcile", "decision", (v, c, key) => operationDecisionResult(v, key, () => current(c).malformed(key, "recorded"))),
    connectionUpdate: kind("connection.update", "connection", (v, c, k) => observedControl(updateResult(v, k, controlConnection(v, c, k), () => current(c).malformed(k, "recorded")), c, k)),
    streamSubscribe: kind("stream.subscribe", "connection", (v, c, k) => observedControl(selectionResult(v, k, controlConnection(v, c, k), () => current(c).malformed(k, "recorded")), c, k)),
    streamUnsubscribe: kind("stream.unsubscribe", "connection", (v, c, k) => observedControl(selectionResult(v, k, controlConnection(v, c, k), () => current(c).malformed(k, "recorded")), c, k)),
    connectionOffer: kind("connection.offer", "connection", connectionResult),
    connectionEnd: kind("connection.end", "connection", connectionResult),
    conversationCreate: kind("conversation.create", "root", (v, c, k) => {
        if (!object(v) ||
            v.kind !== "conversation" ||
            !object(v.data) ||
            !identifier(v.data.id) ||
            !object(v.data.receipt) ||
            v.data.receipt.operation_id !== k.operationId)
            return current(c).malformed(k, "recorded");
        return c.conversations.byId(v.data.id);
    }),
    participantAdd: kind("conversation.participant.add", "conversation", participantResult),
    participantUpdate: kind("conversation.participant.update", "conversation", participantResult),
    participantRemove: kind("conversation.participant.remove", "conversation", participantResult),
    messageSend: kind("message.send", "conversation", (v, c, k) => messageResult(v, k, () => current(c).malformed(k, "recorded"))),
    sessionInvite: kind("session.invite", "session", sessionResult),
    sessionAccept: kind("session.accept", "session", sessionResult),
    sessionUpdate: kind("session.update", "session", sessionResult),
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
            (saved.continuation === undefined
                ? "command,operation_id,schema,scope"
                : "command,continuation,operation_id,schema,scope") ||
        saved.schema !== "connect-operation-reference/1" ||
        saved.command !== d.command ||
        !identifier(saved.operation_id) ||
        !object(saved.scope) ||
        Object.keys(saved.scope).some((v) => !["conversation_id", "session_id", "connection_id"].includes(v)))
        invalid();
    const s = saved.scope;
    if (d.scope === "decision"
        ? s.connection_id !== undefined ||
            (s.conversation_id !== undefined &&
                !identifier(s.conversation_id)) ||
            (s.session_id !== undefined &&
                (!identifier(s.session_id) ||
                    s.conversation_id === undefined))
        : d.scope === "root"
            ? Object.keys(s).length !== 0
            : !identifier(s.conversation_id) ||
                (d.scope === "conversation"
                    ? s.session_id !== undefined
                    : !identifier(s.session_id)) ||
                (d.scope === "connection"
                    ? !identifier(s.connection_id)
                    : s.connection_id !== undefined))
        invalid();
    const context = transitionContinuation(saved.continuation, {
        operationId: saved.operation_id,
        command: d.command,
        scope: { connectionId: s.connection_id },
    }, invalid);
    return freeze({
        ...(context === undefined
            ? {}
            : {
                continuation: copy(context),
            }),
        operationId: saved.operation_id,
        command: d.command,
        scope: {
            ...(s.conversation_id === undefined
                ? {}
                : { conversationId: s.conversation_id }),
            ...(s.session_id === undefined
                ? {}
                : { sessionId: s.session_id }),
            ...(s.connection_id === undefined
                ? {}
                : { connectionId: s.connection_id }),
        },
    });
}
function referenceJson(key) {
    if (!key || !object(key.scope))
        invalid();
    return {
        schema: "connect-operation-reference/1",
        ...(key.continuation === undefined
            ? {}
            : { continuation: key.continuation }),
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
        if (left < 1)
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
function mutationRemainder(budget, key) {
    try {
        return {
            operationId: key.operationId,
            ...budget.options(),
        };
    }
    catch (error) {
        const failure = translated(error, "not-submitted");
        throw new BrowserSdkError(failure.kind, "not-submitted", key, failure.problem);
    }
}
function translated(error, certainty, key) {
    if (error instanceof BrowserSdkError) {
        if (key && !error.operation)
            return new BrowserSdkError(error.kind, error.certainty === "read-only" ? certainty : error.certainty, key, error.problem);
        return error;
    }
    if (error instanceof ConnectError) {
        if (error.code === "media-denied")
            return new BrowserSdkError("application", certainty, key, {
                code: 403,
                category: "capture",
                reason: "permission_denied",
            });
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
            "unsupported-media-profile": "unsupported",
            media: "application",
            "media-failed": "application",
            "media-output-failed": "application",
            "media-input-failed": "application",
            "resync-required": "resync-required",
        };
        const kind = error.code === "refused"
            ? error.problem?.code === 401 || error.problem?.code === 403
                ? "authentication"
                : "application"
            : (kinds[error.code] ?? "transport-uncertain");
        return new BrowserSdkError(kind, ["forbidden", "unauthorized"].includes(error.code) ||
            (error.code === "refused" &&
                error.problem !== undefined &&
                error.problem.code >= 400 &&
                error.problem.code < 500)
            ? "refused"
            : certainty, key, error.problem);
    }
    return new BrowserSdkError("protocol", certainty, key);
}
class State {
    owner;
    tenant;
    actor;
    rtcConfiguration;
    applicationReads;
    cleanup = new BrowserTasks();
    mediaPeer;
    local = new Map();
    renderers = new Map();
    pending = new Map();
    captures = new Map();
    nativePeer() {
        if (this.peer.controlTransport !== "webtransport")
            throw new BrowserSdkError("unsupported", "not-submitted");
        return (this.mediaPeer ??= new BrowserMediaPeer(this.peer, {
            audio: (binding, frame) => {
                const r = this.renderers.get(binding.connectionId);
                if (r instanceof BrowserMediaRenderer)
                    r.audio(binding, frame);
            },
            video: (binding, frame) => {
                const r = this.renderers.get(binding.connectionId);
                if (r instanceof BrowserMediaRenderer)
                    r.video(binding, frame);
            },
        }));
    }
    eventReaders = new Set();
    inbox = new BrowserInvitationInbox();
    invitationRead;
    invitationFailure;
    wake;
    reader = false;
    stopEvents;
    stopClose;
    constructor(owner, tenant, actor, rtcConfiguration, applicationReads) {
        this.owner = owner;
        this.tenant = tenant;
        this.actor = actor;
        this.rtcConfiguration = rtcConfiguration;
        this.applicationReads = applicationReads;
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
    retireMedia() {
        if (this.mediaPeer)
            this.cleanup.track(this.mediaPeer.shutdown(), true);
        this.mediaPeer = undefined;
        for (const local of this.local.values()) {
            this.cleanup.track(local.ready);
            if (local.control)
                this.cleanup.track(local.control);
            if (local.rtc)
                this.cleanup.track(local.rtc.shutdown(), true);
        }
        this.local.clear();
        this.pending.clear();
        for (const renderer of this.renderers.values()) {
            renderer.close();
            this.cleanup.track(renderer.finished);
        }
        this.renderers.clear();
        this.captures.clear();
    }
    watch() {
        this.retireMedia();
        this.stopEvents?.();
        this.stopClose?.();
        this.applicationReads.retire();
        for (const retire of this.eventReaders)
            retire();
        this.eventReaders.clear();
        this.inbox.clear();
        this.invitationRead = undefined;
        this.invitationFailure = undefined;
        const peer = this.peer, epoch = this.owner.status.physicalEpoch;
        this.stopClose = peer.onClose(() => {
            this.applicationReads.retire();
            for (const retire of this.eventReaders)
                retire();
            this.inbox.clear();
            this.invitationRead = undefined;
            this.wake?.();
        });
        this.stopEvents = peer.onEvent((frame) => {
            if (this.owner.status.physicalEpoch !== epoch || peer.closed)
                return;
            try {
                if (this.inbox.record(frame, epoch))
                    this.wake?.();
            }
            catch (error) {
                this.invitationFailure = translated(error, "read-only");
                peer.close(error);
                this.wake?.();
            }
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
        const participants = [], sessions = new Map(), appearances = new Map(), connections = new Map(), streams = new Map();
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
                const view = participantView(d, cid, () => this.malformed());
                if (memberIds.has(view.id))
                    this.malformed();
                memberIds.add(view.id);
                participants.push(view);
            }
            else if (item.kind === "session") {
                const s = sessionView(d, cid, () => this.malformed());
                if (sessions.has(s.id))
                    this.malformed();
                sessions.set(s.id, s);
            }
            else if (item.kind === "connection") {
                const c = connectionView(d, cid, () => this.malformed());
                if (connections.has(c.id))
                    this.malformed();
                connections.set(c.id, c);
            }
            else if (item.kind === "stream") {
                const v = streamView(d, cid, () => this.malformed());
                const key = JSON.stringify([v.sessionId, v.connectionId, v.id]);
                if (streams.has(key))
                    this.malformed();
                streams.set(key, v);
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
            connections,
            streams,
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
const operationRetirements = new WeakMap();
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
        if (this.reference.continuation?.kind === "consultation")
            this.#record = this.reference.continuation.record_id;
        Object.freeze(this);
    }
    referenceJson() {
        const saved = referenceJson(this.reference);
        if (this.reference.continuation?.kind === "consultation" &&
            this.#record)
            saved.continuation = {
                ...saved.continuation,
                record_id: this.#record,
            };
        return copy(saved);
    }
    async observation(options) {
        const s = current(this.client);
        let value;
        try {
            value = await s.owner.recoverOperation({
                ...this.reference,
                command: this.reference.command === "conversation.create"
                    ? "create"
                    : this.reference.command === "message.send"
                        ? "send"
                        : this.reference.command,
            }, new Budget(options).options());
        }
        catch (e) {
            throw translated(e, "read-only", this.reference);
        }
        if (typeof value.record_id !== "string" ||
            !identifier(value.record_id) ||
            (this.#record !== undefined && value.record_id !== this.#record) ||
            Number(value.revision) < this.#revision)
            s.malformed(this.reference);
        if (transitionCommand(this.reference.command))
            transitionObservation(value, this.reference, undefined, () => s.malformed(this.reference, value.state === "completed" ? "recorded" : "read-only"));
        this.#record = value.record_id;
        this.#revision = Number(value.revision);
        if (value.state === "completed" && value.result !== undefined) {
            const completed = operationRetirements.get(this);
            operationRetirements.delete(this);
            completed?.(value.result);
        }
        else if (["failed", "cancelled"].includes(value.state)) {
            operationRetirements.delete(this);
        }
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
    async returnToRetained(input, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const context = this.reference.continuation, key = {
            operationId: options.operationId,
            command: "session.consult.return",
            scope: this.reference.scope,
        };
        const invalidReturn = () => {
            throw new BrowserSdkError("validation", "not-submitted", key);
        };
        fields(input, ["expectedTopologyRevision"], invalidReturn);
        const expectedTopologyRevision = input.expectedTopologyRevision;
        if (this.reference.command !== "session.consult" ||
            context?.kind !== "consultation" ||
            !identifier(options.operationId) ||
            options.operationId === this.reference.operationId ||
            !Number.isSafeInteger(expectedTopologyRevision) ||
            expectedTopologyRevision < 0 ||
            expectedTopologyRevision >= Number.MAX_SAFE_INTEGER)
            invalidReturn();
        let budget, record;
        try {
            budget = new Budget(options);
            record =
                context.kind === "consultation"
                    ? (context.record_id ??
                        this.#record ??
                        (await this.status(budget.options())).recordId)
                    : invalidReturn();
            if (!identifier(record))
                throw new BrowserSdkError("receipt-unavailable", "read-only");
            budget.options();
        }
        catch (error) {
            const failure = translated(error, "not-submitted");
            throw new BrowserSdkError(failure.kind, "not-submitted", key, failure.problem);
        }
        const scope = {
            cid: this.reference.scope.conversationId,
            sid: this.reference.scope.sessionId,
            connid: this.reference.scope.connectionId,
            expected_revision: expectedTopologyRevision,
        };
        return this.client[submit](browserOperationKinds.sessionConsultReturn, {
            consultation_record_id: record,
            retained_connid: context.retained_connection_id,
        }, scope, mutationRemainder(budget, key), scope, {
            kind: "consultation-return",
            retained_connection_id: context.retained_connection_id,
            consultation_record_id: record,
        });
    }
    cancel(options) {
        return this.decide(browserOperationKinds.operationCancel, undefined, options);
    }
    reconcile(input, options) {
        fields(input, ["outcome", "evidenceId"], invalid);
        if (!["completed", "cancelled", "failed"].includes(input.outcome) ||
            !identifier(input.evidenceId))
            invalid();
        return this.decide(browserOperationKinds.operationReconcile, { outcome: input.outcome, evidence_id: input.evidenceId }, options);
    }
    decide(kind, reconciliation, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const target = this.reference, body = { operation_id: target.operationId, ...reconciliation };
        return this.client[submit](kind, body, {
            cid: target.scope.conversationId,
            sid: target.scope.sessionId,
            connid: target.scope.connectionId,
        }, options, { cid: target.scope.conversationId, sid: target.scope.sessionId });
    }
}
/** Local exposed interfaces only; permission and negotiated media are separate. */
export const browserSdkFeatures = Object.freeze({
    originalIdOperations: "original-id-operations",
    durableEvents: "durable-events",
    incomingInvitations: "incoming-invitations",
    messageReceipts: "message-receipts",
    rawCanonical: "raw-canonical",
});
function featureSet(values) {
    const set = new Set(values.sort());
    const result = Object.freeze({
        get size() {
            return set.size;
        },
        has: (value) => set.has(value),
        entries: () => set.entries(),
        keys: () => set.keys(),
        values: () => set.values(),
        [Symbol.iterator]: () => set.values(),
        forEach: (callback, thisArg) => set.forEach((value) => callback.call(thisArg, value, value, result)),
    });
    return result;
}
const submit = Symbol("submit");
export class BrowserSdkClient {
    role = "browser";
    operationKinds = browserOperationKinds;
    raw = createBrowserRaw({
        error: (kind, certainty, key) => new BrowserSdkError(kind, certainty, key),
        perform: async (value, scope, options, key) => {
            let budget;
            try {
                fields(options, ["timeoutMs", "signal"], invalid);
                budget = new Budget(options);
            }
            catch (error) {
                const failure = translated(error, key ? "not-submitted" : "read-only", key);
                throw failure;
            }
            const s = current(this);
            let peer;
            try {
                peer = s.peer;
            }
            catch (error) {
                const failure = translated(error, key ? "not-submitted" : "read-only", key);
                throw failure;
            }
            const epoch = s.owner.status.physicalEpoch;
            try {
                const pending = peer.begin(value.type, value.payload, scope, {
                    ...budget.options(),
                    validateCanonical: true,
                }), frames = await pending.response;
                s.observe(peer, epoch);
                budget.options();
                return {
                    requestId: pending.id,
                    frames,
                    check: () => {
                        try {
                            s.observe(peer, epoch);
                            budget.options();
                        }
                        catch (error) {
                            throw translated(error, key ? "uncertain" : "read-only", key);
                        }
                    },
                    retire: () => peer.close(new ConnectError("protocol", "Invalid frozen canonical response")),
                };
            }
            catch (error) {
                throw translated(error, key ? "uncertain" : "read-only", key);
            }
        },
    });
    capabilities = Object.freeze({
        refresh: async (options) => {
            const budget = new Budget(options), s = current(this), transport = s.owner.status;
            if (transport.state !== "connected" || !transport.selectedTransport)
                throw new BrowserSdkError("cancelled", "read-only");
            const expiresAt = s.peer.authenticationExpiresAt;
            if (expiresAt === undefined || !(expiresAt > Date.now()))
                throw new BrowserSdkError("authentication", "read-only");
            const features = [
                browserSdkFeatures.originalIdOperations,
                browserSdkFeatures.durableEvents,
                browserSdkFeatures.incomingInvitations,
                browserSdkFeatures.rawCanonical,
            ];
            if (s.applicationReads.resolve)
                features.push(browserSdkFeatures.messageReceipts);
            budget.options();
            return Object.freeze({
                profile: "connect/2",
                runtimeRole: "browser",
                selectedTransport: transport.selectedTransport,
                ...(transport.fallbackReason
                    ? { fallbackReason: transport.fallbackReason }
                    : {}),
                features: featureSet(features),
            });
        },
        require: async (feature, options) => {
            if (typeof feature !== "string" ||
                !/^[a-z0-9-]{1,128}$/.test(feature))
                invalid();
            const budget = new Budget(options), s = current(this), peer = s.peer, epoch = s.owner.status.physicalEpoch;
            const snapshot = await this.capabilities.refresh({
                timeoutMs: budget.options().timeoutMs,
                signal: budget.signal,
            });
            budget.options();
            s.observe(peer, epoch);
            if (!snapshot.features.has(feature))
                throw new BrowserSdkError("unsupported", "read-only");
        },
    });
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
                sdkId(p);
                if (identities.has(p))
                    invalid();
                identities.add(p);
                return { identity_id: p, role: "participant" };
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
        restore: (saved, k) => {
            let captured;
            try {
                captured = copy(saved);
            }
            catch {
                return invalid();
            }
            return new BrowserOperation(this, reference(captured, k), k);
        },
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
        if (options.resolveApplicationRead !== undefined &&
            typeof options.resolveApplicationRead !== "function")
            invalid();
        const applicationReads = new BrowserApplicationReads(options.resolveApplicationRead, options.identityId, options.deviceId);
        const tenant = options.tenantId, actor = options.identityId;
        let media;
        try {
            media =
                browserIceConfiguration(options.media?.iceServers, options.media?.iceTransportPolicy) ?? {};
        }
        catch (error) {
            throw translated(error, "not-submitted");
        }
        try {
            return new BrowserSdkClient(new State(await BrowserTransportClient.connect(options), tenant, actor, media, applicationReads));
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
    async close(options) {
        // Validate first, but a cancelled wait must still retire every owner.
        const budget = new Budget({ timeoutMs: options?.timeoutMs });
        const signal = options.signal;
        if (signal !== undefined && !(signal instanceof AbortSignal))
            invalid();
        const s = current(this);
        s.retireMedia();
        s.cleanup.track(s.owner.shutdown(), true);
        s.cleanup.track(s.applicationReads.retire(), true);
        for (const retire of s.eventReaders)
            retire();
        s.eventReaders.clear();
        s.stopEvents?.();
        s.stopClose?.();
        s.inbox.clear();
        s.invitationRead = undefined;
        s.wake?.();
        await boundedLocal(s.cleanup.join(), {
            timeoutMs: budget.options().timeoutMs,
            signal,
        });
    }
    async [submit](k, body, scope, options, recordScope = scope, continuation) {
        const d = descriptor(k);
        if (!options || !identifier(options.operationId))
            invalid();
        let key = reference(referenceJson({
            ...(continuation === undefined ? {} : { continuation }),
            operationId: options.operationId,
            command: d.command,
            scope: {
                ...(recordScope.cid === undefined
                    ? {}
                    : { conversationId: recordScope.cid }),
                ...(recordScope.sid === undefined
                    ? {}
                    : { sessionId: recordScope.sid }),
                ...(recordScope.connid === undefined
                    ? {}
                    : { connectionId: recordScope.connid }),
            },
        }), k);
        let budget;
        try {
            budget = new Budget(options);
        }
        catch (error) {
            throw translated(error, "not-submitted", key);
        }
        const captured = copy(body);
        if (d.scope === "decision" && key.operationId === captured.operation_id)
            throw new BrowserSdkError("validation", "not-submitted", key);
        let control;
        try {
            const s = current(this), peer = s.peer, epoch = s.owner.status.physicalEpoch;
            const local = scope.connid === undefined
                ? undefined
                : s.local.get(scope.connid);
            let frames;
            if (peer.controlTransport === "webtransport" &&
                local &&
                [
                    "connection.update",
                    "stream.subscribe",
                    "stream.unsubscribe",
                ].includes(d.command)) {
                if (local.cid !== scope.cid ||
                    local.sid !== scope.sid ||
                    local.epoch !== epoch)
                    throw new BrowserSdkError("receipt-unavailable", "not-submitted", key);
                await boundedLocal(local.ready, budget.options());
                s.observe(peer, epoch);
                control = s
                    .nativePeer()
                    .beginControl(scope.connid, key.operationId, d.command, captured, scope.expected_revision, budget.options());
                const pending = Promise.allSettled([
                    local.control ?? Promise.resolve(),
                    control.ready,
                ]).then(() => { });
                local.control = pending;
                void pending
                    .finally(() => {
                    if (local.control === pending)
                        local.control = undefined;
                })
                    .catch(() => { });
                frames = await control.response;
                s.observe(peer, epoch);
            }
            else
                frames = await s.request(d.command, captured, { ...scope, operation_id: key.operationId }, budget.options(), key);
            const updates = frames.filter((f) => f.type === "operation.updated");
            if (d.command === "message.send") {
                const ack = frames.find((f) => f.type === "ack");
                if (!ack ||
                    ack.cid !== scope.cid ||
                    ack.sid !== undefined ||
                    ack.connid !== undefined ||
                    !object(ack.payload.details))
                    current(this).malformed(key, "uncertain");
                const accepted = messageResult(ack.payload.details, key, () => current(this).malformed(key, "uncertain"));
                if (accepted.messageId !== captured.msg_id)
                    current(this).malformed(key, "uncertain");
            }
            else if (d.scope !== "root") {
                if (updates.length !== 1 ||
                    updates[0].cid !== scope.cid ||
                    updates[0].sid !== scope.sid ||
                    updates[0].connid !== scope.connid ||
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
            if (transitionCommand(d.command)) {
                const bad = () => current(this).malformed(key, "uncertain");
                if (frames.length !== 2)
                    bad();
                transitionObservation(updates[0].payload, key, scope.expected_revision, bad);
                if (key.continuation?.kind === "consultation")
                    key = reference({
                        ...referenceJson(key),
                        continuation: {
                            ...key.continuation,
                            record_id: updates[0].payload.record_id,
                        },
                    }, k);
            }
            if (d.scope === "decision") {
                const bad = () => current(this).malformed(key, "uncertain"), update = updates[0];
                if (frames.length !== 2 ||
                    !update ||
                    update.payload.state !== "completed" ||
                    update.payload.problem !== undefined ||
                    (update.operation_id !== undefined &&
                        update.operation_id !== key.operationId) ||
                    update.expected_revision !== undefined)
                    bad();
                const result = operationDecisionResult(update.payload.result, key, bad);
                if (result.target !== captured.operation_id ||
                    result.evidenceId !== captured.evidence_id)
                    bad();
            }
            if (d.command.startsWith("conversation.participant.")) {
                const bad = () => current(this).malformed(key, "uncertain");
                if (updates[0]?.payload.state !== "completed")
                    bad();
                const result = membershipResult(updates[0].payload.result, key, bad);
                const notices = frames.filter((f) => f.type === "conversation.participant.changed");
                const action = d.command === "conversation.participant.add"
                    ? "added"
                    : d.command === "conversation.participant.update"
                        ? "updated"
                        : "removed";
                if ((captured.participant_id !== undefined &&
                    result.snapshot.id !== captured.participant_id) ||
                    (captured.identity_id !== undefined &&
                        result.snapshot.identityId !== captured.identity_id) ||
                    notices.length !== 1 ||
                    notices[0].cid !== scope.cid ||
                    notices[0].sid !== undefined ||
                    notices[0].connid !== undefined ||
                    notices[0].operation_id !== key.operationId ||
                    notices[0].expected_revision !== undefined ||
                    notices[0].payload.kind !== action ||
                    notices[0].payload.participant_id !== result.snapshot.id ||
                    notices[0].payload.membership_epoch !==
                        result.snapshot.membershipEpoch)
                    bad();
            }
            budget.options();
            // Only the submitting Operation can retire its captured local owner.
            // Restored references retain observations without physical custody.
            const completed = local &&
                scope.connid &&
                ["session.handoff", "connection.replace"].includes(d.command)
                ? (result) => {
                    if (!object(result) ||
                        !Array.isArray(result.retired_connection_ids) ||
                        !result.retired_connection_ids.includes(scope.connid) ||
                        s.peer !== peer ||
                        s.owner.status.physicalEpoch !== epoch ||
                        s.local.get(scope.connid) !== local)
                        return;
                    s.mediaPeer?.retireLocalConnection(scope.connid);
                    local.rtc?.close();
                    s.local.delete(scope.connid);
                }
                : undefined;
            const operation = new BrowserOperation(this, key, k);
            if (completed)
                operationRetirements.set(operation, completed);
            return operation;
        }
        catch (error) {
            throw translated(error, control && !control.submissionAttempted
                ? "not-submitted"
                : "uncertain", key);
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
    receiptSnapshot;
    receiptConversationRevision;
    constructor(client, conversationId, id, receiptSnapshot, receiptConversationRevision) {
        this.client = client;
        this.conversationId = conversationId;
        this.id = id;
        this.receiptSnapshot = receiptSnapshot;
        this.receiptConversationRevision = receiptConversationRevision;
        Object.freeze(this);
    }
    update(input, options) {
        fields(input, ["role", "expectedConversationRevision"], invalid);
        return this.client[submit](browserOperationKinds.participantUpdate, {
            participant_id: this.id,
            role: membershipRole(input.role, invalid),
        }, {
            cid: this.conversationId,
            expected_revision: membershipRevision(input.expectedConversationRevision, invalid),
        }, options);
    }
    remove(input, options) {
        fields(input, ["expectedConversationRevision"], invalid);
        return this.client[submit](browserOperationKinds.participantRemove, { participant_id: this.id }, {
            cid: this.conversationId,
            expected_revision: membershipRevision(input.expectedConversationRevision, invalid),
        }, options);
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
        add: (input, options) => {
            fields(input, ["identityId", "kind", "role", "expectedRevision"], invalid);
            if (!identifier(input.identityId) || !identityKind(input.kind))
                invalid();
            return this.client[submit](browserOperationKinds.participantAdd, {
                identity_id: input.identityId,
                kind: input.kind,
                role: membershipRole(input.role, invalid),
            }, {
                cid: this.id,
                expected_revision: membershipRevision(input.expectedRevision, invalid),
            }, options);
        },
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
            fields(input, ["sessionId", "to", "medium", "intent", "capabilities"], invalid);
            if (!identifier(input.sessionId))
                invalid();
            const body = sessionInviteBody(input, invalid);
            return this.client[submit](browserOperationKinds.sessionInvite, body, { cid: this.id, sid: input.sessionId }, options);
        },
    };
    constructor(client, id) {
        this.client = client;
        this.id = id;
        Object.freeze(this);
    }
    send(input, options) {
        return this.client[submit](browserOperationKinds.messageSend, messageBody(input, invalid), { cid: this.id }, options);
    }
    async receipts(messageId, options) {
        const budget = new Budget(options), s = current(this.client), peer = s.peer, epoch = s.owner.status.physicalEpoch;
        try {
            return await s.applicationReads.receipts(this.id, messageId, { ...budget.options(), timeoutMs: budget.options().timeoutMs }, epoch, () => s.observe(peer, epoch));
        }
        catch (error) {
            throw translated(error, "read-only");
        }
    }
    events(input, options) {
        new Budget(options);
        const s = current(this.client), peer = s.peer, epoch = s.owner.status.physicalEpoch;
        return createBrowserEvents(this.id, input, options, {
            check: () => s.observe(peer, epoch),
            read: (after) => s.owner.recoverConversation({
                conversationId: this.id,
                ...(after === undefined ? {} : { cursor: after }),
                limit: 1,
            }, { timeoutMs: 2000 }),
            reserve: (retire) => {
                s.observe(peer, epoch);
                if (s.eventReaders.size >= 8)
                    throw new BrowserSdkError("conflict", "read-only");
                s.eventReaders.add(retire);
                return () => s.eventReaders.delete(retire);
            },
            error: (kind) => new BrowserSdkError(kind, "read-only"),
            translated: (error) => translated(error, "read-only"),
            bad: () => s.malformed(),
        });
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
    receiptConsultationReturn;
    connections = {
        byId: (id) => {
            sdkId(id);
            return new BrowserConnection(this.client, this.conversationId, this.id, id);
        },
        offer: async (input, options) => {
            if (!input ||
                !identifier(input.connectionId) ||
                !options ||
                !identifier(options.operationId))
                invalid();
            fields(input, ["connectionId", "streams", "purpose"], invalid);
            const streams = offeredStreams(input.streams, invalid);
            if (input.purpose !== undefined &&
                (input.purpose !== "handoff-preparation" ||
                    streams.length !== 1 ||
                    streams[0].kind !== "audio" ||
                    streams[0].direction !== "sendrecv"))
                invalid();
            const low = {
                conversationId: this.conversationId,
                sessionId: this.id,
                connectionId: input.connectionId,
                operationId: options.operationId,
                purpose: input.purpose,
            };
            const seen = new Set();
            for (const stream of streams) {
                if (!stream ||
                    !identifier(stream.id) ||
                    !["audio", "video", "screen", "data"].includes(stream.kind) ||
                    !["sendrecv", "sendonly", "recvonly", "inactive"].includes(stream.direction) ||
                    !Array.isArray(stream.codecs) ||
                    !stream.codecs.length)
                    invalid();
                // The installed native browser backend currently owns these exact
                // fixed codecs/directions. Unsupported choices never downgrade.
                if (stream.direction !== "sendrecv" ||
                    stream.codecs.length !== 1 ||
                    (stream.kind === "audio"
                        ? stream.codecs[0] !== "opus"
                        : stream.kind === "data"
                            ? !["text", "json", "binary"].includes(stream.codecs[0])
                            : stream.codecs[0] !== "vp8") ||
                    (stream.kind !== "data" && seen.has(stream.kind)))
                    throw new BrowserSdkError("unsupported", "not-submitted");
                seen.add(stream.kind);
                if (stream.kind === "data")
                    (low.data ??= []).push({
                        id: stream.id,
                        protocol: stream.codecs[0],
                    });
                else
                    low[stream.kind] =
                        stream.id;
            }
            const connectionId = low.connectionId;
            const budget = new Budget(options), s = current(this.client), peer = s.peer, epoch = s.owner.status.physicalEpoch;
            const key = reference(referenceJson({
                operationId: options.operationId,
                command: "connection.offer",
                scope: {
                    conversationId: this.conversationId,
                    sessionId: this.id,
                    connectionId: input.connectionId,
                },
            }), browserOperationKinds.connectionOffer);
            if (s.local.has(connectionId) ||
                s.pending.has(connectionId) ||
                s.local.size + s.pending.size >= 8)
                throw new BrowserSdkError("conflict", "not-submitted", key);
            const reservation = {};
            s.pending.set(connectionId, reservation);
            let begun;
            try {
                if (peer.controlTransport === "websocket") {
                    if (!low.audio ||
                        low.data?.length ||
                        (low.screen && low.video))
                        throw new BrowserSdkError("unsupported", "not-submitted", key);
                    const snapshot = await s.snapshot(this.conversationId, budget.options());
                    s.observe(peer, epoch);
                    const participant = snapshot.conversation.participants.find((v) => v.identityId === s.actor && v.active);
                    const presence = participant
                        ? snapshot.appearances.get(this.id + ":" + participant.id)
                        : undefined;
                    if (!participant ||
                        !presence ||
                        presence.state !== "present" ||
                        presence.membership_epoch !==
                            participant.membershipEpoch)
                        throw new BrowserSdkError("application", "not-submitted", key);
                    begun = BrowserRtcConnection.begin(peer, low, participant.id, s.rtcConfiguration, budget.options());
                }
                else
                    begun = s
                        .nativePeer()
                        .beginOfferConnection(low, budget.options());
                const local = {
                    cid: this.conversationId,
                    sid: this.id,
                    epoch,
                    ready: begun.ready,
                    rtc: begun.owner,
                    offerOperation: key.operationId,
                    generation: 1,
                    audio: low.audio,
                    video: low.video,
                    screen: low.screen,
                };
                s.local.set(connectionId, local);
                s.cleanup.track(begun.ready);
                s.cleanup.track(begun.accepted);
                const rtc = begun.owner;
                rtc?.onClose(() => {
                    // The Connection may end before Client.close. Keep its
                    // asynchronous cleanup owned after removing the live handle.
                    s.cleanup.track(rtc.shutdown(), true);
                    if (s.local.get(connectionId) === local)
                        s.local.delete(connectionId);
                });
                void begun.ready.catch(() => {
                    if (s.local.get(connectionId) === local)
                        s.local.delete(connectionId);
                });
                await begun.accepted;
                s.observe(peer, epoch);
                budget.options();
                return new BrowserOperation(this.client, key, browserOperationKinds.connectionOffer);
            }
            catch (error) {
                throw translated(error, begun?.submissionAttempted ? "uncertain" : "not-submitted", key);
            }
            finally {
                if (s.pending.get(connectionId) === reservation)
                    s.pending.delete(connectionId);
            }
        },
    };
    constructor(client, conversationId, id, receiptSnapshot, receiptConsultationReturn) {
        this.client = client;
        this.conversationId = conversationId;
        this.id = id;
        this.receiptSnapshot = receiptSnapshot;
        this.receiptConsultationReturn = receiptConsultationReturn;
        Object.freeze(this);
    }
    async refresh(options) {
        const snapshot = await current(this.client).snapshot(this.conversationId, options), s = snapshot.sessions.get(this.id);
        if (!s)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return s;
    }
    async handoff(input, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const key = freeze({
            operationId: options.operationId,
            command: "session.handoff",
            scope: {
                conversationId: this.conversationId,
                sessionId: this.id,
                ...(identifier(input?.via) ? { connectionId: input.via } : {}),
            },
        });
        const bad = () => {
            throw new BrowserSdkError("validation", "not-submitted", key);
        };
        let intent, budget;
        try {
            if (!identifier(key.operationId))
                bad();
            intent = handoffIntent(input, bad);
            budget = new Budget(options);
            if (input.consultation !== undefined) {
                if (!(input.consultation instanceof BrowserOperation))
                    bad();
                const parent = this.client.operations.restore(input.consultation.referenceJson(), browserOperationKinds.sessionConsult), saved = parent.reference;
                if (saved.scope.conversationId !== this.conversationId ||
                    saved.scope.sessionId !== this.id ||
                    saved.operationId === key.operationId ||
                    saved.continuation?.kind !== "consultation")
                    bad();
                const record = saved.continuation.kind === "consultation"
                    ? (saved.continuation.record_id ??
                        (await parent.status(budget.options())).recordId)
                    : undefined;
                if (!identifier(record))
                    throw new BrowserSdkError("receipt-unavailable", "not-submitted", key);
                intent.body.consultation_record_id = record;
            }
            budget.options();
        }
        catch (error) {
            const failure = translated(error, "not-submitted");
            throw new BrowserSdkError(failure.kind, "not-submitted", key, failure.problem);
        }
        const scope = {
            cid: this.conversationId,
            sid: this.id,
            connid: intent.via,
            expected_revision: intent.expected,
        };
        return this.client[submit](browserOperationKinds.sessionHandoff, intent.body, scope, mutationRemainder(budget, key), scope, intent.context);
    }
    consult(input, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const body = consultBody(input, invalid), scope = {
            cid: this.conversationId,
            sid: this.id,
            connid: input.via,
            expected_revision: input.expectedTopologyRevision,
        };
        return this.client[submit](browserOperationKinds.sessionConsult, body, scope, options, scope, { kind: "consultation", retained_connection_id: input.retain });
    }
    inviteParticipants(input, options) {
        fields(input, [
            "to",
            "medium",
            "intent",
            "capabilities",
            "expectedTopologyRevision",
        ], invalid);
        const body = sessionInviteBody({ ...input, to: nonEmpty(input.to, identifier, 256, invalid) }, invalid);
        return this.client[submit](browserOperationKinds.sessionInvite, body, {
            cid: this.conversationId,
            sid: this.id,
            expected_revision: sdkRevision(input.expectedTopologyRevision),
        }, options);
    }
    update(input, expectedTopologyRevision, options) {
        return this.client[submit](browserOperationKinds.sessionUpdate, sessionUpdateBody(input, invalid), {
            cid: this.conversationId,
            sid: this.id,
            expected_revision: sdkRevision(expectedTopologyRevision),
        }, options);
    }
    accept(input, options) {
        fields(input, ["capabilities", "expectedTopologyRevision"], invalid);
        sessionCapabilities(input.capabilities, invalid);
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
                    if (s.invitationFailure)
                        throw s.invitationFailure;
                    const peer = s.peer, epoch = s.owner.status.physicalEpoch;
                    const queued = s.inbox.peek();
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
                    if (queued.epoch !== epoch) {
                        s.inbox.consume(queued);
                        continue;
                    }
                    const f = queued.frame, p = f.payload;
                    const to = p.to[0];
                    // A local wait can detach without cancelling the physical's
                    // finite read. A later wait drains that original work, then
                    // performs its own fresh authorized read; no cached delivery.
                    const previous = s.invitationRead;
                    if (previous) {
                        await boundedLocal(previous.then(() => { }, () => { }), budget.options());
                        s.observe(peer, epoch);
                        if (s.invitationFailure)
                            throw s.invitationFailure;
                        if (s.inbox.peek() !== queued)
                            continue;
                    }
                    const work = s.snapshot(f.cid, {
                        timeoutMs: 2000,
                    });
                    s.invitationRead = work;
                    const release = () => {
                        if (s.invitationRead === work)
                            s.invitationRead = undefined;
                    };
                    void work.then(release, release);
                    const snapshot = await boundedLocal(work, budget.options());
                    s.observe(peer, epoch);
                    budget.options();
                    if (s.invitationFailure)
                        throw s.invitationFailure;
                    if (!s.inbox.consume(queued))
                        continue;
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
function connectionView(d, cid, bad) {
    if (!identifier(d.connection_id) ||
        !identifier(d.session_id) ||
        !identifier(d.participant_id) ||
        !identifier(d.endpoint_id) ||
        !identifier(d.device_id) ||
        !numeric(d.membership_epoch) ||
        d.membership_epoch < 1 ||
        !numeric(d.generation) ||
        d.generation < 1 ||
        !numeric(d.revision) ||
        !(d.stream_revision === null ||
            (numeric(d.stream_revision) && d.stream_revision > 0)) ||
        !boundedText(d.state, 64) ||
        !(d.replaces === null || identifier(d.replaces)))
        return bad();
    return freeze({
        id: d.connection_id,
        conversationId: cid,
        sessionId: d.session_id,
        participantId: d.participant_id,
        generation: d.generation,
        resourceRevision: d.revision,
        ...(d.stream_revision === null
            ? {}
            : { streamRevision: d.stream_revision }),
        state: d.state,
    });
}
function streamView(d, cid, bad) {
    if (!identifier(d.stream_id) ||
        !identifier(d.session_id) ||
        !identifier(d.participant_id) ||
        !(d.connection_id === null || identifier(d.connection_id)) ||
        !["audio", "video", "screen", "data"].includes(String(d.medium)) ||
        !boundedText(d.state, 64) ||
        !numeric(d.revision))
        return bad();
    return freeze({
        id: d.stream_id,
        conversationId: cid,
        sessionId: d.session_id,
        connectionId: d.connection_id,
        participantId: d.participant_id,
        medium: d.medium,
        state: d.state,
        resourceRevision: d.revision,
        ownership: "observation",
    });
}
async function boundedLocal(work, options) {
    // A pre-aborted wait still leaves owned retirement running.
    void work.catch(() => { });
    const budget = new Budget(options);
    return new Promise((resolve, reject) => {
        let settled = false;
        const finish = (error, value) => {
            if (settled)
                return;
            settled = true;
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
            error ? reject(error) : resolve(value);
        };
        const abort = () => finish(new BrowserSdkError("cancelled", "read-only"));
        const timer = setTimeout(() => finish(new BrowserSdkError("deadline", "read-only")), budget.options().timeoutMs);
        options.signal?.addEventListener("abort", abort, { once: true });
        if (options.signal?.aborted)
            abort();
        work.then((value) => finish(undefined, value), (error) => finish(translated(error, "read-only")));
    });
}
export class BrowserConnection {
    client;
    conversationId;
    sessionId;
    id;
    receiptStreamRevision;
    /** Restart only this physical's original WebRTC peer. The operation ID,
     * complete offer and pending application stay with that owner on timeout.
     * Completion records server-observed ICE; decoded media remains separate. */
    async restartIce(expectedStreamRevision, expectedIceGeneration, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const revision = sdkRevision(expectedStreamRevision);
        const key = {
            operationId: options.operationId,
            command: "connection.update",
            scope: {
                conversationId: this.conversationId,
                sessionId: this.sessionId,
                connectionId: this.id,
            },
        };
        let rtc;
        let recorded = false;
        try {
            const { native, local, s, peer, epoch } = await this.native(options);
            if (!(native instanceof BrowserRtcConnection))
                throw new BrowserSdkError("unsupported", "not-submitted", key);
            rtc = native;
            const receipt = await rtc.restartIce(options.operationId, revision, expectedIceGeneration, options);
            recorded = true;
            s.observe(peer, epoch);
            if (s.local.get(this.id) !== local ||
                receipt.connectionGeneration !== local.generation)
                throw new BrowserSdkError("receipt-unavailable", "recorded", key);
            return receipt;
        }
        catch (error) {
            throw translated(error, recorded
                ? "recorded"
                : error instanceof ConnectError &&
                    error.code === "operation-failed"
                    ? "refused"
                    : rtc?.savedIceRestart?.submissionAttempted
                        ? "uncertain"
                        : "not-submitted", key);
        }
    }
    streams = {
        subscribe: (input, options) => {
            if (!input)
                invalid();
            return this.client[submit](browserOperationKinds.streamSubscribe, subscribeBody(input.selectors, invalid), {
                cid: this.conversationId,
                sid: this.sessionId,
                connid: this.id,
                expected_revision: sdkRevision(input.expectedStreamRevision),
            }, options);
        },
        unsubscribe: (input, options) => {
            if (!input)
                invalid();
            return this.client[submit](browserOperationKinds.streamUnsubscribe, unsubscribeBody(input.streams, invalid), {
                cid: this.conversationId,
                sid: this.sessionId,
                connid: this.id,
                expected_revision: sdkRevision(input.expectedStreamRevision),
            }, options);
        },
        byId: (id) => {
            sdkId(id);
            return new BrowserStream(this.client, this.conversationId, this.sessionId, this.id, id);
        },
        list: async (options) => {
            const snapshot = await current(this.client).snapshot(this.conversationId, options);
            const connection = snapshot.connections.get(this.id);
            if (!connection || connection.sessionId !== this.sessionId)
                throw new BrowserSdkError("receipt-unavailable", "read-only");
            return Object.freeze([...snapshot.streams.values()]
                .filter((v) => v.connectionId === this.id &&
                v.sessionId === this.sessionId)
                .map((v) => this.streams.byId(v.id)));
        },
    };
    constructor(client, conversationId, sessionId, id, receiptStreamRevision) {
        this.client = client;
        this.conversationId = conversationId;
        this.sessionId = sessionId;
        this.id = id;
        this.receiptStreamRevision = receiptStreamRevision;
        Object.freeze(this);
    }
    async refresh(options) {
        const c = (await current(this.client).snapshot(this.conversationId, options)).connections.get(this.id);
        if (!c || c.sessionId !== this.sessionId)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        const s = current(this.client), local = s.local.get(this.id);
        if (local &&
            local.cid === this.conversationId &&
            local.sid === this.sessionId &&
            local.epoch === s.owner.status.physicalEpoch &&
            local.generation === c.generation) {
            let revision = Math.max(c.streamRevision ?? 0, local.revision ?? 0);
            if (s.mediaPeer)
                try {
                    revision = Math.max(revision, s.mediaPeer.revision(this.id));
                }
                catch {
                    /* Retired or not installed locally. */
                }
            if (revision > 0)
                return freeze({
                    ...c,
                    streamRevision: revision,
                });
        }
        return c;
    }
    update(input, expectedStreamRevision, options) {
        return this.client[submit](browserOperationKinds.connectionUpdate, updateBody(input, invalid), {
            cid: this.conversationId,
            sid: this.sessionId,
            connid: this.id,
            expected_revision: sdkRevision(expectedStreamRevision),
        }, options);
    }
    replaceDevice(input, options) {
        fields(options, ["timeoutMs", "signal", "operationId"], invalid);
        const intent = moveIntent(input, this.id, invalid), scope = {
            cid: this.conversationId,
            sid: this.sessionId,
            connid: this.id,
            expected_revision: intent.expected,
        };
        return this.client[submit](browserOperationKinds.connectionReplace, intent.body, scope, options, scope, intent.context);
    }
    end(input, options) {
        if (!input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            !boundedText(input.reason, 256, true))
            invalid();
        return this.client[submit](browserOperationKinds.connectionEnd, { reason_code: input.reasonCode, reason: input.reason }, {
            cid: this.conversationId,
            sid: this.sessionId,
            connid: this.id,
            ...(input.expectedStreamRevision === undefined
                ? {}
                : {
                    expected_revision: sdkRevision(input.expectedStreamRevision),
                }),
        }, options);
    }
    async native(options) {
        const s = current(this.client), peer = s.peer, epoch = s.owner.status.physicalEpoch, local = s.local.get(this.id);
        if (!local ||
            local.cid !== this.conversationId ||
            local.sid !== this.sessionId ||
            local.epoch !== epoch)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        const budget = new Budget(options);
        const native = await boundedLocal(local.ready, budget.options());
        while (local.control)
            await boundedLocal(local.control, budget.options());
        s.observe(peer, epoch);
        if (s.local.get(this.id) !== local)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return { native, local, s, peer, epoch };
    }
    /** Local capture is explicit and owns only this physical's negotiated senders. */
    async capture(input, options) {
        if (!input ||
            (input.audio !== undefined && typeof input.audio !== "boolean") ||
            (input.video !== undefined &&
                !["camera", "screen"].includes(input.video)) ||
            (!input.audio && !input.video))
            invalid();
        const captured = { ...input }, budget = new Budget(options), { native, local, s, peer, epoch } = await this.native(budget.options());
        const audio = captured.audio ? local.audio : undefined, video = captured.video === undefined
            ? undefined
            : captured.video === "screen"
                ? local.screen
                : local.video;
        if ((captured.audio && !audio) || (captured.video && !video))
            invalid();
        if (s.captures.has(this.id))
            throw new BrowserSdkError("conflict", "read-only");
        const reservation = {};
        s.captures.set(this.id, reservation);
        const release = () => {
            if (s.captures.get(this.id) === reservation)
                s.captures.delete(this.id);
        };
        let capture;
        try {
            const state = (state, error) => {
                if (state === "closed")
                    release();
                captured.state?.(state, error ? translated(error, "read-only") : undefined);
            };
            capture = await s.cleanup.track(native instanceof BrowserRtcConnection
                ? native.capture({ ...captured, state }, budget.options())
                : BrowserMediaCapture.open(native, {
                    audioStreamId: audio,
                    videoStreamId: video,
                    videoSource: captured.video,
                    audioDeviceId: captured.audioDeviceId,
                    videoDeviceId: captured.videoDeviceId,
                    stream: captured.stream,
                    state,
                    ...budget.options(),
                }));
            s.observe(peer, epoch);
            const owner = capture;
            return Object.freeze({
                get closed() {
                    return owner.closed;
                },
                pause: () => owner.pause(),
                resume: async () => {
                    try {
                        await owner.resume();
                    }
                    catch (error) {
                        throw translated(error, "read-only");
                    }
                },
                close: () => owner.close(),
            });
        }
        catch (error) {
            release();
            capture?.close();
            throw translated(error, "read-only");
        }
    }
    /** Decoded frames route to the owned renderer without protocol callbacks in app code. */
    async render(input, options) {
        if (!input ||
            (input.audio !== undefined && typeof input.audio !== "boolean") ||
            (input.video !== undefined &&
                (!Array.isArray(input.video) || input.video.length > 8)))
            invalid();
        const video = {};
        for (const v of input.video ?? []) {
            if (!v ||
                !(v.stream instanceof BrowserStream) ||
                !v.stream.belongs(this.client, this.conversationId, this.sessionId, this.id) ||
                Object.hasOwn(video, v.stream.id))
                invalid();
            video[v.stream.id] = v.canvas;
        }
        const audio = input.audio, volume = input.volume, budget = new Budget(options), { native, s, peer, epoch } = await this.native(budget.options());
        if (s.renderers.has(this.id))
            throw new BrowserSdkError("conflict", "read-only");
        let renderer;
        try {
            renderer = await s.cleanup.track(native instanceof BrowserRtcConnection
                ? native.render({ audio, video, volume }, budget.options())
                : BrowserMediaRenderer.open(native, {
                    audio,
                    video,
                    volume,
                    ...budget.options(),
                }));
            s.observe(peer, epoch);
            if (s.renderers.has(this.id))
                throw new BrowserSdkError("conflict", "read-only");
            s.renderers.set(this.id, renderer);
            const owner = renderer;
            void renderer.finished
                .finally(() => {
                if (s.renderers.get(this.id) === owner)
                    s.renderers.delete(this.id);
            })
                .catch(() => { });
            const finished = renderer.finished.catch((error) => {
                throw translated(error, "read-only");
            });
            void finished.catch(() => { });
            return Object.freeze({
                finished,
                get diagnostics() {
                    const v = owner.diagnostics;
                    return Object.freeze({
                        closed: v.closed,
                        paused: v.paused,
                        audio: v.audio,
                        video: v.video,
                    });
                },
                pause: () => owner.pause(),
                resume: async () => {
                    try {
                        await owner.resume();
                    }
                    catch (error) {
                        throw translated(error, "read-only");
                    }
                },
                close: () => owner.close(),
            });
        }
        catch (error) {
            renderer?.close();
            throw translated(error, "read-only");
        }
    }
}
export class BrowserStream {
    client;
    conversationId;
    sessionId;
    connectionId;
    id;
    constructor(client, conversationId, sessionId, connectionId, id) {
        this.client = client;
        this.conversationId = conversationId;
        this.sessionId = sessionId;
        this.connectionId = connectionId;
        this.id = id;
        Object.freeze(this);
    }
    /** @internal Only validates handle parentage, never current authority. */
    belongs(client, cid, sid, connid) {
        return (this.client === client &&
            this.conversationId === cid &&
            this.sessionId === sid &&
            this.connectionId === connid);
    }
    async refresh(options) {
        const v = (await current(this.client).snapshot(this.conversationId, options)).streams.get(JSON.stringify([this.sessionId, this.connectionId, this.id]));
        if (!v ||
            v.sessionId !== this.sessionId ||
            v.connectionId !== this.connectionId)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return v;
    }
}
