import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, stat, readFile, chmod } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { WorkerState, stateJson } from "./state.mjs";
const run = promisify(execFile);
const root = fileURLToPath(new URL("../../../.local/gate-6/decision-worker/", import.meta.url));
await mkdir(root, {recursive:true,mode:0o700});
const initial = () => ({stage:"opening-end",cursor:null,pending:null,completed:{},seen:[]});
const options = directory => ({directory,workerId:"worker-1",conversationId:"conversation-1",initial:initial()});
const directory = () => mkdtemp(join(root,"journal-"));
const digest = bytes => createHash("sha256").update(bytes).digest("hex");

test("private application state reopens the exact original intent and gives independent snapshots", async () => {
    const dir = await directory(); const state = await WorkerState.open(options(dir));
    const value = initial(); value.pending = {operationId:"original-1",intent:{text:"approved original",recipients:["owner"]},phase:"submitted"};
    state.write(0,value); value.pending.intent.text="later local mutation";
    const snapshot=state.read(); assert.equal(snapshot.value.pending.intent.text,"approved original");
    snapshot.value.pending.operationId="copied-scalar"; assert.equal(state.read().value.pending.operationId,"original-1");
    assert.equal((await stat(join(dir,"decision-worker.sqlite3"))).mode&0o777,0o600);
    assert.equal((await stat(dir)).mode&0o777,0o700);
    state.close(); state.close();
    const reopened=await WorkerState.open(options(dir));
    assert.deepEqual(reopened.read(),{revision:1,value:{...initial(),pending:{operationId:"original-1",intent:{text:"approved original",recipients:["owner"]},phase:"submitted"}}});
    reopened.close();
});
test("one live owner cannot be stolen and a different worker/conversation cannot reuse the state", async () => {
    const dir=await directory();const first=await WorkerState.open(options(dir));
    await assert.rejects(WorkerState.open(options(dir)),{code:"worker-already-running"});
    first.close();
    for(const changed of [{workerId:"other-worker"},{conversationId:"other-conversation"}]) {
        await assert.rejects(WorkerState.open({...options(dir),...changed}),{code:"different-worker-scope"});
    }
    const resumed=await WorkerState.open(options(dir));assert.equal(resumed.read().revision,0);resumed.close();
});
test("a real process exit leaves the original pending effect durable for the next owner", async () => {
    const dir=await directory();
    const source='const {WorkerState}=await import(process.argv[1]);const s=await WorkerState.open({directory:process.argv[2],workerId:"worker-1",conversationId:"conversation-1",initial:{stage:"opening-end",cursor:null,pending:null,completed:{},seen:[]}});s.write(0,{stage:"opening-end",cursor:"opaque-original",pending:{operationId:"same-original",phase:"submitted",intent:{text:"never replan this"}},completed:{},seen:["event-1"]});process.stdout.write("COMMITTED");process.exit(0);';
    const exited=await run(process.execPath,["--input-type=module","-e",source,new URL("./state.mjs",import.meta.url).href,dir],{timeout:10000});
    assert.equal(exited.stdout,"COMMITTED");
    const resumed=await WorkerState.open(options(dir));const saved=resumed.read();
    assert.equal(saved.revision,1);assert.equal(saved.value.pending.operationId,"same-original");
    assert.equal(saved.value.cursor,"opaque-original");assert.equal(saved.value.pending.phase,"submitted");
    assert.throws(()=>resumed.write(0,{pending:null}),{code:"stale-business-state"});
    assert.deepEqual(resumed.read(),saved);resumed.close();
});
test("oversized or non-JSON writes leave the accepted original state unchanged", async () => {
    const state=await WorkerState.open(options(await directory()));const original=state.read();
    assert.throws(()=>state.write(0,{text:"x".repeat(1024*1024)}),{code:"business-state-capacity"});
    for(const value of [{bad:undefined},{bad:Infinity},{bad:()=>true},{bad:new Date()},[]]) {
        assert.throws(()=>state.write(0,value),{code:"invalid-business-state"});
    }
    assert.deepEqual(state.read(),original);state.close();
    assert.throws(()=>state.read(),{code:"worker-state-closed"});
    assert.equal(stateJson({z:1,a:{b:2,a:1}}),'{"a":{"a":1,"b":2},"z":1}');
});
test("an unrelated database profile is refused without rewriting its bytes", async () => {
    const dir=await directory();const path=join(dir,"decision-worker.sqlite3");
    const foreign=new DatabaseSync(path);foreign.exec("CREATE TABLE unrelated(value TEXT);INSERT INTO unrelated VALUES('preserve')");
    foreign.close();await chmod(path,0o600);const before=digest(await readFile(path));
    await assert.rejects(WorkerState.open(options(dir)),{code:"foreign-worker-database"});
    assert.equal(digest(await readFile(path)),before);
});
test("corrupt retained state is refused without replacing it with defaults", async () => {
    const dir=await directory();const state=await WorkerState.open(options(dir));state.write(0,{pending:{operationId:"preserve-original"}});state.close();
    const path=join(dir,"decision-worker.sqlite3");const corrupt=new DatabaseSync(path);corrupt.prepare("UPDATE worker SET state_json=? WHERE id=1").run("{bad json");corrupt.close();
    const before=digest(await readFile(path));
    await assert.rejects(WorkerState.open(options(dir)),{code:"corrupt-business-state"});
    assert.equal(digest(await readFile(path)),before);
});
