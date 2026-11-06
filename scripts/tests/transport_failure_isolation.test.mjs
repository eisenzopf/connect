import assert from "node:assert/strict";
import test from "node:test";
import { recordPrivateTargetCleanup, privateTargetCleanup, isolatedPrivateTargetFailure, blockedMovementSlots, emitOriginalObservation, observationFailure, isolatedProgrammeFailure } from "../lib/transport_failure_isolation.mjs";
const facts = () => ({ format: "connect/original-private-target-cleanup/1", logical_outcome: "not-submitted", submission_definitively_refused: true, completion_observed: false, transferred: false,
    original_root_completed: false, original_source_owner_joined: false,
    worker_started: false, target_worker_joined: true, target_worker_forced: false,
    target_executable_unchanged: true, target_config_unchanged: true });
test("only the same original error with actual complete private cleanup can be isolated", () => {
    const original=Error("same original"); const value=facts();
    recordPrivateTargetCleanup(original,value);
    value.logical_outcome="unknown";
    assert.equal(isolatedPrivateTargetFailure(original),true);
    assert.equal(isolatedPrivateTargetFailure(Error("same original")),false);
    assert.equal(Object.isFrozen(privateTargetCleanup(original).facts),true);
    for (const [key,bad] of [["transferred",true],["original_root_completed",true],
        ["original_source_owner_joined",true],["logical_outcome","unknown"],["submission_definitively_refused",false],["completion_observed",true],["worker_started",true],
        ["target_worker_forced",true],["target_executable_unchanged",false],["target_config_unchanged",false]]) {
        const error=Error(key);recordPrivateTargetCleanup(error,{...facts(),[key]:bad});
        assert.equal(isolatedPrivateTargetFailure(error),false,key);
    }
});
test("original cleanup rejection remains fatal and retains its exact identity alongside root failure", () => {
    const original=Error("root"),close=Error("original join refused");
    recordPrivateTargetCleanup(original,facts(),close);
    assert.equal(privateTargetCleanup(original).cleanupError,close);
    assert.equal(isolatedPrivateTargetFailure(original),false);
});
test("blocked return keeps the prewritten logical ID without submitting or inventing an admitted owner", () => {
    const plan=[{sourceIndex:0,ordinal:0,operationId:"move-original-0",scheduled_start_ms:15000},
        {sourceIndex:0,ordinal:1,operationId:"move-original-1",scheduled_start_ms:40000},
        {sourceIndex:4,ordinal:1,operationId:"move-other-1",scheduled_start_ms:40000}];
    const slots=blockedMovementSlots(plan,0,0,23000);
    assert.equal(slots.length,1);assert.equal(slots[0].operation_id,"move-original-1");
    assert.equal(slots[0].scheduled_start_ms,40000);assert.equal(slots[0].mutation_submissions,0);
    assert.equal(slots[0].replacement_owner_inferred,false);
    assert.equal(blockedMovementSlots(plan,0,1,48000).length,0);
});

test("throwing observation retains exact original error plus exact fatal observer error and revokes isolation", () => {
    const original=Error("original refusal"),observer=Error("original observer failed");
    recordPrivateTargetCleanup(original,facts());assert.equal(isolatedProgrammeFailure(original),true);
    const receipt={persisted:false};
    const result=emitOriginalObservation(() => {throw observer;},[Object.freeze({receipt})],original);
    receipt.persisted=result.emitted;Object.freeze(receipt);
    assert.equal(result.error,original);assert.equal(result.observerError,observer);
    assert.equal(observationFailure(original),observer);assert.equal(isolatedProgrammeFailure(original),false);
    assert.equal(receipt.persisted,false);
});
test("throwing observation with no operation error preserves exact observer identity and stays fatal", () => {
    const observer=Error("no original work error");
    const result=emitOriginalObservation(() => {throw observer;},[{}]);
    assert.equal(result.emitted,false);assert.equal(result.error,observer);
    assert.equal(observationFailure(observer),observer);assert.equal(isolatedProgrammeFailure(observer),false);
});
test("an unobserved completed outcome is unknown and never a positive noncommit proof", () => {
    const original=Object.assign(Error("original deadline"),{certainty:"read-only"});
    recordPrivateTargetCleanup(original,{...facts(),logical_outcome:"unknown",submission_definitively_refused:false,
        worker_started:true,target_worker_joined:true,completion_observed:false});
    assert.equal(isolatedPrivateTargetFailure(original),false);
});
