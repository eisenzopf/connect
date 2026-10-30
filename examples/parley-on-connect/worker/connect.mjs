// Public NodeClient adapter. No HTTP/store/Core/provider fallback exists here.
import { digest } from "./policy.mjs";
const fail = code => { const error = Error(code); error.code = code; throw error; };
const kinds = Object.freeze({ "message.send": "message_send", "session.invite": "session_invite",
    "session.end": "session_end", "session.consult": "session_consult",
    "session.consult.return": "session_consult_return", "session.handoff": "session_handoff",
    "connection.replace": "connection_replace" });
const scopeJson = scope => Object.fromEntries(Object.entries({ conversation_id: scope.conversationId,
    session_id: scope.sessionId, connection_id: scope.connectionId }).filter(([, value]) => value !== undefined));
export class ConnectWorkerGateway {
    #client; #cid; #timeout;
    constructor({ client, conversationId, timeoutMs = 5000 }) {
        if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 10000)
            fail("invalid-sdk-budget");
        this.#client = client; this.#cid = conversationId; this.#timeout = timeoutMs;
    }
    #options() { return { timeoutMs: this.#timeout }; }
    #conversation() { return this.#client.conversations.byId(this.#cid); }
    #kind(command) {
        if (!Object.hasOwn(kinds, command)) fail("unsupported-worker-command");
        return this.#client.operationKinds[kinds[command]];
    }
    events(after, signal) { return this.#conversation().events(after === null ? {} : { after },
        { ...this.#options(), ...(signal ? { signal } : {}) }); }
    async readEvent(reference) {
        if (reference.conversationId !== this.#cid) fail("different-event-scope");
        const stream = this.events(reference.previousCursor, AbortSignal.timeout(this.#timeout));
        try {
            await stream.ready();
            for (let count = 0; count < 256; count++) {
                const next = await stream.next();
                if (next.done) return null;
                if (next.value.eventId === reference.eventId) {
                    if (next.value.cursor !== reference.cursor) fail("different-event-cursor");
                    return next.value;
                }
                if (next.value.cursor === reference.cursor) return null;
            }
            fail("event-replay-budget-exhausted");
        } finally { await stream.close(); }
    }
    /** Only pure handle/reference validation; operations.restore performs no IO. */
    prepare(intent) {
        if (intent.scope.conversationId !== this.#cid) fail("different-action-scope");
        const input = intent.input;
        const saved = { schema: "connect-operation-reference/1", operation_id: intent.operationId,
            command: intent.command, scope: scopeJson(intent.scope) };
        switch (intent.command) {
            case "session.invite":
                if (intent.inviteKind === "new" ? input.sessionId !== intent.scope.sessionId : intent.inviteKind !== "participants")
                    fail("different-invitation-scope");
                break;
            case "session.consult":
                if (input.via !== intent.scope.connectionId) fail("different-controller-scope");
                saved.continuation = { kind: "consultation", retained_connection_id: input.retain };
                break;
            case "session.consult.return": {
                const original = input.consultation;
                if (original?.command !== "session.consult" || digest(original.scope) !== digest(saved.scope) ||
                    !original.continuation?.record_id) fail("missing-original-consultation");
                this.#client.operations.restore(original, this.#kind("session.consult"));
                saved.continuation = { kind: "consultation-return",
                    retained_connection_id: original.continuation.retained_connection_id,
                    consultation_record_id: original.continuation.record_id };
                break;
            }
            case "session.handoff":
                if (input.via !== intent.scope.connectionId) fail("different-controller-scope");
                saved.continuation = { kind: "handoff", retained_connection_id: input.retain,
                    source_participant: input.from, target_participant: input.to,
                    candidate: input.candidate ?? null, role: input.role.scope === "session" ? { scope: "session" } :
                        { scope: "conversation", expected_conversation_revision: input.role.expectedConversationRevision },
                    outgoing: input.outgoing };
                break;
            case "connection.replace":
                saved.continuation = { kind: "replacement", retained_connection_id: input.retain,
                    target: input.target.endpoint === undefined ? { kind: "candidate", id: input.target.candidate } :
                        { kind: "endpoint", id: input.target.endpoint } };
                break;
        }
        this.#client.operations.restore(saved, this.#kind(intent.command));
        return saved;
    }
    sameReference(original, returned) {
        const normalized = structuredClone(returned);
        if (original.command === "session.consult" && original.continuation?.record_id === undefined &&
            normalized.continuation?.kind === "consultation") delete normalized.continuation.record_id;
        return digest(original) === digest(normalized);
    }
    async submit(intent) {
        this.prepare(intent);
        const conversation = this.#conversation(), input = structuredClone(intent.input);
        const options = { ...this.#options(), operationId: intent.operationId };
        const session = () => conversation.sessions.byId(intent.scope.sessionId);
        let operation;
        switch (intent.command) {
            case "message.send": operation = await conversation.send(input, options); break;
            case "session.invite": operation = await (intent.inviteKind === "new" ?
                conversation.sessions.invite(input, options) : session().inviteParticipants(input, options)); break;
            case "session.end": operation = await session().end(input, options); break;
            case "session.consult": operation = await session().consult(input, options); break;
            case "session.consult.return": {
                const original = this.#client.operations.restore(input.consultation, this.#kind("session.consult"));
                operation = await original.returnToRetained({ expectedTopologyRevision: input.expectedTopologyRevision }, options);
                break;
            }
            case "session.handoff":
                if (input.consultation) input.consultation = this.#client.operations.restore(input.consultation, this.#kind("session.consult"));
                operation = await session().handoff(input, options); break;
            case "connection.replace": operation = await session().connections.byId(intent.scope.connectionId).replaceDevice(input, options); break;
            default: fail("unsupported-worker-command");
        }
        return { reference: operation.referenceJson() };
    }
    async observe(reference, intent) {
        if (!this.sameReference(this.prepare(intent), reference)) fail("different-original-operation");
        return this.#client.operations.restore(reference, this.#kind(intent.command)).status(this.#options());
    }
    /** Refreshes current public facts. Speech completion and business plan are
     * supplied by the trusted app; journal bodies never impersonate them. */
    async context(plan, { sessionId, speech, approverIds }) {
        const conversation = await this.#conversation().refresh(this.#options());
        if (conversation.closed) fail("conversation-closed");
        const session = sessionId === undefined ? null : await this.#conversation().sessions.byId(sessionId).refresh(this.#options());
        return { conversationId: this.#cid, stage: plan.stage, planId: plan.planId, planVersion: plan.planVersion,
            ...(session ? { sessionId: session.id, sessionRevision: session.topologyRevision, sessionState: session.state } : {}),
            speech: structuredClone(speech), approvers: conversation.participants.filter(p => approverIds.includes(p.id))
                .map(p => ({ participantId: p.id, membershipEpoch: p.membershipEpoch, active: p.active })) };
    }
}
