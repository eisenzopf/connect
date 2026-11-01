import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, stat, readFile, chmod, symlink, link } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { DatabaseSync } from "node:sqlite";
import { AppIntents } from "./intents.mjs";
const run = promisify(execFile);
const root = fileURLToPath(new URL("../../../.local/gate-6/reference-app/", import.meta.url));
await mkdir(root, { recursive: true, mode: 0o700 });
const directory = () => mkdtemp(join(root, "intents-"));
const input = () => ({ actor: "owner", command: "message.send", scope: { conversationId: "conversation-1" },
    input: { text: "private-app-body", to: ["recipient-1"] } });
const reference = value => ({ schema: "connect-operation-reference/1", operation_id: value.operationId,
    command: value.intent.command, scope: { conversation_id: "conversation-1" } });
const snapshot = (value, state = "accepted", revision = 1) => ({ key: { operationId: value.operationId,
    command: value.intent.command, scope: value.intent.scope }, state, revision });

test("original intent and ID precede dispatch and survive reopen without another attempt", async () => {
    const dir = await directory(), journal = await AppIntents.open(dir), original = input();
    const saved = journal.prepare("button-1", original);
    original.input.text = "later local edit";
    assert.equal(journal.read(saved.operationId).intent.input.text, "private-app-body");
    assert.equal(journal.prepare("button-1", input()).operationId, saved.operationId);
    assert.throws(() => journal.prepare("button-1", original), { code: "different-original-intent" });
    assert.equal((await stat(join(dir, "app-intents.sqlite3"))).mode & 0o777, 0o600);
    journal.attempt(saved.operationId); journal.close();
    const reopened = await AppIntents.open(dir);
    assert.equal(reopened.prepare("button-1", input()).operationId, saved.operationId);
    assert.equal(reopened.read(saved.operationId).phase, "attempted");
    assert.throws(() => reopened.attempt(saved.operationId), { code: "original-operation-needs-recovery" });
    reopened.close();
});

test("actual process exit retains attempted original ID and a body-free recovery projection", async () => {
    const dir = await directory();
    const source = 'const {AppIntents}=await import(process.argv[1]);const j=await AppIntents.open(process.argv[2]);const v=j.prepare("crash-button",JSON.parse(process.argv[3]));j.attempt(v.operationId);process.stdout.write(v.operationId);process.exit(0);';
    const child = await run(process.execPath, ["--input-type=module", "-e", source,
        new URL("./intents.mjs", import.meta.url).href, dir, JSON.stringify(input())], { timeout: 10000 });
    const journal = await AppIntents.open(dir);
    assert.equal(journal.prepare("crash-button", input()).operationId, child.stdout);
    const progress = journal.progress("owner");
    assert.deepEqual(progress, [{ operationId: child.stdout, actor: "owner", command: "message.send",
        submission: "attempted", state: null, revision: null, needsRecovery: true }]);
    assert.ok(!JSON.stringify(progress).includes("private-app-body"));
    assert.equal(JSON.stringify(journal), '{"kind":"app-intents"}');
    journal.close();
});

test("observations bind original scope and refuse rollback or a changed terminal result", async () => {
    const journal = await AppIntents.open(await directory()), saved = journal.prepare("progress-button", input());
    journal.attempt(saved.operationId);
    journal.observe(saved.operationId, reference(saved), snapshot(saved));
    const before = journal.read(saved.operationId);
    const foreign = reference(saved); foreign.scope.conversation_id = "foreign-conversation";
    assert.throws(() => journal.observe(saved.operationId, foreign, snapshot(saved)), { code: "different-operation-observation" });
    assert.deepEqual(journal.read(saved.operationId), before);
    journal.uncertain(saved.operationId);
    assert.equal(journal.progress("owner")[0].needsRecovery, true);
    journal.observe(saved.operationId, reference(saved), snapshot(saved, "completed", 2));
    const completed = journal.read(saved.operationId);
    assert.throws(() => journal.observe(saved.operationId, reference(saved), snapshot(saved)), { code: "stale-operation-observation" });
    assert.throws(() => journal.uncertain(saved.operationId), { code: "invalid-app-transition" });
    assert.deepEqual(journal.observe(saved.operationId, reference(saved), snapshot(saved, "completed", 2)), completed);
    journal.close();
});

test("live ownership is exclusive and unrelated database bytes are preserved", async () => {
    const dir = await directory(), journal = await AppIntents.open(dir);
    await assert.rejects(AppIntents.open(dir), { code: "app-already-running" });
    assert.deepEqual(journal.progress("owner"), []); journal.close();
    const foreign = await directory(), path = join(foreign, "app-intents.sqlite3"), db = new DatabaseSync(path);
    db.exec("CREATE TABLE preserve(value TEXT); INSERT INTO preserve VALUES('unrelated')"); db.close();
    await chmod(path, 0o600); const original = await readFile(path);
    await assert.rejects(AppIntents.open(foreign), { code: "foreign-app-database" });
    assert.deepEqual(await readFile(path), original);
});

test("linked state files and malformed saved observations refuse without reset", async () => {
    const dir = await directory(), journal = await AppIntents.open(dir), saved = journal.prepare("saved", input());
    journal.attempt(saved.operationId); journal.observe(saved.operationId, reference(saved), snapshot(saved)); journal.close();
    const path = join(dir, "app-intents.sqlite3");
    for (const create of [symlink, link]) {
        const elsewhere = await directory(); await create(path, join(elsewhere, "app-intents.sqlite3"));
        await assert.rejects(AppIntents.open(elsewhere), { code: "private-app-state-required" });
        if (create === link) { const { unlink } = await import("node:fs/promises"); await unlink(join(elsewhere, "app-intents.sqlite3")); }
    }
    const db = new DatabaseSync(path), wrong = snapshot(saved); wrong.key.operationId = "foreign-operation";
    db.prepare("UPDATE intents SET snapshot_json=?").run(JSON.stringify(wrong)); db.close();
    const original = await readFile(path);
    await assert.rejects(AppIntents.open(dir), { code: "corrupt-app-state" });
    assert.deepEqual(await readFile(path), original);
});

test("capacity and malformed input never evict or alter retained original actions", async () => {
    const journal = await AppIntents.open(await directory()), first = journal.prepare("first", input());
    for (const invalid of [{ ...input(), input: { bad: undefined } }, { ...input(), input: { text: "x".repeat(65536) } }])
        assert.throws(() => journal.prepare("invalid", invalid));
    for (let count = 1; count < 128; count++) journal.prepare("button-" + count, input());
    assert.throws(() => journal.prepare("overflow", input()), { code: "app-intent-capacity" });
    assert.deepEqual(journal.prepare("first", input()), first);
    journal.close();
});

test("progress is actor-scoped and known not-submitted refusal cannot replace accepted state", async () => {
    const journal = await AppIntents.open(await directory()), owner = journal.prepare("owner-button", input());
    const other = journal.prepare("other-button", { ...input(), actor: "organizer" });
    journal.attempt(owner.operationId); journal.notSubmitted(owner.operationId);
    assert.equal(journal.progress("owner")[0].submission, "refused");
    assert.equal(journal.progress("owner")[0].needsRecovery, false);
    assert.equal(journal.progress("organizer")[0].operationId, other.operationId);
    assert.equal(journal.progress("owner").length, 1);
    assert.throws(() => journal.progress(), { code: "invalid-app-actor" });
    journal.attempt(other.operationId);
    journal.observe(other.operationId, reference(other), snapshot(other));
    assert.throws(() => journal.notSubmitted(other.operationId), { code: "invalid-app-transition" });
    journal.close();
    assert.throws(() => journal.attempt(other.operationId), { code: "app-journal-closed" });
});
