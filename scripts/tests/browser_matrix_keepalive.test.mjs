import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixKeepalive,retireBrowserMatrixProgramme,closeBrowserMatrixProgramme} from '../lib/browser_matrix_programme.mjs';
const deferred=()=>{let resolve;const promise=new Promise(value=>{resolve=value;});return {promise,resolve};};
const flush=async()=>{for(let n=0;n<80;n++)await Promise.resolve();};

// Original request/epoch/clock ownership models only; these produce no media or
// transport-quality evidence and do not contact any service.
async function fixture(actors,run){
    const saved={setTimeout:globalThis.setTimeout,clearTimeout:globalThis.clearTimeout};
    const timers=new Map();let next=0,now=0,owner;
    globalThis.setTimeout=(work,delay)=>{assert.equal(delay,20000);const id=++next;timers.set(id,{work,at:now+delay});return id;};
    globalThis.clearTimeout=id=>timers.delete(id);
    try{
        owner=browserMatrixKeepalive(actors);
        await run({owner,timers,advance:async at=>{
            now=at;
            for(const [id,timer] of [...timers])if(timer.at<=at){timers.delete(id);timer.work();}
            await flush();
        }});
    }finally{await owner?.close().catch(()=>{});Object.assign(globalThis,saved);}
}

test('one original keepalive refusal does not retry or stop an unaffected owner',async()=>{
    const original=Object.assign(Error('original refusal'),{kind:'transport-uncertain',certainty:'read-only'}),calls=[];
    const client=(role,work)=>({transport:{physicalEpoch:1},async keepAlive(options){
        assert.deepEqual(options,{timeoutMs:3000});calls.push(role);return work();
    }});
    const actors={failed:{client:client('failed',()=>{throw original;})},live:{client:client('live',()=>{})}};
    await fixture(actors,async f=>{
        await f.advance(20000);assert.deepEqual(calls,['failed','live']);
        assert.throws(()=>f.owner.check(),error=>error===original);
        await f.advance(40000);assert.deepEqual(calls,['failed','live','live']);
        assert.equal(actors.live.client.transport.physicalEpoch,1);
        // An independently changed physical epoch is refused before a new
        // request; it cannot be adopted as a reconnect by this original owner.
        actors.live.client.transport.physicalEpoch=2;
        await f.advance(60000);assert.deepEqual(calls,['failed','live','live']);
        const report=f.owner.observation();assert.equal(report.failed,true);
        assert.equal(report.requests,3);assert.equal(report.rounds,3);
        assert.deepEqual(report.failures.map(value=>value.owner),['failed','live']);
        assert.equal(report.failures[0].kind,original.kind);assert.equal(f.timers.size,0);
        await assert.rejects(f.owner.close(),error=>error===original);
        await f.advance(100000);assert.deepEqual(calls,['failed','live','live']);
    });
});

test('original pending keepalive batch joins before close and never overlaps another batch',async()=>{
    const original=Error('first original rejection'),pending=deferred(),calls=[];
    const actors={failed:{client:{transport:{physicalEpoch:1},async keepAlive(){calls.push('failed');throw original;}}},
        pending:{client:{transport:{physicalEpoch:1},async keepAlive(){calls.push('pending');await pending.promise;}}}};
    await fixture(actors,async f=>{
        try{
            await f.advance(20000);assert.equal(f.timers.size,0);
            await f.advance(40000);assert.deepEqual(calls,['failed','pending']);
            let settled=false;const closing=f.owner.close();
            assert.equal(f.owner.close(),closing,'Every close retains the same original join');
            void closing.then(()=>{settled=true;},()=>{settled=true;});
            await flush();assert.equal(settled,false,'Original pending request must actually join');
            pending.resolve();await assert.rejects(closing,error=>error===original);
            assert.equal(f.owner.observation().rounds,1);assert.equal(f.owner.observation().stopped,true);
            await f.advance(80000);assert.deepEqual(calls,['failed','pending']);
        }finally{pending.resolve();await flush();}
    });
});

test('original retirement is attempted once after a failed reachability close joins',async()=>{
    const pending=deferred(),original=Object.assign(Error('original keepalive failure'),{kind:'deadline',certainty:'read-only'});
    const report={passed:true},receipt={passed:true,originalEndsCompleted:6,originalSessionsEnded:2,originalClientsClosed:7};
    let closes=0,retirements=0;
    const work=retireBrowserMatrixProgramme({report,keepalive:{async close(){closes++;await pending.promise;throw original;}},
        retire:async()=>{retirements++;return receipt;}});
    try{
        await flush();assert.equal(retirements,0,'Do not abandon the original pending close');
        pending.resolve();assert.equal(await work,receipt);assert.equal(closes,1);assert.equal(retirements,1);
        assert.equal(report.passed,false);assert.deepEqual(report.keepalive_failure,
            {name:original.name,kind:'deadline',certainty:'read-only',problem:null});
    }finally{pending.resolve();await work.catch(()=>{});}
    // A real retirement refusal is still a refusal, never a fabricated receipt.
    const retirementFailure=Error('original end refusal');
    await assert.rejects(retireBrowserMatrixProgramme({report,keepalive:{async close(){throw original;}},
        retire:async()=>{throw retirementFailure;}}),error=>error===retirementFailure);
    assert.equal(report.passed,false);assert.equal(report.keepalive_failure.kind,'deadline');
});

test('a recorded keepalive failure cannot discard the independent report during final cleanup',async()=>{
    const original=Object.assign(Error('original reachability loss'),{kind:'transport-uncertain',certainty:'read-only'});
    const source=deferred(),events=[],report={passed:true,decoded_media:{observation:'original-independent-result'}};
    let closeWork;
    const keepalive={close(){return closeWork??=(async()=>{events.push('original-keepalive-close');throw original;})();}};
    const observer={async close(){events.push('renderer-close');}};
    const sources={async close(){events.push('source-close');await source.promise;}};
    const finish=(async()=>{
        try{
            report.retirement=await retireBrowserMatrixProgramme({keepalive,report,retire:async()=>{
                events.push('original-retirement');return {originalEndsCompleted:6,originalSessionsEnded:2,originalClientsClosed:7};
            }});
            return report;
        }finally{
            const closed=await closeBrowserMatrixProgramme({keepalive,observer,sources,report});
            if('failure' in closed)throw closed.failure;
        }
    })();
    let settled=false;void finish.then(()=>{settled=true;},()=>{settled=true;});
    try{
        await flush();assert.equal(settled,false,'Final source close still has to join');
        assert.deepEqual(events,['original-keepalive-close','original-retirement','renderer-close','source-close']);
        source.resolve();assert.equal(await finish,report);assert.equal(report.passed,false);
        assert.equal(report.keepalive_failure.kind,'transport-uncertain');
        assert.equal(report.decoded_media.observation,'original-independent-result');assert.equal(report.retirement.originalEndsCompleted,6);
    }finally{source.resolve();await finish.catch(()=>{});}
});

test('known reachability failure never hides a new cleanup refusal or skips another owner',async()=>{
    const reachability=Error('recorded reachability refusal'),renderer=Error('new renderer close refusal'),calls=[];
    const report={passed:false,keepalive_failure:{name:reachability.name}};
    const result=await closeBrowserMatrixProgramme({report,
        keepalive:{close(){calls.push('reachability');throw reachability;}},
        observer:{close(){calls.push('renderer');throw renderer;}},
        sources:{async close(){calls.push('source');}}});
    assert.equal(result.failure,renderer);assert.deepEqual(calls,['reachability','renderer','source']);
    assert.deepEqual(result.outcomes.map(value=>value.status),['rejected','rejected','fulfilled']);
    const unrecorded=await closeBrowserMatrixProgramme({report:{passed:false},keepalive:{close(){throw reachability;}}});
    assert.equal(unrecorded.failure,reachability,'A previously unrecorded failure must still propagate');
    const failed=await closeBrowserMatrixProgramme({report,previousFailure:true,observer:{close(){throw renderer;}}});
    assert.equal('failure' in failed,false);assert.equal(failed.outcomes[1].status,'rejected','Failure-path diagnostics retain the actual refusal');
});
