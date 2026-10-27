import { readFile } from "node:fs/promises";
const freeze = (value) => { if (value && typeof value === "object") { Object.values(value).forEach(freeze); Object.freeze(value); } return value; };
export async function loadScenario() { return freeze(JSON.parse(await readFile(new URL("./scenario.json", import.meta.url), "utf8"))); }
/** Shared definition. Mode changes backend/contact bindings, never business stages. */
export function bindScenario(scenario, mode, bindings) {
    if (!["fixture", "live"].includes(mode)) throw Error("explicit mode");
    if (Object.keys(bindings).some((key) => !scenario.binding_contract.allowed_keys.includes(key))) throw Error("bindings cannot override scenario or policy");
    if (!["model", "telephone", "sms", "contacts"].every((key) => Object.hasOwn(bindings, key))) throw Error("complete explicit backend/contact bindings required");
    return freeze({ mode, simulated: mode === "fixture", definition: scenario, bindings: structuredClone(bindings) });
}
/** Verify the definition against the published target contract, not mock SDK success. */
export function checkMappings(scenario, sdk, wire) {
    const methods = new Map(sdk.methods.map((row) => [row.id, row]));
    const kinds = new Set(wire.properties.type.enum);
    const ids = new Set(scenario.stages.map((stage) => stage.id));
    if (ids.size !== scenario.stages.length || !ids.has(scenario.start)) throw Error("scenario graph IDs");
    const rows = [];
    for (const stage of scenario.stages) {
        if (stage.acceptance.some((id) => !/^A6-(0[1-9]|1[0-9]|2[0-8])$/.test(id))) throw Error("unknown A6 criterion");
        if (!stage.calls.length || !stage.gates.length || !stage.observations.length || !stage.acceptance.length) throw Error("incomplete stage");
        if (stage.next.some((next) => !ids.has(next))) throw Error("unknown next stage");
        for (const call of stage.calls) {
            if (!Object.hasOwn(scenario.actors, call.actor) || call.audience.some((actor) => !Object.hasOwn(scenario.actors, actor.replace(/\?$/, "")))) throw Error("unknown actor");
            const raw = call.method.startsWith("raw:");
            const method = raw ? { target_typescript: "ParticipantClient.raw.validate/request", canonical_type: call.method.slice(4) } : methods.get(call.method);
            if (!method || !kinds.has(method.canonical_type) || (raw && !sdk.rules.raw_canonical_access_available)) throw Error(`unknown public SDK mapping: ${call.method}`);
            rows.push({ stage: stage.id, method: call.method, sdk: method.target_typescript, canonical_type: method.canonical_type });
        }
    }
    const seen = new Set(), active = new Set();
    const visit = (id) => {
        if (active.has(id)) throw Error("cyclic scenario graph");
        if (seen.has(id)) return;
        active.add(id); scenario.stages.find((s) => s.id === id).next.forEach(visit);
        active.delete(id); seen.add(id);
    };
    visit(scenario.start);
    if (seen.size !== ids.size) throw Error("unreachable stage");
    return rows;
}

/** Common application stage definition for both bindings. This selects a path;
 * it does not satisfy gates, invoke SDK calls or claim their observations. */
export function rehearsalPath(bound, { consultation, observer = bound.definition.business_policy.observer_default } = {}) {
    if (!["takeover", "decline-return"].includes(consultation)) throw Error("explicit consultation outcome required");
    if (typeof observer !== "boolean") throw Error("explicit observer selection");
    const { definition } = bound;
    const stages = new Map(definition.stages.map((stage) => [stage.id, stage]));
    const result = [], seen = new Set();
    let id = definition.start;
    while (id !== undefined) {
        const stage = stages.get(id);
        if (!stage || seen.has(id)) throw Error("invalid rehearsal path");
        seen.add(id);
        result.push({
            ...structuredClone(stage),
            calls: stage.calls.map((call) => ({
                ...structuredClone(call),
                audience: call.audience.flatMap((actor) => actor === "observer?" ? observer ? ["observer"] : [] : [actor]),
            })).filter((call) => call.audience.length > 0),
        });
        if (id === "private-consultation") {
            if (!stage.next.includes(consultation)) throw Error("unsupported consultation branch");
            id = consultation;
        } else {
            if (stage.next.length > 1) throw Error("unresolved rehearsal branch");
            [id] = stage.next;
        }
    }
    return freeze(result);
}
