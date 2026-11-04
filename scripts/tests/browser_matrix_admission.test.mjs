import assert from "node:assert/strict";
import {test} from "node:test";
import {browserMatrixAdmission,browserMatrixPlan,retireBrowserMatrixAdmission} from "../lib/browser_matrix_admission.mjs";
function fixture(create) {
    const roles=["participant-1","participant-2","participant-3","participant-4","replacement-1","controller-1","controller-2"];
    return Object.fromEntries(roles.map((role,index)=>[role,{identityId:"identity-"+(index<4?index:0),deviceId:"device-"+index,
        client:{role:"browser",capabilities:{refresh:async()=>({selectedTransport:"webtransport"})},conversations:{create}}}]));
}
test("an uncertain browser mutation preserves its original scoped operation and admits no later root",async()=>{
    const plan=browserMatrixPlan(),events=[];let submissions=0;
    const original={operationId:plan.operations.create,command:"conversation.create",scope:{}};
    const error=Object.assign(Error("uncertain"),{certainty:"uncertain",operation:original});
    const actors=fixture(async(_input,options)=>{submissions++;assert.equal(options.operationId,original.operationId);throw error;});
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async value=>events.push(value)}),e=>e===error);
    assert.equal(submissions,1);
    assert.deepEqual(events.map(value=>value.kind),["plan","intent","failure"]);
    assert.deepEqual(events.at(-1).operation,original);
    assert.equal(events.at(-1).operationId,plan.operations.create);
});
test("failure to persist the original browser intent prevents its mutation submission",async()=>{
    const plan=browserMatrixPlan();let submissions=0;
    const actors=fixture(async()=>{submissions++;throw Error("must not submit");});
    const persist=Error("intent could not be saved");
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async event=>{
        if(event.kind==="intent")throw persist;
    }}),e=>e===persist);
    assert.equal(submissions,0);
});
test("an accepted browser root retains the exact reference when its original wait is uncertain",async()=>{
    const plan=browserMatrixPlan(),events=[];let submissions=0;
    const reference={schema:"connect-operation-reference/1",operation_id:plan.operations.create,command:"conversation.create",scope:{}};
    const error=Object.assign(Error("wait uncertain"),{certainty:"uncertain"});
    const actors=fixture(async()=>{submissions++;return {referenceJson:()=>reference,wait:async()=>{throw error;}};});
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async event=>events.push(event)}),e=>e===error);
    assert.equal(submissions,1);
    assert.deepEqual(events.map(value=>value.kind),["plan","intent","accepted","failure"]);
    assert.deepEqual(events.at(-1).reference,reference);
});

test("missed browser terminal deadline still joins original cleanup and cannot turn green afterward",async()=>{
    let clock=0,closeCalls=0;
    const plan=browserMatrixPlan(),actors={owner:{client:{close:async()=>{closeCalls++;if(closeCalls===1)throw Error("deadline");}}}};
    const connection={id:plan.routes[0].connection,refresh:async()=>({streamRevision:1}),end:async()=>({
        referenceJson:()=>({operation_id:plan.routes[0].end}),wait:async()=>{clock=6000;return {snapshot:{state:"completed"}};}})};
    const result=await retireBrowserMatrixAdmission({state:{connections:[connection],sessions:[]},actors,plan,journal:async()=>{},now:()=>clock});
    assert.equal(closeCalls,2);assert.equal(result.passed,false);assert.equal(result.originalClientsClosed,1);assert.equal(result.elapsedMs,6000);
});

test("browser terminal ends both original Sessions before client closure without resubmitting uncertainty",async()=>{
    for(const uncertain of [false,true]){
        const plan=browserMatrixPlan(),events=[],submitted=[];let clock=100,closed=false;
        const sessions=plan.sessions.map((s,index)=>({id:s.id,refresh:async()=>({topologyRevision:10+index}),
            end:async(input,options)=>{
                assert.equal(input.expectedTopologyRevision,10+index);assert.equal(options.operationId,s.end);
                assert.ok(options.timeoutMs<=4900);assert.equal(closed,false);submitted.push(s.end);
                return {referenceJson:()=>({operation_id:s.end}),wait:async()=>{
                    if(index===0&&uncertain)throw Object.assign(Error('uncertain'),{certainty:'uncertain'});
                    return {snapshot:{state:'completed'}};
                }};
            }}));
        const actors={owner:{client:{close:async()=>{assert.equal(submitted.length,2);closed=true;clock=400;}}}};
        const result=await retireBrowserMatrixAdmission({state:{connections:[],sessions},actors,plan,
            journal:async event=>events.push(event),now:()=>clock,terminalMs:0});
        assert.deepEqual(submitted,plan.sessions.map(s=>s.end));assert.ok(closed);
        assert.equal(result.originalSessionsEnded,uncertain?1:2);assert.equal(result.passed,!uncertain);
        assert.equal(events.filter(e=>e.kind==='session-end-accepted').length,2);
    }
});
