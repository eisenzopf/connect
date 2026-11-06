import {test} from 'node:test';
import assert from 'node:assert/strict';
import {originalBrowserMatrixResult} from '../lib/browser_matrix_result.mjs';

test('one long original programme survives a short BiDi request deadline without another start',async()=>{
    let clock=0,requests=0;
    const original={state:'pending'},previous=globalThis.connectMatrixOriginalProgramme;
    globalThis.connectMatrixOriginalProgramme=original;
    try{
        const page={async evaluate(callback){
            requests++;
            const response=callback();
            // This regression reproduces the automation contract: awaiting the
            // actual 62-second work inside a request would exceed its 10s cap.
            assert.equal(typeof response?.then,'undefined','BiDi observation must be synchronous');
            return response;
        }};
        const result=await originalBrowserMatrixResult(page,{deadline:90000,now:()=>clock,wait:async ms=>{
            clock+=ms;
            if(clock>=62000){original.value={original:'retained-result',passed:false};original.state='fulfilled';}
        }});
        assert.deepEqual(result,{original:'retained-result',passed:false});
        assert.equal(clock,62000);assert.ok(requests>1);
        assert.equal(globalThis.connectMatrixOriginalProgramme,original);
    }finally{if(previous===undefined)delete globalThis.connectMatrixOriginalProgramme;else globalThis.connectMatrixOriginalProgramme=previous;}
});

test('pending original owner reaches its host deadline without resubmitting work',async()=>{
    let clock=0,observations=0;
    const page={async evaluate(){observations++;return {state:'pending',phase:'current-authority-refusals'};}};
    await assert.rejects(originalBrowserMatrixResult(page,{deadline:1000,now:()=>clock,wait:async ms=>{clock+=ms;}}),
        error=>error.name==='OriginalProgrammeDeadline'&&error.matrixPhase==='current-authority-refusals');
    assert.equal(clock,1000);assert.equal(observations,4);
});

test('original programme rejection preserves its failure and missing owner cannot pass',async()=>{
    await assert.rejects(originalBrowserMatrixResult({evaluate:async()=>({state:'rejected',phase:'six-browser-media-offers',
        error:{name:'OriginalMediaFailure',message:'decoder stopped',stack:'original private stack'}})},
        {deadline:1000,now:()=>0}),error=>error.name==='OriginalMediaFailure'&&error.message==='decoder stopped'
            &&error.stack==='original private stack'&&error.matrixPhase==='six-browser-media-offers');
    await assert.rejects(originalBrowserMatrixResult({evaluate:async()=>({state:'missing'})},
        {deadline:1000,now:()=>0}),/owner missing/);
});


test('one original report preserves repeated authority and media facts across the BiDi value boundary',async()=>{
    const previous=globalThis.connectMatrixOriginalProgramme;
    const conflict={passed:true,changed_original_intent_refused:true,stale_revision_refused:true};
    const decoded={passed:false,reason:'original-profile-budget-failure'};
    const value={passed:false,admission:{conflict},current_authority:{conflict,
        expiry:{passed:true},revocation:{passed:true},passed:true},decoded_media:decoded,programme:{decoded_media:decoded}};
    globalThis.connectMatrixOriginalProgramme={state:'fulfilled',value};let observations=0;
    try{
        const page={async evaluate(callback){
            observations++;const original=callback();assert.equal(typeof original?.then,'undefined');
            // The pinned BiDi deserializer returns undefined for a repeated
            // internalId-only object. Reproduce that loss at this automation
            // boundary without changing the original in-page report graph.
            const seen=new WeakSet();
            return JSON.parse(JSON.stringify(original,(_key,item)=>{
                if(item&&typeof item==='object'){if(seen.has(item))return undefined;seen.add(item);}return item;
            }));
        }};
        const result=await originalBrowserMatrixResult(page,{deadline:1000,now:()=>0});
        assert.equal(observations,1);assert.deepEqual(result,value);assert.equal(result.passed,false);
        assert.deepEqual(result.current_authority.conflict,conflict);
        assert.deepEqual(result.programme.decoded_media,decoded);
        assert.notEqual(result.current_authority.conflict,result.admission.conflict);
        assert.equal(value.current_authority.conflict,value.admission.conflict,'Original report ownership is unchanged');
    }finally{if(previous===undefined)delete globalThis.connectMatrixOriginalProgramme;else globalThis.connectMatrixOriginalProgramme=previous;}
});

test('JSON transfer cannot manufacture a missing authority subcase or turn a programme failure into a pass',async()=>{
    const previous=globalThis.connectMatrixOriginalProgramme;
    const original={passed:false,current_authority:{expiry:{passed:true},revocation:{passed:true},passed:true},
        failure:{name:'OriginalMovementDeadline'}};
    globalThis.connectMatrixOriginalProgramme={state:'fulfilled',value:original};
    try{
        const result=await originalBrowserMatrixResult({evaluate:async callback=>callback()},{deadline:1000,now:()=>0});
        assert.deepEqual(result,original);assert.equal('conflict'in result.current_authority,false);
        assert.equal(result.passed,false);assert.deepEqual(result.failure,{name:'OriginalMovementDeadline'});
    }finally{if(previous===undefined)delete globalThis.connectMatrixOriginalProgramme;else globalThis.connectMatrixOriginalProgramme=previous;}
});

test('an invalid final JSON transfer fails once without renewing or restarting the original programme',async()=>{
    const previous=globalThis.connectMatrixOriginalProgramme;
    try{
        await assert.rejects(originalBrowserMatrixResult({evaluate:async()=>({state:'fulfilled',value_json:undefined})},
            {deadline:1000,now:()=>0}),/Original browser report JSON unavailable/);
        await assert.rejects(originalBrowserMatrixResult({evaluate:async()=>({state:'fulfilled',value_json:'{'})},
            {deadline:1000,now:()=>0}),SyntaxError);
        const value={passed:false};value.cycle=value;
        const original=globalThis.connectMatrixOriginalProgramme={state:'fulfilled',value};let observations=0;
        await assert.rejects(originalBrowserMatrixResult({evaluate:async callback=>{observations++;return callback();}},
            {deadline:1000,now:()=>0}),TypeError);
        assert.equal(observations,1);assert.equal(globalThis.connectMatrixOriginalProgramme,original);
    }finally{if(previous===undefined)delete globalThis.connectMatrixOriginalProgramme;else globalThis.connectMatrixOriginalProgramme=previous;}
});
