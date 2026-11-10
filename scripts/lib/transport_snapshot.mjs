// Read one current-authorized public snapshot for the complete original cohort.
// Six typed Connection.refresh calls otherwise recapture the same Conversation
// six times, at different revisions, on every retirement observation round.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

export async function connectionSnapshots(client, conversationId, originals, options) {
    assert.ok(originals.length > 0 && originals.length <= 6);
    assert.equal(new Set(originals.map(v => v.id)).size, originals.length);
    assert.ok(Number.isFinite(options.timeoutMs) && options.timeoutMs > 0);
    const until = performance.now() + options.timeoutMs;
    const connections = new Map(), cursors = new Set();
    let page, revision, cursor, bytes = 0, pages = 0, resyncs = 0;
    for (let round = 0; round < 256; round++) {
        const remaining = Math.floor(until - performance.now());
        assert.ok(remaining > 0, "snapshot-observation-deadline");
        const request = { v: 2, id: "snapshot_" + randomUUID(), ts: new Date().toISOString(),
            type: "conversation.get", cid: conversationId, payload: { limit: 100, ...(page ? { page } : {}) } };
        let frames;
        try {
            // The existing owned helper performs the same frozen Rust schema
            // validation and current signed read. No synchronous validator child.
            frames = (await client.raw.read(request, { ...options, timeoutMs: remaining })).map(v => v.value);
        } catch (failure) {
            const refusedRead = failure.certainty === "refused" && !failure.operation && !failure.observation;
            if (refusedRead && failure.kind === "application" && failure.problem?.code === 503
                && failure.problem?.category === "authorization" && failure.problem?.reason === "authority_unavailable") {
                await sleep(Math.min(125, Math.max(0, until - performance.now())), undefined, { signal: options.signal });
                continue;
            }
            if (refusedRead && page && resyncs < 2 && failure.kind === "conflict"
                && failure.problem?.code === 409 && failure.problem?.category === "state"
                && failure.problem?.reason === "resync_required") {
                resyncs++;page=revision=cursor=undefined;pages=0;connections.clear();cursors.clear();
                // Original deadline, cumulative bytes and256 total requests stay.
                continue;
            }
            throw failure;
        }
        assert.ok(performance.now() <= until, "snapshot-observation-deadline");
        const snapshots = frames.filter(v => v.type === "conversation.snapshot");
        assert.equal(frames.length, 2);
        assert.equal(frames.filter(v => v.type === "ack").length, 1);
        assert.equal(snapshots.length, 1);
        const snapshot = snapshots[0].payload;pages++;
        assert.ok(Array.isArray(snapshot.items) && typeof snapshot.has_more === "boolean");
        assert.ok(Number.isSafeInteger(snapshot.revision) && snapshot.revision >= 0);
        assert.ok(typeof snapshot.cursor === "string" && snapshot.cursor.length > 0);
        if (revision === undefined) { revision = snapshot.revision; cursor = snapshot.cursor; }
        assert.equal(snapshot.revision, revision, "snapshot-revision-changed");
        assert.equal(snapshot.cursor, cursor, "snapshot-watermark-changed");
        for (const item of snapshot.items) {
            bytes += Buffer.byteLength(JSON.stringify(item));
            assert.ok(bytes <= 2 * 1024 * 1024, "snapshot-byte-ceiling");
            if (item.kind !== "connection") continue;
            const value = item.data;
            assert.ok(typeof value.connection_id === "string" && typeof value.session_id === "string");
            assert.ok(Number.isSafeInteger(value.generation) && value.generation > 0 && typeof value.state === "string");
            assert.equal(connections.has(value.connection_id), false, "duplicate-snapshot-connection");
            connections.set(value.connection_id, value);
        }
        if (!snapshot.has_more) return { revision, pages, connections: originals.map(original => {
            const current = connections.get(original.id);
            assert.ok(current, "original-connection-missing");
            assert.equal(current.session_id, original.sessionId, "original-connection-session-changed");
            return { id: current.connection_id, sessionId: current.session_id, state: current.state, generation: current.generation };
        }) };
        assert.ok(snapshot.items.length > 0, "empty-nonterminal-snapshot-page");
        assert.ok(typeof snapshot.next_page === "string" && snapshot.next_page.length > 0);
        assert.equal(cursors.has(snapshot.next_page), false, "repeated-snapshot-page");
        cursors.add(snapshot.next_page);
        page = snapshot.next_page;
    }
    assert.fail("snapshot-page-ceiling");
}
