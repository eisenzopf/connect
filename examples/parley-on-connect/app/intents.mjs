// Private application intent journal. This file never opens Connect's database.
import { DatabaseSync } from "node:sqlite";
import { randomUUID, createHash } from "node:crypto";
import { mkdir, lstat, open } from "node:fs/promises";
import { isAbsolute, resolve, sep, join } from "node:path";
import { inspect } from "node:util";

const PROFILE = "parley-app-intents/2", APPLICATION = 0x50414932, MAX_INTENTS = 128;
const phases = new Set(["prepared", "attempted", "uncertain", "observed", "refused"]);
const states = new Set(["accepted", "preparing", "ready", "committing", "completed", "cancelled", "failed", "interrupted", "unknown"]);
const terminal = new Set(["completed", "cancelled", "failed"]);
const roles = new Set(["owner", "assistant", "booker", "organizer", "observer"]);
const identifier = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const fail = code => { const error = Error(code); error.code = code; throw error; };
function json(value) {
    function ordered(item, depth = 0) {
        if (depth > 32) fail("invalid-app-intent");
        if (item === null || typeof item === "string" || typeof item === "boolean") return item;
        if (typeof item === "number" && Number.isFinite(item)) return item;
        if (Array.isArray(item)) return item.map(child => ordered(child, depth + 1));
        if (item && [Object.prototype, null].includes(Object.getPrototypeOf(item)))
            return Object.fromEntries(Object.keys(item).sort().map(key => [key, ordered(item[key], depth + 1)]));
        fail("invalid-app-intent");
    }
    const encoded = JSON.stringify(ordered(value));
    if (Buffer.byteLength(encoded) > 65536) fail("app-intent-too-large");
    return encoded;
}
const checksum = encoded => createHash("sha256").update(encoded).digest("hex");
function intent(value) {
    if (!value || Object.keys(value).sort().join() !== "actor,command,input,scope" || !roles.has(value.actor) ||
        typeof value.command !== "string" || !/^[a-z][a-z0-9._-]{1,63}(?:\.[a-z0-9._-]+)+$/.test(value.command) ||
        !value.scope || Array.isArray(value.scope) || typeof value.scope !== "object" ||
        !value.input || Array.isArray(value.input) || typeof value.input !== "object") fail("invalid-app-intent");
    for (const [key, target] of Object.entries(value.scope))
        if (!["conversationId", "sessionId", "connectionId"].includes(key) || !identifier(target)) fail("invalid-app-intent");
    return json(value);
}
function live(pid) {
    if (!Number.isSafeInteger(pid) || pid < 1) fail("corrupt-app-owner");
    try { process.kill(pid, 0); return true; }
    catch (error) { if (error.code === "ESRCH") return false; if (error.code === "EPERM") return true; throw error; }
}

/** Original IDs are durable before any public SDK call. An attempted intent
 * must be recovered through the SDK; reopening this journal never resends it.
 * Credentials and Connect state remain outside this application journal. */
export class AppIntents {
    #db; #token = randomUUID(); #closed = false;
    [inspect.custom]() { return "AppIntents(private)"; }
    toJSON() { return { kind: "app-intents" }; }
    constructor(db) { this.#db = db; }
    static async open(directory) {
        if (!isAbsolute(directory) || !resolve(directory).split(sep).includes(".local")) fail("private-app-state-required");
        await mkdir(directory, { recursive: true, mode: 0o700 });
        const folder = await lstat(directory);
        if (!folder.isDirectory() || (folder.mode & 0o077)) fail("private-app-state-required");
        const path = join(directory, "app-intents.sqlite3");
        try { const created = await open(path, "wx", 0o600); await created.close(); }
        catch (error) { if (error.code !== "EEXIST") throw error; }
        const file = await lstat(path);
        if (!file.isFile() || file.nlink !== 1 || (file.mode & 0o077) || file.size > 16 * 1024 * 1024)
            fail("private-app-state-required");
        const journal = new AppIntents(new DatabaseSync(path, { timeout: 1000, allowExtension: false }));
        try {
            journal.#db.exec("PRAGMA synchronous=FULL");
            journal.#transaction(() => {
                const tables = journal.#db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map(row => row.name);
                const application = journal.#db.prepare("PRAGMA application_id").get().application_id;
                if (tables.length === 0 && application === 0) {
                    journal.#db.exec("CREATE TABLE app (id INTEGER PRIMARY KEY CHECK(id=1),profile TEXT NOT NULL,owner_pid INTEGER,owner_token TEXT) STRICT; CREATE TABLE intents (client_key TEXT PRIMARY KEY,operation_id TEXT UNIQUE NOT NULL,intent_json TEXT NOT NULL,intent_sha256 TEXT NOT NULL,phase TEXT NOT NULL,reference_json TEXT,snapshot_json TEXT,result_json TEXT) STRICT");
                    journal.#db.exec("PRAGMA application_id=" + APPLICATION);
                    journal.#db.prepare("INSERT INTO app VALUES(1,?,NULL,NULL)").run(PROFILE);
                } else if (application !== APPLICATION || tables.join() !== "app,intents") fail("foreign-app-database");
                const owner = journal.#db.prepare("SELECT * FROM app WHERE id=1").get();
                if (!owner || owner.profile !== PROFILE || (owner.owner_pid === null) !== (owner.owner_token === null)) fail("corrupt-app-owner");
                if (owner.owner_pid !== null && live(owner.owner_pid)) fail("app-already-running");
                const rows = journal.#db.prepare("SELECT * FROM intents").all();
                if (rows.length > MAX_INTENTS) fail("corrupt-app-state");
                for (const row of rows) journal.#decode(row);
                journal.#db.prepare("UPDATE app SET owner_pid=?,owner_token=? WHERE id=1").run(process.pid, journal.#token);
            });
            return journal;
        } catch (error) { journal.#closed = true; journal.#db.close(); throw error; }
    }
    #transaction(work) {
        if (this.#closed) fail("app-journal-closed");
        this.#db.exec("BEGIN IMMEDIATE");
        try { const value = work(); this.#db.exec("COMMIT"); return value; }
        catch (error) { this.#db.exec("ROLLBACK"); throw error; }
    }
    #owned() {
        if (this.#closed) fail("app-journal-closed");
        const row = this.#db.prepare("SELECT owner_pid,owner_token FROM app WHERE id=1").get();
        if (!row || row.owner_pid !== process.pid || row.owner_token !== this.#token) fail("app-owner-lost");
    }
    #decode(row) {
        try {
            const original = JSON.parse(row.intent_json);
            if (!identifier(row.client_key) || !identifier(row.operation_id) || !phases.has(row.phase) ||
                intent(original) !== row.intent_json || checksum(row.intent_json) !== row.intent_sha256) fail("invalid-app-intent");
            const saved = { clientKey: row.client_key, operationId: row.operation_id, intent: original, phase: row.phase,
                reference: row.reference_json === null ? null : JSON.parse(row.reference_json),
                snapshot: row.snapshot_json === null ? null : JSON.parse(row.snapshot_json),
                result: row.result_json === null ? null : JSON.parse(row.result_json) };
            if (saved.reference === null || saved.snapshot === null) {
                if (saved.reference !== null || saved.snapshot !== null || saved.phase === "observed") fail("invalid-app-intent");
            } else {
                if (!["observed", "uncertain"].includes(saved.phase)) fail("invalid-app-intent");
                this.#observation(saved, saved.reference, saved.snapshot);
            }
            if (saved.result !== null && (saved.snapshot?.state !== "completed" ||
                !saved.result || Array.isArray(saved.result) || typeof saved.result !== "object" ||
                Object.entries(saved.result).some(([key, value]) =>
                    !["conversationId", "sessionId", "connectionId", "messageId", "identityId"].includes(key) || !identifier(value))))
                fail("invalid-app-intent");
            return saved;
        } catch { fail("corrupt-app-state"); }
    }
    #read(operationId) {
        const row = this.#db.prepare("SELECT * FROM intents WHERE operation_id=?").get(operationId);
        if (!row) fail("unknown-app-operation");
        return this.#decode(row);
    }
    prepare(clientKey, original) {
        if (!identifier(clientKey)) fail("invalid-app-intent");
        const encoded = intent(original);
        return this.#transaction(() => {
            this.#owned();
            const existing = this.#db.prepare("SELECT * FROM intents WHERE client_key=?").get(clientKey);
            if (existing) {
                if (existing.intent_json !== encoded) fail("different-original-intent");
                return this.#decode(existing);
            }
            if (this.#db.prepare("SELECT COUNT(*) AS count FROM intents").get().count >= MAX_INTENTS) fail("app-intent-capacity");
            const id = "op_app_" + randomUUID();
            this.#db.prepare("INSERT INTO intents VALUES(?,?,?,?,?,NULL,NULL,NULL)")
                .run(clientKey, id, encoded, checksum(encoded), "prepared");
            return this.#read(id);
        });
    }
    attempt(operationId) {
        return this.#transaction(() => {
            this.#owned(); const saved = this.#read(operationId);
            if (saved.phase !== "prepared") fail("original-operation-needs-recovery");
            this.#db.prepare("UPDATE intents SET phase='attempted' WHERE operation_id=?").run(operationId);
            return this.#read(operationId);
        });
    }
    uncertain(operationId) {
        return this.#transaction(() => {
            this.#owned(); const saved = this.#read(operationId);
            if (!["attempted", "uncertain", "observed"].includes(saved.phase) || terminal.has(saved.snapshot?.state)) fail("invalid-app-transition");
            this.#db.prepare("UPDATE intents SET phase='uncertain' WHERE operation_id=?").run(operationId);
            return this.#read(operationId);
        });
    }
    /** Trusted gateway calls this only for the public SDK's not-submitted
     * certainty. A transport error or an accepted operation cannot use it. */
    notSubmitted(operationId) {
        return this.#transaction(() => {
            this.#owned(); const saved = this.#read(operationId);
            if (!["prepared", "attempted"].includes(saved.phase) || saved.snapshot !== null || saved.reference !== null)
                fail("invalid-app-transition");
            this.#db.prepare("UPDATE intents SET phase='refused' WHERE operation_id=?").run(operationId);
            return this.#read(operationId);
        });
    }
    refused(operationId) {
        // A known SDK refusal preserves the same intent. It makes no claim
        // about whether Connect retained a durable refusal record.
        return this.notSubmitted(operationId);
    }
    #observation(saved, reference, snapshot) {
        const expectedScope = Object.fromEntries(Object.entries(saved.intent.scope).map(([key, value]) =>
            [key.replace(/[A-Z]/g, c => "_" + c.toLowerCase()), value]));
        if (reference?.schema !== "connect-operation-reference/1" ||
            reference.operation_id !== saved.operationId || reference.command !== saved.intent.command ||
            json(reference.scope) !== json(expectedScope) || snapshot?.key?.operationId !== saved.operationId ||
            snapshot.key.command !== saved.intent.command || json(snapshot.key.scope) !== json(saved.intent.scope) ||
            !states.has(snapshot.state) || !Number.isSafeInteger(snapshot.revision) || snapshot.revision < 0)
            fail("different-operation-observation");
    }
    observe(operationId, reference, snapshot) {
        return this.#transaction(() => {
            this.#owned(); const saved = this.#read(operationId);
            if (!["attempted", "uncertain", "observed"].includes(saved.phase)) fail("invalid-app-transition");
            this.#observation(saved, reference, snapshot);
            const ref = json(reference), observed = json(snapshot);
            if (saved.reference && json(saved.reference) !== ref) fail("different-operation-observation");
            if (saved.snapshot && (snapshot.revision < saved.snapshot.revision ||
                (snapshot.revision === saved.snapshot.revision && observed !== json(saved.snapshot)) ||
                (terminal.has(saved.snapshot.state) && observed !== json(saved.snapshot)))) fail("stale-operation-observation");
            this.#db.prepare("UPDATE intents SET phase='observed',reference_json=?,snapshot_json=? WHERE operation_id=?")
                .run(ref, observed, operationId);
            return this.#read(operationId);
        });
    }
    read(operationId) { this.#owned(); return this.#read(operationId); }
    lookup(clientKey) {
        if (!identifier(clientKey)) fail("invalid-app-intent");
        this.#owned();
        const row = this.#db.prepare("SELECT * FROM intents WHERE client_key=?").get(clientKey);
        return row ? this.#decode(row) : null;
    }
    /** Persist SDK-returned resource IDs for discovery after restart. These
     * hints grant no access: the requested actor must refresh via its SDK. */
    rememberResult(operationId, result) {
        const encoded = json(result);
        return this.#transaction(() => {
            this.#owned(); const saved = this.#read(operationId);
            if (saved.phase !== "observed" || saved.snapshot?.state !== "completed" ||
                !result || Array.isArray(result) || typeof result !== "object" || !Object.keys(result).length ||
                Object.entries(result).some(([key, value]) =>
                    !["conversationId", "sessionId", "connectionId", "messageId", "identityId"].includes(key) || !identifier(value)) ||
                Object.entries(saved.intent.scope).some(([key, value]) => key in result && result[key] !== value) ||
                (saved.result !== null && json(saved.result) !== encoded)) fail("different-operation-result");
            this.#db.prepare("UPDATE intents SET result_json=? WHERE operation_id=?").run(encoded, operationId);
            return this.#read(operationId);
        });
    }
    discovery() {
        this.#owned();
        return this.#db.prepare("SELECT * FROM intents ORDER BY rowid").all().map(row => this.#decode(row))
            .filter(value => value.result !== null).map(value => ({ actor: value.intent.actor,
                operationId: value.operationId, command: value.intent.command, result: value.result }));
    }
    /** UI progress excludes private command bodies and saved SDK references. */
    progress(actor) {
        if (!roles.has(actor)) fail("invalid-app-actor");
        this.#owned();
        return this.#db.prepare("SELECT * FROM intents ORDER BY rowid").all().map(row => this.#decode(row))
            .filter(value => value.intent.actor === actor).map(value => {
            return { operationId: value.operationId, actor: value.intent.actor, command: value.intent.command,
                submission: value.phase, state: value.snapshot?.state ?? null, revision: value.snapshot?.revision ?? null,
                needsRecovery: ["attempted", "uncertain"].includes(value.phase) ||
                    (value.snapshot?.state === "completed" && value.result === null) };
        });
    }
    close() {
        if (this.#closed) return;
        try { this.#transaction(() => { this.#owned(); this.#db.prepare("UPDATE app SET owner_pid=NULL,owner_token=NULL WHERE id=1").run(); }); }
        finally { this.#closed = true; this.#db.close(); }
    }
}
