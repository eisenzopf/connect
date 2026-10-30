// App-owned business state only. The fixed file is never Connect's database.
import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";
import { mkdir, lstat, open } from "node:fs/promises";
import { isAbsolute, join, sep, resolve } from "node:path";

const PROFILE = "parley-decision-worker/1";
const APPLICATION_ID = 0x50445731;
const LIMIT = 1024 * 1024;
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const fail = code => { const error = Error(code); error.code = code; throw error; };
function encode(value, depth = 0) {
    if (depth > 40) fail("invalid-business-state");
    if (value === null || typeof value === "string" || typeof value === "boolean") return value;
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (Array.isArray(value)) return value.map(child => encode(child, depth + 1));
    if (value && typeof value === "object" &&
        [Object.prototype, null].includes(Object.getPrototypeOf(value))) {
        return Object.fromEntries(Object.keys(value).sort().map(key => [key, encode(value[key], depth + 1)]));
    }
    fail("invalid-business-state");
}
export function stateJson(value) {
    if (!value || Array.isArray(value) || typeof value !== "object") fail("invalid-business-state");
    const text = JSON.stringify(encode(value));
    if (Buffer.byteLength(text) > LIMIT) fail("business-state-capacity");
    return text;
}
function decode(text) {
    try {
        const value = JSON.parse(text);
        stateJson(value);
        return value;
    } catch {
        fail("corrupt-business-state");
    }
}
function alive(pid) {
    if (!Number.isSafeInteger(pid) || pid < 1) fail("corrupt-business-owner");
    try { process.kill(pid, 0); return true; }
    catch (error) {
        if (error.code === "ESRCH") return false;
        if (error.code === "EPERM") return true;
        throw error;
    }
}
/** One live worker owns its application journal. No database transaction spans
 * an SDK/model await. A dead process can be replaced; an alive owner cannot be
 * stolen by a timer or another worker. Pending intent survives either restart. */
export class WorkerState {
    #db;
    #token = randomUUID();
    #closed = false;
    constructor(db, scope) { this.#db = db; Object.defineProperty(this,"scope",{value:Object.freeze(scope),enumerable:true}); }
    static async open({ directory, workerId, conversationId, initial }) {
        if (!isAbsolute(directory) || !resolve(directory).split(sep).includes(".local") ||
            !id(workerId) || !id(conversationId)) fail("invalid-worker-configuration");
        const original = stateJson(initial);
        await mkdir(directory, { recursive: true, mode: 0o700 });
        const folder = await lstat(directory);
        if (!folder.isDirectory() || (folder.mode & 0o077) !== 0) fail("private-worker-directory-required");
        const path = join(directory, "decision-worker.sqlite3");
        try { const file = await open(path, "wx", 0o600); await file.close(); }
        catch (error) { if (error.code !== "EEXIST") throw error; }
        const info = await lstat(path);
        if (!info.isFile() || (info.mode & 0o077) !== 0 || info.size > 16 * LIMIT)
            fail("private-worker-database-required");
        const state = new WorkerState(new DatabaseSync(path, { timeout: 1000, allowExtension: false }), {workerId, conversationId});
        try {
            state.#db.exec("PRAGMA synchronous=FULL");
            state.#transaction(() => {
                const tables = state.#db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map(row => row.name);
                const application = state.#db.prepare("PRAGMA application_id").get().application_id;
                if (tables.length === 0 && application === 0) {
                    state.#db.exec("CREATE TABLE worker (id INTEGER PRIMARY KEY CHECK(id=1), profile TEXT NOT NULL, worker_id TEXT NOT NULL, conversation_id TEXT NOT NULL, revision INTEGER NOT NULL, state_json TEXT NOT NULL, owner_pid INTEGER, owner_token TEXT) STRICT");
                    state.#db.exec("PRAGMA application_id=" + APPLICATION_ID);
                    state.#db.prepare("INSERT INTO worker VALUES(1,?,?,?,?,?,NULL,NULL)")
                        .run(PROFILE, workerId, conversationId, 0, original);
                } else if (application !== APPLICATION_ID || tables.length !== 1 || tables[0] !== "worker") {
                    fail("foreign-worker-database");
                }
                const row = state.#db.prepare("SELECT * FROM worker WHERE id=1").get();
                if (!row || row.profile !== PROFILE || row.worker_id !== workerId ||
                    row.conversation_id !== conversationId) fail("different-worker-scope");
                if (!Number.isSafeInteger(row.revision) || row.revision < 0) fail("corrupt-business-state");
                decode(row.state_json);
                if ((row.owner_pid === null) !== (row.owner_token === null)) fail("corrupt-business-owner");
                if (row.owner_pid !== null && alive(row.owner_pid)) fail("worker-already-running");
                state.#db.prepare("UPDATE worker SET owner_pid=?,owner_token=? WHERE id=1")
                    .run(process.pid, state.#token);
            });
            return state;
        } catch (error) { state.#db.close(); state.#closed = true; throw error; }
    }
    #transaction(work) {
        this.#db.exec("BEGIN IMMEDIATE");
        try { const value = work(); this.#db.exec("COMMIT"); return value; }
        catch (error) { this.#db.exec("ROLLBACK"); throw error; }
    }
    #owned() {
        if (this.#closed) fail("worker-state-closed");
        const row = this.#db.prepare("SELECT revision,state_json,owner_pid,owner_token FROM worker WHERE id=1").get();
        if (!row || row.owner_pid !== process.pid || row.owner_token !== this.#token) fail("worker-state-owner-lost");
        return row;
    }
    read() {
        const row = this.#owned();
        return { revision: row.revision, value: decode(row.state_json) };
    }
    write(expectedRevision, value) {
        if (!Number.isSafeInteger(expectedRevision) || expectedRevision < 0 ||
            expectedRevision >= Number.MAX_SAFE_INTEGER) fail("invalid-business-revision");
        const encoded = stateJson(value);
        return this.#transaction(() => {
            const row = this.#owned();
            if (row.revision !== expectedRevision) fail("stale-business-state");
            const changed = this.#db.prepare("UPDATE worker SET revision=?,state_json=? WHERE id=1 AND owner_token=? AND revision=?")
                .run(expectedRevision + 1, encoded, this.#token, expectedRevision).changes;
            if (changed !== 1) fail("stale-business-state");
            return { revision: expectedRevision + 1, value: decode(encoded) };
        });
    }
    close() {
        if (this.#closed) return;
        try {
            this.#transaction(() => {
                this.#owned();
                this.#db.prepare("UPDATE worker SET owner_pid=NULL,owner_token=NULL WHERE id=1 AND owner_token=?")
                    .run(this.#token);
            });
        } finally { this.#closed = true; this.#db.close(); }
    }
}
