import assert from 'node:assert/strict';
import {test} from 'node:test';
import {originalSessionRetirement} from '../lib/native_session_retirement.mjs';

function session(id,{state='accepted',waitError,step=()=>{}}={}){
    const calls=[];let current=state;
    return {id,calls,
        async refresh(options){calls.push({kind:'refresh',options});step();return {id,state:current,topologyRevision:7};},
        async end(intent,options){calls.push({kind:'end',intent,options});step();return {
            referenceJson:()=>({operation_id:options.operationId,session_id:id,digest:'original-end-intent'}),
            async wait(waitOptions){calls.push({kind:'wait',options:waitOptions});step();if(waitError)throw waitError;
                current='ended';return {snapshot:{state:'completed'}};},
        };},
    };
}

test('failure cleanup retires both original accepted Sessions without changing programme failure',async()=>{
    const owner=originalSessionRetirement(),original=[session('original0'),session('original1')];
    original.forEach((s,i)=>owner.own(s,'planned-end'+i));
    const programme={passed:false,failure:{kind:'original-movement-deadline'}};
    const result=await owner.cleanup(owner.cleanupDeadline());
    programme.passed=Boolean(programme.passed&&result.passed);
    assert.equal(result.passed,true);assert.equal(programme.passed,false);
    assert.deepEqual(programme.failure,{kind:'original-movement-deadline'});
    for(const [i,s]of original.entries()){
        const ends=s.calls.filter(c=>c.kind==='end');assert.equal(ends.length,1);
        assert.equal(ends[0].options.operationId,'planned-end'+i);
        assert.deepEqual(ends[0].intent,{expectedTopologyRevision:7,reasonCode:200,reason:'Synthetic fixture complete'});
    }
});

test('successful original end and later cleanup share one original operation and receipt',async()=>{
    const owner=originalSessionRetirement(),first=session('original0'),second=session('original1');
    owner.own(first,'planned-end0');owner.own(second,'planned-end1');
    assert.equal((await owner.end(first.id,{timeoutMs:8000})).state,'ended');
    const result=await owner.cleanup(owner.cleanupDeadline());assert.equal(result.passed,true);
    assert.equal(first.calls.filter(c=>c.kind==='end').length,1);
    assert.equal(first.calls.filter(c=>c.kind==='wait').length,1);
    assert.match(result.sessions[0].original_reference_sha256,/^[a-f0-9]{64}$/);
    assert.equal(second.calls.find(c=>c.kind==='end').options.operationId,'planned-end1');
});

test('uncertain original end is recorded and cleanup cannot resubmit under any intent',async()=>{
    const error=Object.assign(Error('lost original response'),{kind:'transport-uncertain',certainty:'uncertain'});
    const owner=originalSessionRetirement(),s=session('original0',{waitError:error});owner.own(s,'planned-end0');
    await assert.rejects(owner.end(s.id,{timeoutMs:8000}),e=>e===error);
    const before=s.calls.length,result=await owner.cleanup(owner.cleanupDeadline());
    assert.equal(result.passed,false);assert.equal(s.calls.length,before);
    assert.equal(s.calls.filter(c=>c.kind==='end').length,1);
    assert.equal(s.calls.find(c=>c.kind==='end').options.operationId,'planned-end0');
    assert.equal(result.sessions[0].status,'uncertain');
    assert.deepEqual(result.sessions[0].failure,{kind:'transport-uncertain',certainty:'uncertain'});
});

test('already terminal original Session requires a current read and no mutation',async()=>{
    const owner=originalSessionRetirement(),s=session('original0',{state:'ended'});owner.own(s,'planned-end0');
    const result=await owner.cleanup(owner.cleanupDeadline());assert.equal(result.passed,true);
    assert.deepEqual(s.calls.map(c=>c.kind),['refresh']);assert.equal(result.sessions[0].status,'already-terminal');
});

test('Session and client cleanup share existing5s budget capped by original global deadline',async()=>{
    let now=1000;const owner=originalSessionRetirement({now:()=>now,globalDeadlineMs:1100});
    const original=[session('original0',{step:()=>{now+=10;}}),session('original1',{step:()=>{now+=10;}})];
    original.forEach((s,i)=>owner.own(s,'planned-end'+i));const cutoff=owner.cleanupDeadline();assert.equal(cutoff,1100);
    const result=await owner.cleanup(cutoff);assert.equal(result.passed,true);
    for(const s of original)for(const call of s.calls)assert.ok(call.options.timeoutMs>0&&call.options.timeoutMs<=100);
    assert.equal(owner.remainingCloseMs(cutoff),20);now=1200;assert.equal(owner.remainingCloseMs(cutoff),1);
});

test('an expired original cutoff cannot admit a fresh cleanup effect',async()=>{
    const owner=originalSessionRetirement({now:()=>1000,globalDeadlineMs:1000}),s=session('original0');
    owner.own(s,'planned-end0');const result=await owner.cleanup(owner.cleanupDeadline());
    assert.equal(result.passed,false);assert.deepEqual(s.calls,[]);
    assert.equal(result.sessions[0].failure.kind,'deadline');assert.equal(result.sessions[0].submission_attempted,false);
});
