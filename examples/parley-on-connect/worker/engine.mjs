import { contextVersion, proposal, intentFor, approve, digest } from "./policy.mjs";
import { stateJson } from "./state.mjs";

const fail = code => { const error = Error(code); error.code = code; throw error; };
const copy = value => structuredClone(value);
const waiting = new Set(["speech-in-progress", "session-still-active", "approval-required"]);
const terminal = new Set(["completed", "failed", "cancelled", "expired"]);
export function initialPlan({ planId, planVersion, stage }) {
    contextVersion({ conversationId: "validation", planId, planVersion, stage, speech: null, approvers: [] });
    return { profile: "parley-worker-engine/1", planId, planVersion, stage,
        cursor: null, trigger: null, model: null, intent: null, pending: null,
        slots: {}, seen: [], proposals: 0, attention: null };
}

/** Adapters are trusted application code. Model/event data never supply an
 * adapter, approval reader, resource binding or stage transition. A pending
 * external call is persisted BEFORE invocation and can only be observed after
 * uncertainty/restart. No automatic resubmission or second model request. */
export class DecisionWorker {
    #state; #sdk; #app; #model; #bindings; #queue = Promise.resolve();
    constructor({ state, sdk, app, model, bindings }) {
        this.#state = state; this.#sdk = sdk; this.#app = app; this.#model = model;
        // Copy configuration so callers cannot swap a gate during an await.
        this.#bindings = Object.freeze(Object.fromEntries(Object.entries(bindings)
            .map(([name, value]) => [name, Object.freeze({ ...value,
                scope: Object.freeze(copy(value.scope)),
                ...(value.approval ? { approval: Object.freeze(copy(value.approval)) } : {}) })])));
        if (this.snapshot().profile !== "parley-worker-engine/1") fail("different-worker-engine");
    }
    snapshot() { return this.#state.read().value; }
    #save(value) { this.#state.write(this.#state.read().revision, value); }
    #serial(work) {
        const next = this.#queue.then(work);
        this.#queue = next.catch(() => {});
        return next;
    }
    async #current(plan) {
        const current = await this.#app.context(copy(plan));
        contextVersion(current);
        if (current.conversationId !== this.#state.scope.conversationId ||
            current.planId !== plan.planId || current.planVersion !== plan.planVersion ||
            current.stage !== plan.stage) fail("different-business-context");
        return current;
    }
    /** Called by the authenticated app, never by model output. Validate its
     * reviewed stage graph/evidence before changing business state. */
    advance(next, evidence) {
        return this.#serial(async () => {
            const plan = this.snapshot();
            if (plan.trigger || plan.pending || plan.attention) fail("worker-needs-recovery");
            const current = await this.#current(plan);
            if (await this.#app.allowAdvance(copy(plan), next, copy(evidence), current) !== true)
                fail("business-transition-refused");
            const checked = initialPlan({ planId: plan.planId, planVersion: plan.planVersion, stage: next });
            plan.stage = checked.stage;
            this.#save(plan);
        });
    }
    /** Stream is created by the public SDK, and ready() establishes an
     * authorized bootstrap cursor without consuming an event. */
    async run(stream, { signal, maximumEvents = 256 } = {}) {
        if (!Number.isInteger(maximumEvents) || maximumEvents < 1 || maximumEvents > 256)
            fail("invalid-event-budget");
        let count = 0;
        try {
            await this.#serial(async () => {
                const plan = this.snapshot();
                const cursor = await stream.ready();
                if (plan.cursor === null) { plan.cursor = cursor; this.#save(plan); }
            });
            await this.wake();
            if (this.snapshot().trigger || this.snapshot().attention) return;
            while (!signal?.aborted && count++ < maximumEvents) {
                const next = await stream.next();
                if (next.done) break;
                await this.accept(next.value);
                const current = this.snapshot();
                if (current.trigger || current.attention) break;
            }
        } finally { await stream.close(); }
    }
    accept(event) {
        return this.#serial(async () => {
            const plan = this.snapshot();
            if (plan.trigger || plan.pending) return this.#drive();
            if (plan.attention) return copy(plan);
            if (event.conversationId !== this.#state.scope.conversationId ||
                typeof event.eventId !== "string" || !/^[A-Za-z0-9._~-]{1,128}$/.test(event.eventId) ||
                typeof event.cursor !== "string" || !event.cursor || event.cursor.length > 2048 ||
                plan.cursor === null) fail("invalid-event-reference");
            if (plan.seen.includes(event.eventId)) return copy(plan);
            // Only a reference is retained. Re-read authorized content even on
            // first handling; a cached body never renews authority after replay.
            plan.trigger = { eventId: event.eventId, cursor: event.cursor,
                previousCursor: plan.cursor, conversationId: event.conversationId };
            plan.cursor = event.cursor;
            this.#save(plan);
            return this.#drive();
        });
    }
    /** Re-evaluate original proposal after speech/approval; or observe an
     * original pending operation/model request. Never regenerate its identity. */
    wake() { return this.#serial(() => this.#drive()); }
    async #drive() {
        const plan = this.snapshot();
        if (!plan.trigger || plan.attention) return copy(plan);
        try {
            // Every replay needs a fresh authorized read. Failure retains the
            // original reference and stops before model/effect/status access.
            const event = await this.#sdk.readEvent(copy(plan.trigger));
            if (!event || event.eventId !== plan.trigger.eventId ||
                event.cursor !== plan.trigger.cursor ||
                event.conversationId !== this.#state.scope.conversationId)
                fail("event-no-longer-visible");
            if (plan.pending) {
                const observed = await this.#sdk.observe(copy(plan.pending.reference), copy(plan.intent));
                return this.#observed(plan, observed);
            }
            let current = await this.#current(plan);
            if (!plan.model) {
                if (plan.proposals >= 32) fail("proposal-budget-exhausted");
                const actions = Object.keys(this.#bindings).filter(name => this.#bindings[name].stage === plan.stage);
                if (!actions.length) fail("no-stage-actions");
                const input = { profile: "parley-model-proposal/1", planId: plan.planId,
                    planVersion: plan.planVersion, stage: plan.stage,
                    context: current, actions, untrustedEvent: event };
                if (Buffer.byteLength(stateJson(input)) > 32768) fail("model-context-capacity");
                plan.model = { requestId: "model-" + digest({ workerId: this.#state.scope.workerId,
                    eventId: event.eventId, planId: plan.planId, planVersion: plan.planVersion,
                    stage: plan.stage }).slice(0, 48), input, output: null };
                plan.proposals++;
                this.#save(plan);
                // A throw leaves the saved model request pending. Wake/restart
                // uses recover(), never generate() a second time.
                const output = await this.#model.generate(copy(plan.model));
                const fresh = await this.#current(plan);
                if (contextVersion(current) !== contextVersion(fresh)) fail("stale-model-context");
                plan.model.output = proposal(output, this.#bindings);
                this.#save(plan);
                current = fresh;
            } else if (plan.model.output === null) {
                const recovered = await this.#model.recover(copy(plan.model));
                if (recovered.state !== "completed") return copy(plan);
                if (contextVersion(current) !== contextVersion(plan.model.input.context)) fail("stale-model-context");
                plan.model.output = proposal(recovered.output, this.#bindings);
                this.#save(plan);
            }
            const selected = plan.model.output, binding = this.#bindings[selected.action];
            if (!binding || binding.stage !== plan.stage) fail("invalid-stage-action");
            if (!plan.intent) {
                const intent = intentFor({ workerId: this.#state.scope.workerId, eventId: event.eventId,
                    context: current, selected, binding });
                if (Object.hasOwn(plan.slots, intent.operationId)) return this.#finish(plan);
                plan.intent = intent;
                this.#save(plan);
            }
            const decision = binding.approval ? await this.#app.decision(copy(plan.intent)) : null;
            current = await this.#current(plan);
            approve(plan.intent, current, binding, decision);
            // prepare() is pure: validate SDK reference/inputs before recording
            // uncertain submission. It must not issue any network mutation.
            const reference = this.#sdk.prepare(copy(plan.intent));
            plan.pending = { reference };
            this.#save(plan);
            const operation = await this.#sdk.submit(copy(plan.intent));
            // Match the original reference before using any returned status.
            if (!this.#sdk.sameReference(reference, operation.reference)) fail("different-original-operation");
            plan.pending.reference = copy(operation.reference);
            this.#save(plan);
            return this.#observed(plan, await this.#sdk.observe(copy(plan.pending.reference), copy(plan.intent)));
        } catch (error) {
            // Exceptions during external calls preserve original pending state.
            // Only known policy failures become terminal attention. Gate waits
            // retain the existing model answer and do not spend another turn.
            const code = error.code;
            if (waiting.has(code)) { plan.waiting = code; this.#save(plan); }
            else if (typeof code === "string" && !plan.pending && !["authority-busy", "timeout", "unavailable"].includes(code)) {
                plan.attention = code; this.#save(plan);
            }
            return copy(plan);
        }
    }
    #observed(plan, observed) {
        if (!observed || typeof observed.state !== "string") fail("invalid-operation-observation");
        if (!terminal.has(observed.state)) return copy(plan);
        if (Object.keys(plan.slots).length >= 128) fail("business-slot-capacity");
        plan.slots[plan.intent.operationId] = { state: observed.state,
            intentDigest: digest(plan.intent), reference: copy(plan.pending.reference) };
        // completed means only the SDK operation's own boundary. Stage/media/
        // delivery success requires independent application evidence.
        if (observed.state !== "completed") {
            plan.attention = "original-operation-" + observed.state;
            this.#save(plan); return copy(plan);
        }
        return this.#finish(plan);
    }
    #finish(plan) {
        plan.seen = [...plan.seen, plan.trigger.eventId].slice(-256);
        plan.trigger = null; plan.model = null; plan.intent = null; plan.pending = null;
        delete plan.waiting;
        this.#save(plan); return copy(plan);
    }
}
