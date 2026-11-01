import test from "node:test";
import assert from "node:assert/strict";
import { operationFailure } from "../lib/transport_failure.mjs";

test("recorded native failure keeps original terminal state and public code without private fields", () => {
    const value = operationFailure({ problem: { code: 409, reason: "private-reason", category: "private-category" },
        operation: { operationId: "private-original-operation" }, observation: { state: "failed", revision: 7,
            key: { operationId: "private-original-operation" }, problem: { code: "unavailable", message: "private-description" } } });
    assert.deepEqual(value, { protocol_problem_code: 409,
        original_recorded_observation: { state: "failed", revision: 7, problem_code: "unavailable" } });
    assert.ok(!JSON.stringify(value).includes("private"));
});
test("unknown problem tokens are marked without copying user or provider text", () => {
    const value = operationFailure({ observation: { state: "failed", revision: 1,
        problem: { code: "private-token", message: "private-description" } } });
    assert.deepEqual(value, { original_recorded_observation: { state: "failed", revision: 1, problem_code_unrecognized: true } });
    assert.ok(!JSON.stringify(value).includes("private"));
});
test("missing or malformed observations cannot manufacture a recorded state", () => {
    for (const observation of [null, {}, { state: "private-token", revision: 1 }, { state: "failed", revision: -1 }, { state: "failed", revision: 1.5 }])
        assert.deepEqual(operationFailure({ observation, problem: { code: "private-token" } }), {});
    assert.deepEqual(operationFailure(null), {});
});
test("curation performs no operation methods or recovery calls", () => {
    let invoked = 0;
    const value = operationFailure({ observation: { state: "interrupted", revision: 2 }, status() { invoked++; }, wait() { invoked++; }, toJSON() { invoked++; } });
    assert.deepEqual(value, { original_recorded_observation: { state: "interrupted", revision: 2 } });
    assert.equal(invoked, 0);
});
