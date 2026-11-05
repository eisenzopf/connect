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
