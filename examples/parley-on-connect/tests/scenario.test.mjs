import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { loadScenario, bindScenario, checkMappings, rehearsalPath } from "../scenario.mjs";
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
        (s) => { s.stages.at(-1).next = [s.start]; },
    ]) { const copy = structuredClone(scenario); change(copy); assert.throws(() => checkMappings(copy, sdk, wire)); }
});
test("fixture and live definitions use the same explicit paths, audiences and unsatisfied business gates", () => {
    const bindings = { model: "scripted-model", telephone: "loopback-sip", sms: "loopback-sms", contacts: "synthetic-contacts" };
    const fixture = bindScenario(scenario, "fixture", bindings);
    const live = bindScenario(scenario, "live", { model: "configured-model", telephone: "configured-sip", sms: "configured-sms", contacts: "reviewed-contacts" });
    for (const consultation of ["takeover", "decline-return"]) {
        for (const observer of [true, false]) {
            const path = rehearsalPath(fixture, { consultation, observer });
            assert.deepEqual(path, rehearsalPath(live, { consultation, observer }));
            assert.equal(path.length, consultation === "takeover" ? 16 : 15);
            assert.equal(path[0].id, scenario.start);
            assert.equal(path.at(-1).id, "recover-and-audit");
            assert.equal(path.find((stage) => stage.id === "follow-up").calls.length, observer ? 4 : 3);
            assert.equal(path.some((stage) => stage.id === "move"), consultation === "takeover");
            assert.equal(path.some((stage) => stage.id === "decline-return"), consultation === "decline-return");
            for (const stage of path) {
                const original = scenario.stages.find((entry) => entry.id === stage.id);
                assert.deepEqual(stage.gates, original.gates);
                assert.deepEqual(stage.observations, original.observations);
                assert.deepEqual(stage.acceptance, original.acceptance);
                for (const call of stage.calls) assert.ok(call.audience.every((actor) => Object.hasOwn(scenario.actors, actor)));
            }
            assert.throws(() => { path[0].gates.length = 0; }, TypeError);
        }
    }
    assert.throws(() => rehearsalPath(fixture), /explicit consultation/);
    assert.throws(() => rehearsalPath(fixture, { consultation: "answer-means-takeover" }), /explicit consultation/);
    assert.throws(() => rehearsalPath(fixture, { consultation: "takeover", observer: "yes" }), /observer/);
});

test("shared path selection refuses cycles and an unrecognized future branch", () => {
    const bindings = { model: "model", telephone: "telephone", sms: "sms", contacts: "contacts" };
    for (const change of [
        (s) => { s.stages.at(-1).next = [s.start]; },
        (s) => { s.stages[0].next.push("proposal"); },
        (s) => { s.stages.find((stage) => stage.id === "private-consultation").next = ["voice-end"]; },
    ]) {
        const changed = structuredClone(scenario); change(changed);
        assert.throws(() => rehearsalPath(bindScenario(changed, "fixture", bindings), { consultation: "takeover" }));
    }
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
