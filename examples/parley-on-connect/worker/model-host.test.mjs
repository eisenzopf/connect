// Parser/approval/custody tests only: no Codex host, account or model invocation.
import test, { after } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, readdir, rm } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
const run = promisify(execFile), directories = [];
const base = new URL("../../../.local/gate-6/decision-worker-host-tests/", import.meta.url);
const host = fileURLToPath(new URL("./model-host.py", import.meta.url));
after(async () => { for (const dir of directories) await rm(dir, { recursive: true, force: true }); });
async function directory() {
    await mkdir(base, { recursive: true, mode: 0o700 });
    const path = await mkdtemp(new URL("host-", base).pathname); directories.push(path); return path;
}
const prefix = "import importlib.util,json,sys\nspec=importlib.util.spec_from_file_location('host',sys.argv[1]);host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)\n";
const valid = () => [{ type: "thread.started", thread_id: "0199a213-81c0-7800-8aa1-bbab2a035a53" },
    { type: "turn.started" }, { type: "item.completed", item: { id: "reason", type: "reasoning", text: "synthetic fixture" } },
    { type: "item.completed", item: { id: "answer", type: "agent_message", text: JSON.stringify({ action: "propose", text: "Review the sandbox proposal." }) } },
    { type: "turn.completed" }];
async function parse(events) {
    const dir = await directory(), path = join(dir, "events.jsonl");
    await writeFile(path, events.map(x => JSON.stringify(x)).join("\n") + "\n", { mode: 0o600 });
    return run("python3", ["-c", prefix + "print(json.dumps(host.audit(open(sys.argv[2]).read())))", host, path], { timeout: 10000 });
}
test("synthetic JSONL audit recognizes one original completed proposal without treating it as actual-model evidence", async () => {
    const result = JSON.parse((await parse(valid())).stdout);
    assert.equal(result.session_id, valid()[0].thread_id); assert.equal(result.output.action, "propose");
    assert.equal(result.tool_items, 0); assert.equal(result.event_count, 5);
});
test("tool activity, duplicate turns and failed/unknown host events refuse", async () => {
    for (const type of ["command_execution", "file_change", "mcp_tool_call", "web_search", "plan_update"]) {
        const events = valid(); events.splice(2, 0, { type: "item.started", item: { id: "forbidden", type } });
        await assert.rejects(parse(events));
    }
    for (const event of [{ type: "turn.completed" }, { type: "turn.started" }, { type: "turn.failed" }, { type: "unexpected" }])
        await assert.rejects(parse([...valid(), event]));
});
test("host output cannot carry extra authority fields, oversized text or malformed JSON", async () => {
    for (const text of [JSON.stringify({ action: "propose", text: "x", approved: true }),
        JSON.stringify({ action: "propose", text: "x".repeat(4097) }), "```json\n{}\n```", "{broken"]) {
        const events = valid(); events[3].item.text = text; await assert.rejects(parse(events));
    }
});
test("unapproved G6-073 config refuses before output creation, authentication probe or host execution", async () => {
    const dir = await directory(), config = join(dir, "config.json"), request = join(dir, "request.json");
    await writeFile(config, JSON.stringify({ profile: "parley-worker-model-rehearsal/1", task: "G6-073", approved_model_execution: false }), { mode: 0o600 });
    await writeFile(request, JSON.stringify({ profile: "parley-model-request/1" }), { mode: 0o600 });
    await assert.rejects(run("python3", [host, "--config", config, "--request", request], { timeout: 10000 }), error => {
        assert.match(error.stderr, /Separate G6-073 owner agent-execution authorization required/); return true;
    });
    assert.deepEqual((await readdir(dir)).sort(), ["config.json", "request.json"]);
});
test("owned supervisor sends exact bounded stdin, joins normal exit and detects a surviving descendant", async () => {
    const dir = await directory();
    const code = prefix + `
out=open(sys.argv[2],'xb')
payload=b'synthetic private prompt'*2048
normal,forced=host.native.owned_worker([sys.executable,'-c','import sys; data=sys.stdin.buffer.read(); print(len(data))'],out,timeout_seconds=5,input_bytes=payload)
out.close()
assert normal==0 and not forced
assert open(sys.argv[2]).read().strip()==str(len(payload))
out=open(sys.argv[3],'xb')
code,forced=host.native.owned_worker([sys.executable,'-c',"import subprocess;subprocess.Popen(['sleep','30'])"],out,timeout_seconds=5)
out.close()
assert code==0 and forced
print('synthetic process custody only')
`;
    const result = await run("python3", ["-c", code, host, join(dir, "stdin.log"), join(dir, "group.log")], { timeout: 10000 });
    assert.equal(result.stdout.trim(), "synthetic process custody only");
});
