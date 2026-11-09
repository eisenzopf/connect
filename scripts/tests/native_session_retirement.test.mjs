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


test('independent client closes all start under one original cutoff before any gated reply', async () => {
    let clock = 1000;
    const owner = originalSessionRetirement({now: () => clock, globalDeadlineMs: 1050});
    const starts = [], releases = [];
    const clients = Array.from({length: 3}, (_, index) => ({
        close(options) {
            starts.push({index, timeoutMs: options.timeoutMs});
            return new Promise(resolve => releases.push(resolve));
        },
    }));
    const closing = owner.closeClients(clients, 1100);
    assert.deepEqual(starts, [{index: 0, timeoutMs: 50}, {index: 1, timeoutMs: 50}, {index: 2, timeoutMs: 50}]);
    assert.equal(releases.length, 3);
    clock = 1049;
    for (const release of releases) release();
    const result = await closing;
    assert.equal(result.passed, true);
    assert.equal(result.original_cutoff_monotonic_ms, 1050);
    assert.ok(result.clients.every(client => client.status === 'fulfilled' && client.deadline_met));
});

test('one original client close failure joins every sibling and preserves the original programme failure', async () => {
    const original = Error('original programme failure');
    const closeError = Object.assign(Error('private close detail'), {kind: 'deadline', certainty: 'read-only'});
    const programme = {passed: false, failure: original};
    const owner = originalSessionRetirement({now: () => 1000, globalDeadlineMs: 1050});
    const releases = [];
    const clients = Array.from({length: 3}, (_, index) => ({
        close() {
            if (index === 1) return Promise.reject(closeError);
            return new Promise(resolve => releases.push(resolve));
        },
    }));
    let settled = false;
    const closing = owner.closeClients(clients, 1050).then(result => { settled = true; return result; });
    await Promise.resolve();
    assert.equal(settled, false);
    assert.equal(releases.length, 2);
    for (const release of releases) release();
    const result = await closing;
    programme.passed = Boolean(programme.passed && result.passed);
    assert.equal(result.passed, false);
    assert.deepEqual(result.clients.map(client => client.status), ['fulfilled', 'rejected', 'fulfilled']);
    assert.deepEqual(result.clients[1].failure, {name: 'Error', kind: 'deadline', certainty: 'read-only'});
    assert.equal(programme.passed, false);
    assert.equal(programme.failure, original);
    assert.equal(JSON.stringify(result).includes('private close detail'), false);
});

test('late client completion and an expired original cutoff cannot qualify or renew the cleanup budget', async () => {
    let clock = 1000, release;
    const owner = originalSessionRetirement({now: () => clock, globalDeadlineMs: 1050});
    const calls = [];
    const closing = owner.closeClients([{close(options) {
        calls.push(options.timeoutMs);
        return new Promise(resolve => { release = resolve; });
    }}], 1100);
    clock = 1051;
    release();
    const late = await closing;
    assert.equal(late.passed, false);
    assert.equal(late.clients[0].status, 'fulfilled');
    assert.equal(late.clients[0].deadline_met, false);
    const expired = await owner.closeClients([{async close(options) { calls.push(options.timeoutMs); }}], 1100);
    assert.deepEqual(calls, [50, 1]);
    assert.equal(expired.original_cutoff_monotonic_ms, 1050);
    assert.equal(expired.passed, false);
});


test('independent SDK close releases blocked original Session cleanup before its owner closes', async () => {
    const retirement = originalSessionRetirement({now: () => 1000, globalDeadlineMs: 1100});
    let release, independentStarted = false, ownerClosed = false, releasedByIndependent = false;
    const blocked = new Promise(resolve => { release = resolve; });
    const first = session('original0'), second = session('original1');
    const refresh = first.refresh;
    let firstRead = true;
    first.refresh = async options => {
        assert.equal(ownerClosed, false);
        if (firstRead) { firstRead = false; await blocked; }
        return refresh(options);
    };
    const primary = {async close() {
        assert.deepEqual(first.calls.map(call => call.kind), ['refresh', 'end', 'wait', 'refresh']);
        assert.deepEqual(second.calls.map(call => call.kind), ['refresh', 'end', 'wait', 'refresh']);
        ownerClosed = true;
    }};
    const independent = {async close() {
        independentStarted = true;
        releasedByIndependent = true;
        release();
    }};
    retirement.own(first, 'planned-end0', primary);
    retirement.own(second, 'planned-end1', primary);
    const pending = retirement.cleanupAndCloseClients([primary, independent], 1100);
    let result;
    try {
        assert.equal(independentStarted, true, 'Independent close waited behind Session cleanup');
        assert.equal(releasedByIndependent, true);
        assert.equal(ownerClosed, false, 'Session owner closed before its original end calls settled');
    } finally {
        release();
        result = await pending;
    }
    assert.equal(result.sessions.passed, true);
    assert.equal(result.clients.passed, true);
    assert.equal(ownerClosed, true);
    assert.deepEqual(result.clients.clients.map(client => client.index), [0, 1]);
    for (const original of [first, second]) {
        assert.equal(original.calls.filter(call => call.kind === 'end').length, 1);
        assert.equal(original.calls.find(call => call.kind === 'end').options.operationId,
            original === first ? 'planned-end0' : 'planned-end1');
    }
});

test('dependent SDK cleanup keeps the original cutoff and cannot renew or duplicate a Session end', async () => {
    let clock = 1000, releaseWait, releaseClose, ownerCloses = 0, endCalls = 0, enterWait, enterOwnerClose;
    const waitEntered = new Promise(resolve => { enterWait = resolve; });
    const ownerCloseEntered = new Promise(resolve => { enterOwnerClose = resolve; });
    const waiting = new Promise(resolve => { releaseWait = resolve; });
    const closing = new Promise(resolve => { releaseClose = resolve; });
    const optionsSeen = [];
    const retirement = originalSessionRetirement({now: () => clock, globalDeadlineMs: 1050});
    const original = {
        id: 'original0',
        async refresh(options) { optionsSeen.push(options.timeoutMs); return {id: this.id, state: 'accepted', topologyRevision: 7}; },
        async end(intent, options) {
            endCalls++;
            assert.equal(options.operationId, 'planned-end0');
            assert.equal(intent.expectedTopologyRevision, 7);
            optionsSeen.push(options.timeoutMs);
            return {referenceJson: () => ({operation_id: 'planned-end0'}), async wait(waitOptions) {
                optionsSeen.push(waitOptions.timeoutMs); enterWait();
                await waiting; return {snapshot: {state: 'completed'}};
            }};
        },
    };
    const primary = {async close(options) { ownerCloses++; enterOwnerClose(); assert.equal(options.timeoutMs, 1); }};
    let independentStarted = false;
    const independent = {close() { independentStarted = true; return closing; }};
    retirement.own(original, 'planned-end0', primary);
    const pending = retirement.cleanupAndCloseClients([primary, independent], 1050);
    await waitEntered;
    const independentStartedBeforeSession = independentStarted, originalEndCalls = endCalls;
    clock = 1060;
    releaseWait();
    let settled = false;
    pending.then(() => { settled = true; }, () => { settled = true; });
    await ownerCloseEntered;
    const settledBeforeOriginalClose = settled;
    releaseClose();
    const result = await pending;
    assert.equal(independentStartedBeforeSession, true);
    assert.equal(originalEndCalls, 1);
    assert.equal(settledBeforeOriginalClose, false, 'An original independent close was abandoned');
    assert.equal(result.sessions.passed, false);
    assert.equal(result.sessions.sessions[0].failure.kind, 'deadline');
    assert.equal(result.clients.passed, false);
    assert.equal(result.clients.original_cutoff_monotonic_ms, 1050);
    assert.equal(ownerCloses, 1);
    assert.ok(optionsSeen.every(value => value > 0 && value <= 50));
    const again = await retirement.cleanup(5000);
    assert.equal(again.passed, false);
    assert.equal(endCalls, 1);
});

test('a failed independent SDK close joins every original sibling after Session cleanup', async () => {
    const retirement = originalSessionRetirement({now: () => 1000, globalDeadlineMs: 1100});
    const original = session('original0');
    let ownerClosed = false, release, thirdStarted = false, enterOwnerClose;
    const ownerCloseEntered = new Promise(resolve => { enterOwnerClose = resolve; });
    const primary = {async close() { ownerClosed = true; enterOwnerClose(); }};
    const refusal = Object.assign(Error('private close detail'), {kind: 'cleanup-incomplete', certainty: 'read-only'});
    const failed = {async close() { throw refusal; }};
    const third = {close() { thirdStarted = true; return new Promise(resolve => { release = resolve; }); }};
    retirement.own(original, 'planned-end0', primary);
    const pending = retirement.cleanupAndCloseClients([primary, failed, third], 1100);
    const thirdStartedBeforeSession = thirdStarted;
    let settled = false;
    pending.then(() => { settled = true; }, () => { settled = true; });
    await ownerCloseEntered;
    const settledBeforeOriginalClose = settled;
    release();
    const result = await pending;
    assert.equal(thirdStartedBeforeSession, true);
    assert.equal(settledBeforeOriginalClose, false);
    assert.equal(result.sessions.passed, true);
    assert.equal(ownerClosed, true);
    assert.equal(result.clients.passed, false);
    assert.deepEqual(result.clients.clients.map(client => client.status), ['fulfilled', 'rejected', 'fulfilled']);
    assert.deepEqual(result.clients.clients[1].failure, {name: 'Error', kind: 'cleanup-incomplete', certainty: 'read-only'});
    assert.equal(original.calls.filter(call => call.kind === 'end').length, 1);
});

test('cleanup before any original Session exists closes every original SDK without an invented dependency', async () => {
    const retirement = originalSessionRetirement({now: () => 1000, globalDeadlineMs: 1100});
    const calls = [];
    const clients = [0, 1].map(index => ({async close(options) { calls.push([index, options.timeoutMs]); }}));
    const result = await retirement.cleanupAndCloseClients(clients, 1100);
    assert.deepEqual(calls, [[0, 100], [1, 100]]);
    assert.equal(result.sessions.passed, true);
    assert.deepEqual(result.sessions.sessions, []);
    assert.equal(result.clients.passed, true);
});
