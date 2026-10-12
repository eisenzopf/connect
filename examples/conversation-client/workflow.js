// Public-client workflows. Persist intent before the network; never persist credentials.
export class IntentJournal {
    constructor(storage, namespace) {
        this.storage = storage;
        this.key = `connect-intents:${namespace}`;
    }
    read() { return JSON.parse(this.storage.getItem(this.key) || "{}"); }
    write(value) { this.storage.setItem(this.key, JSON.stringify(value)); }
    forget(slot) { const all = this.read(); delete all[slot]; this.write(all); }
    progress(slot) { return Number(this.storage.getItem(`${this.key}:progress:${slot}`) || 0); }
    saveProgress(slot, offset) { this.storage.setItem(`${this.key}:progress:${slot}`, String(offset)); }
    async run(client, slot, input) {
        const all = this.read();
        let intent = all[slot];
        const firstSubmission = !intent;
        if (intent && JSON.stringify(intent.input) !== JSON.stringify(input))
            throw new Error("An earlier action is unresolved. Retry its saved inputs before starting different work.");
        if (!intent) {
            intent = { input, operation: { ...input, request_id: crypto.randomUUID() } };
            all[slot] = intent;
            this.write(all); // Failing to save stops submission.
        }
        if (intent.result) return intent.result;
        let result;
        try { result = await client.request(intent.operation); }
        catch (error) {
            // A first-attempt, definite application rejection has no uncertain
            // prior effect. After any uncertainty retain the original identity,
            // even if a later attempt is denied by current authorization.
            if (firstSubmission && ["invalid-request", "forbidden", "unauthorized", "conversation-closed", "capacity-exceeded", "unsupported-operation"].includes(error.code))
                this.forget(slot);
            throw error;
        }
        const latest = this.read();
        latest[slot] = { ...intent, result };
        this.write(latest);
        return result;
    }
    async retry(client) {
        const entry = Object.entries(this.read()).find(([, value]) => !value.result);
        if (!entry) return null;
        const [slot, intent] = entry;
        return { slot, result: await this.run(client, slot, intent.input) };
    }
}

/** Explicit history cursor paging; bounded per page and cancellable between pages. */
export async function historyPage(client, conversation, cursor = null) {
    return client.history(conversation, cursor, 100);
}

/** Match the selected conversation and participant before acquiring microphone tracks. */
export async function configuredVoiceBinding(client, conversation, participant) {
    const binding = await client.browserBinding();
    if (binding.conversation_id !== conversation || binding.participant_id !== participant)
        throw new Error(`Voice is configured for conversation ${binding.conversation_id} and participant ${binding.participant_id}. Select that binding before joining.`);
    return binding;
}
