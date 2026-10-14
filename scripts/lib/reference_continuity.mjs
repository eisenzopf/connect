/** Independent public-client observation of a source-backed decision across AV/restart. */
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

async function readArtifact(client, conversation, expected) {
    const chunks = [];
    let offset = 0;
    for (let page = 0; page < 64; page++) {
        const chunk = await client.artifactGet(
            conversation,
            expected.id,
            offset,
            32768,
        );
        assert.equal(chunk.offset, offset);
        assert.equal(chunk.artifact.id, expected.id);
        assert.equal(chunk.artifact.conversation_id, conversation);
        assert.equal(chunk.artifact.owner_id, expected.owner_id);
        assert.equal(chunk.artifact.size, expected.size);
        assert.equal(chunk.artifact.sha256, expected.sha256);
        assert.equal(chunk.artifact.state, "ready");
        const bytes = Buffer.from(chunk.data, "base64");
        assert(bytes.length > 0 && bytes.length <= 32768);
        chunks.push(bytes);
        offset += bytes.length;
        assert(offset <= expected.size);
        if (chunk.eof) {
            const contents = Buffer.concat(chunks);
            assert.equal(contents.length, expected.size);
            assert.equal(hash(contents), expected.sha256);
            return contents;
        }
    }
    throw Error("Continuity artifact exceeded bounded observer pagination");
}

export class ReferenceContinuity {
    constructor(client, observer, conversation, owner, reviewer) {
        Object.assign(this, {
            client,
            observer,
            conversation,
            owner,
            reviewer,
        });
    }
    async establish() {
        this.brief = {
            project: "Community launch",
            requested_color: "violet",
            deadline: "Friday",
            reference_code: randomUUID(),
        };
        const bytes = Buffer.from(JSON.stringify(this.brief));
        const pending = await this.client.artifactBegin(
            "continuity-brief-begin",
            this.conversation,
            "launch-brief.json",
            "application/json",
            bytes.length,
            hash(bytes),
        );
        await this.client.artifactAppend(
            "continuity-brief-append",
            this.conversation,
            pending.id,
            0,
            bytes.toString("base64"),
        );
        this.artifact = await this.client.artifactFinish(
            "continuity-brief-finish",
            this.conversation,
            pending.id,
        );
        assert.equal(this.artifact.owner_id, this.owner);
        const observed = JSON.parse(
            await readArtifact(this.observer, this.conversation, this.artifact),
        );
        assert.deepEqual(observed, this.brief);
        this.fact = await this.client.send(
            "continuity-fact",
            this.conversation,
            JSON.stringify({
                action: "review_launch_brief",
                artifact_id: this.artifact.id,
                artifact_sha256: this.artifact.sha256,
                question:
                    "Review the attached launch brief in our video session and record your decision.",
            }),
        );
        const factSource = (
            await this.observer.contextView(this.conversation, 100)
        ).items.find((item) => item.source_id === this.fact.message_id);
        assert(factSource && factSource.actor_id === this.owner);
    }
    async decide(sessionId) {
        const session = await this.observer.sessionGet(
            this.conversation,
            sessionId,
        );
        assert(["accepted", "ready"].includes(session.state));
        assert.deepEqual(session.media, ["audio", "video"]);
        const observed = JSON.parse(
            await readArtifact(this.observer, this.conversation, this.artifact),
        );
        this.sessionId = sessionId;
        this.decisionBody = JSON.stringify({
            action: "launch_decision",
            decision: "approved",
            conversation_id: this.conversation,
            session_id: sessionId,
            source_message_id: this.fact.message_id,
            artifact_id: this.artifact.id,
            artifact_sha256: this.artifact.sha256,
            project: observed.project,
            color: observed.requested_color,
            deadline: observed.deadline,
            reference_code: observed.reference_code,
        });
        this.decision = await this.observer.send(
            "continuity-decision",
            this.conversation,
            this.decisionBody,
        );
        const decisionSource = (
            await this.client.contextView(this.conversation, 100)
        ).items.find((item) => item.source_id === this.decision.message_id);
        assert(decisionSource && decisionSource.actor_id === this.reviewer);
        assert.equal(decisionSource.body, this.decisionBody);
    }
    async afterRestart(otherConversation) {
        const retained = await this.client.contextView(this.conversation, 100);
        assert(Number.isInteger(retained.omitted) && retained.omitted > 0);
        const fact = retained.items.find(
            (item) => item.source_id === this.fact.message_id,
        );
        const decision = retained.items.find(
            (item) => item.source_id === this.decision.message_id,
        );
        assert(
            fact &&
                fact.actor_id === this.owner &&
                Number.isInteger(fact.sequence),
        );
        assert(
            decision &&
                decision.actor_id === this.reviewer &&
                Number.isInteger(decision.sequence),
        );
        assert.equal(decision.body, this.decisionBody);
        assert(decision.sequence > fact.sequence);
        const outcome = JSON.parse(decision.body);
        const artifactSource = retained.items.find(
            (item) =>
                item.kind === "artifact_ready" &&
                item.source_id === outcome.artifact_id,
        );
        const sessionSource = retained.items.find(
            (item) =>
                item.kind === "session_ended" &&
                item.source_id === outcome.session_id,
        );
        assert(artifactSource && artifactSource.actor_id === this.owner);
        assert(
            sessionSource &&
                JSON.parse(sessionSource.body).conversation_id ===
                    this.conversation,
        );
        assert.equal(outcome.source_message_id, fact.source_id);
        const brief = JSON.parse(
            await readArtifact(this.observer, this.conversation, this.artifact),
        );
        assert.equal(outcome.reference_code, brief.reference_code);
        assert.equal(outcome.color, brief.requested_color);
        assert.equal(outcome.artifact_sha256, this.artifact.sha256);
        assert.deepEqual(
            await this.observer.send(
                "continuity-decision",
                this.conversation,
                this.decisionBody,
            ),
            this.decision,
        );
        await assert.rejects(
            () =>
                this.observer.artifactGet(otherConversation, this.artifact.id),
            (error) => ["forbidden", "not-found"].includes(error.code),
        );
        const foreign = await this.observer.contextView(otherConversation, 100);
        assert(
            !foreign.items.some((item) =>
                [
                    fact.source_id,
                    decision.source_id,
                    this.artifact.id,
                    this.sessionId,
                ].includes(item.source_id),
            ),
        );
        const followupBody = JSON.stringify({
            action: "launch_followup",
            decision_message_id: decision.source_id,
            source_message_id: fact.source_id,
            artifact_id: outcome.artifact_id,
            session_id: outcome.session_id,
            body: `${outcome.project} is ${outcome.decision}: use ${outcome.color} for ${brief.deadline}; reference ${brief.reference_code}.`,
        });
        const followup = await this.client.send(
            "continuity-followup",
            this.conversation,
            followupBody,
        );
        assert.deepEqual(
            await this.client.send(
                "continuity-followup",
                this.conversation,
                followupBody,
            ),
            followup,
        );
        const final = await this.observer.contextView(this.conversation, 100);
        assert.equal(
            final.items.filter((item) => item.source_id === decision.source_id)
                .length,
            1,
        );
        assert.equal(
            final.items.filter(
                (item) =>
                    item.source_id === followup.message_id &&
                    item.actor_id === this.owner &&
                    item.body === followupBody,
            ).length,
            1,
        );
        assert(
            !final.items.some((item) =>
                ["voice_transcript", "voice_summary"].includes(item.kind),
            ),
        );
        return {
            status: "passed",
            conversation_id: this.conversation,
            session_id: this.sessionId,
            source_message_id: fact.source_id,
            decision_message_id: decision.source_id,
            decision_actor_id: decision.actor_id,
            followup_message_id: followup.message_id,
            artifact_id: this.artifact.id,
            artifact_sha256: this.artifact.sha256,
            artifact_size: this.artifact.size,
            artifact_bytes_reverified_after_restart: true,
            explicit_omitted_sources: final.omitted,
            exact_decision_replay: true,
            one_followup: true,
            foreign_conversation_sources: 0,
            transcript_status:
                "No transcript or derived summary was produced; AV transport supplies no ASR or model vision.",
        };
    }
}
