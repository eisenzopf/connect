import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { loadScenario, bindScenario, checkMappings } from "../scenario.mjs";
import { ScriptedModel } from "../fixtures/model.mjs";
const json = async (path) => JSON.parse(await readFile(new URL(path, import.meta.url), "utf8"));
const scenario = await loadScenario();
const sdk = await json("../../../contracts/sdk-gate-6-api.json");
const wire = await json("../../../contracts/schemas/uctp-envelope-2.schema.json");

test("every reachable reference stage maps to the public target contract and A6", () => {
    const rows = checkMappings(scenario, sdk, wire);
    assert.equal(new Set(rows.map((r) => r.stage)).size, 17);
    assert.ok(rows.some((r) => r.canonical_type === "message.history"));
    for (const id of ["A6-07", "A6-08", "A6-09", "A6-11", "A6-20", "A6-21", "A6-28"])
        assert.ok(scenario.stages.some((s) => s.acceptance.includes(id)), id);
    assert.throws(() => { scenario.business_policy.sandbox = false; }, TypeError);
});
test("fixture and live bindings retain the identical immutable stage and policy definition", () => {
    const bindings = { model: "model-ref", telephone: "sip-ref", sms: "sms-ref", contacts: "contacts-ref" };
    const fixture = bindScenario(scenario, "fixture", bindings);
    const live = bindScenario(scenario, "live", { ...bindings, authorization_reference: "G6-093-example-only" });
    assert.equal(fixture.definition, live.definition);
    assert.equal(fixture.simulated, true); assert.equal(live.simulated, false);
    assert.throws(() => bindScenario(scenario, "fixture", { ...bindings, business_policy: {} }));
    assert.throws(() => bindScenario(scenario, "fixture", {}));
    assert.throws(() => bindScenario(scenario, "automatic", bindings));
});
test("broken mappings, actors, graph and acceptance identifiers refuse", () => {
    for (const change of [
        (s) => { s.stages[0].calls[0].method = "parley.start"; },
        (s) => { s.stages[0].calls[0].actor = "unknown"; },
        (s) => { s.stages[0].next = ["missing"]; },
        (s) => { s.stages[0].acceptance = ["A6-99"]; },
        (s) => { s.stages[0].next = []; },
    ]) { const copy = structuredClone(scenario); change(copy); assert.throws(() => checkMappings(copy, sdk, wire)); }
});
test("scripted assistant output is deterministic and visibly simulated", async () => {
    const model = await ScriptedModel.load();
    for (const stage of Object.keys(model.script)) {
        const output = model.respond(stage);
        assert.deepEqual(output, model.respond(stage));
        assert.equal(output.role, "assistant"); assert.equal(output.simulated, true);
        assert.match(output.text, /SIMULATED/);
        output.text = "changed"; assert.notEqual(model.respond(stage).text, "changed");
    }
    assert.throws(() => model.respond("owner-approval"));
    const spoof = new ScriptedModel({ a: { role: "owner", simulated: false, source: "live" } }).respond("a");
    assert.equal(spoof.role, "assistant"); assert.equal(spoof.simulated, true);
    assert.equal(spoof.source, "scripted-model-fixture");
});
