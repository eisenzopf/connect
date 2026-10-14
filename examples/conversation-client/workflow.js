// Public-client workflows. Persist intent before the network; never persist credentials.
export class IntentJournal {
    constructor(storage, namespace) {
        this.storage = storage;
        this.key = `connect-intents:${namespace}`;
    }
    read() {
        return JSON.parse(this.storage.getItem(this.key) || "{}");
    }
    write(value) {
        this.storage.setItem(this.key, JSON.stringify(value));
    }
    forget(slot) {
        const all = this.read();
        delete all[slot];
        this.write(all);
    }
    progress(slot) {
        return Number(
            this.storage.getItem(`${this.key}:progress:${slot}`) || 0,
        );
    }
    saveProgress(slot, offset) {
        this.storage.setItem(`${this.key}:progress:${slot}`, String(offset));
    }
    async run(client, slot, input) {
        const all = this.read();
        let intent = all[slot];
        const firstSubmission = !intent;
        if (intent && JSON.stringify(intent.input) !== JSON.stringify(input))
            throw new Error(
                "An earlier action is unresolved. Retry its saved inputs before starting different work.",
            );
        if (!intent) {
            intent = {
                input,
                operation: { ...input, request_id: crypto.randomUUID() },
            };
            all[slot] = intent;
            this.write(all); // Failing to save stops submission.
        }
        if (intent.result) return intent.result;
        let result;
        try {
            result = await client.request(intent.operation);
        } catch (error) {
            // A first-attempt, definite application rejection has no uncertain
            // prior effect. After any uncertainty retain the original identity,
            // even if a later attempt is denied by current authorization.
            if (
                firstSubmission &&
                [
                    "invalid-request",
                    "forbidden",
                    "unauthorized",
                    "conversation-closed",
                    "capacity-exceeded",
                    "unsupported-operation",
                ].includes(error.code)
            )
                this.forget(slot);
            throw error;
        }
        const latest = this.read();
        latest[slot] = { ...intent, result };
        this.write(latest);
        return result;
    }
    /** Retain only identity/hash for pairing; never store its challenge or secret result. */
    async runPairing(client, slot, input) {
        const metadata = { ...input };
        if (input.challenge !== undefined) {
            const digest = await crypto.subtle.digest(
                "SHA-256",
                new TextEncoder().encode(input.challenge),
            );
            delete metadata.challenge;
            metadata.challenge_hash = Array.from(new Uint8Array(digest))
                .map((v) => v.toString(16).padStart(2, "0"))
                .join("");
        }
        const all = this.read();
        let intent = all[slot];
        if (intent && JSON.stringify(intent.input) !== JSON.stringify(metadata))
            throw new Error(
                "Re-enter the original pairing inputs to resolve the saved action.",
            );
        if (!intent) {
            intent = {
                input: metadata,
                request_id: crypto.randomUUID(),
                private_pairing: true,
            };
            all[slot] = intent;
            this.write(all);
        }
        const result = await client.request({
            ...input,
            request_id: intent.request_id,
        });
        this.forget(slot);
        return result;
    }
    async retry(client) {
        const entry = Object.entries(this.read()).find(
            ([, value]) => !value.result && !value.private_pairing,
        );
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
export async function configuredVoiceBinding(
    client,
    conversation,
    participant,
    session,
) {
    const binding = await client.browserBinding(
        session ? conversation : undefined,
        session,
    );
    if (
        binding.conversation_id !== conversation ||
        binding.participant_id !== participant ||
        (session !== undefined && binding.session_id !== session)
    )
        throw new Error(
            `Voice is configured for conversation ${binding.conversation_id} and participant ${binding.participant_id}. Select that binding before joining.`,
        );
    return binding;
}

/** Read bounded chunks with current server authority, then independently verify bytes. */
export async function readArtifactBytes(client, conversation, id) {
    let artifact,
        bytes,
        offset = 0;
    const deadline = performance.now() + 60000;
    for (;;) {
        if (performance.now() >= deadline)
            throw new Error(
                "File download exceeded the local 60-second budget.",
            );
        const chunk = await client.artifactGet(conversation, id, offset, 32768);
        const current = chunk.artifact;
        if (
            current.id !== id ||
            current.conversation_id !== conversation ||
            current.state !== "ready" ||
            !Number.isSafeInteger(current.size) ||
            current.size < 0 ||
            current.size > 16 * 1024 * 1024 ||
            chunk.offset !== offset ||
            typeof chunk.eof !== "boolean" ||
            typeof chunk.data !== "string" ||
            chunk.data.length > 43692
        )
            throw new Error("Invalid file response.");
        if (!artifact) {
            artifact = current;
            bytes = new Uint8Array(current.size);
        } else if (
            current.size !== artifact.size ||
            current.sha256 !== artifact.sha256
        )
            throw new Error("File metadata changed during download.");
        const data = Uint8Array.from(atob(chunk.data), (c) => c.charCodeAt(0));
        if (
            data.length !== Math.min(32768, bytes.length - offset) ||
            chunk.eof !== (offset + data.length === bytes.length)
        )
            throw new Error("Invalid file chunk.");
        bytes.set(data, offset);
        offset += data.length;
        if (!chunk.eof) continue;
        if (offset !== bytes.length)
            throw new Error("File ended before all declared bytes arrived.");
        const hash = Array.from(
            new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
        )
            .map((b) => b.toString(16).padStart(2, "0"))
            .join("");
        if (hash !== artifact.sha256)
            throw new Error("File integrity verification failed.");
        return { artifact, bytes };
    }
}
