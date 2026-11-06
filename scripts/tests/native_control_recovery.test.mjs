import {test} from 'node:test';
import assert from 'node:assert/strict';
import {nativeControlRecovery} from '../lib/native_control_recovery.mjs';

function fixture(fault={}){
    const live=new Set([1,2,3,4,5]),observed=new Set(live),events=[],checks=[],report={},clients=new Map();let next=6;
    const errors=[];let releaseReserve;const reserveGate=new Promise(resolve=>{releaseReserve=resolve;});
    const reference={operationId:'original-create',recordId:'original-record',digest:'original-digest'};
    const expectedParticipants=['p1','p2','p3','p4'],expectedHistory=['before','included','from4','after'];
    const failure=kind=>{const error=Object.assign(Error('Original authority or close refusal'),{name:'NodeSdkError',kind,certainty:'read-only'});
        errors.push(error);return error;};
    function client(role,pid){return {capabilities:{async refresh(options){
        assert.equal(options.timeoutMs,8000);events.push('authority:'+role+':'+pid);
        if(fault.auth===role)throw failure('authentication');return {selectedTransport:fault.transport===role?'wss':'quic'};
    }},operations:{restore(value,kind){
        events.push('restore:'+role);assert.equal(role,'replacement-1');assert.equal(kind,'conversation_create');
        assert.equal(value,reference,'Use the original retained logical reference');
        if(fault.mutateReference)value.recordId='changed-record';
        return {referenceJson:()=>fault.reference?{...value,digest:'changed-digest'}:value};
    }},operationKinds:{conversation_create:'conversation_create'},async close(options){
        assert.equal(options.timeoutMs,5000);events.push('close-start:'+role+':'+pid);
        if(fault.delayReserve&&pid===5)await reserveGate;
        if(fault.close===pid)throw failure('transport-uncertain');
        if(fault.survive!==pid)live.delete(pid);events.push('close-joined:'+role+':'+pid);
    }};}
    clients.set('participant-1',client('participant-1',1));clients.set('replacement-1',client('replacement-1',5));
    for(let i=2;i<=4;i++)clients.set('participant-'+i,client('participant-'+i,i));
    const arguments_={clients,configuration:{'participant-1':{path:'creator.config',sha256:'creator-digest'},
        'replacement-1':{path:'reserve.config',sha256:'reserve-digest'}},
        configDigest:async path=>fault.config===path?'changed-config':path==='creator.config'?'creator-digest':'reserve-digest',
        openClient:path=>{const role=path==='creator.config'?'participant-1':'replacement-1',pid=next++;
            events.push('open:'+role+':'+pid);live.add(pid);return client(role,pid);},
        helpers:async()=>{for(const pid of live)observed.add(pid);return new Set(live);},helperExists:async pid=>live.has(pid),
        complete:async operation=>{assert.ok(operation.referenceJson);events.push('recover-completed');return {
            id:fault.conversation?'wrong-conversation':'original-conversation',async refresh(options){
                assert.equal(options.timeoutMs,8000);events.push('current-membership');
                return {participants:expectedParticipants.map((id,i)=>({id:fault.membership&&i===3?'foreign-participant':id,active:true}))};
            }};},history:async owner=>{assert.equal(owner,clients.get('replacement-1'));events.push('current-history');
            return fault.history?[...expectedHistory,'foreign-history']:[...expectedHistory].sort();},
        originalReference:reference,conversationId:'original-conversation',expectedHistory,expectedParticipants,options:{timeoutMs:8000},
        observe:(stage,owners)=>events.push('observe:'+stage+':'+owners.size),check:value=>checks.push(value),
        stage:value=>events.push('stage:'+value),report};
    return {arguments_,live,observed,events,checks,report,clients,releaseReserve,errors};
}
const markerActivation='independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins';
const markerRecovery='independent-device-recovers-original-create-and-history-after-original-helper-close';

test('actual original joins, unchanged recovery and recovery join precede the sole current creator reactivation',async()=>{
    const f=fixture(),creator=await nativeControlRecovery(f.arguments_);
    assert.equal(f.report.passed,true);assert.equal(f.report.mutation_replayed,false);
    assert.equal(f.clients.get('participant-1'),creator);assert.equal(f.clients.has('replacement-1'),false);
    assert.equal(f.live.size,4);assert.equal(f.observed.size,7,'Both extra actual helper incarnations are counted');
    assert.ok(f.checks.includes(markerActivation));assert.ok(f.checks.includes(markerRecovery));
    for(const [earlier,later] of [['close-joined:replacement-1:5','close-start:participant-1:1'],
        ['close-joined:participant-1:1','open:replacement-1:6'],['current-history','close-start:replacement-1:6'],
        ['close-joined:replacement-1:6','open:participant-1:7']])
        assert.ok(f.events.indexOf(earlier)<f.events.indexOf(later),earlier+' must precede '+later);
    assert.equal(f.events.filter(x=>x.startsWith('restore:')).length,1,'No logical mutation resubmission or second recovery');
    assert.equal(f.report.current_recovery_authority_verified,true);assert.equal(f.report.creator_current_authority_verified,true);
});

test('pending original reserve close cannot start any independent helper or creator reactivation',async()=>{
    const f=fixture({delayReserve:true}),work=nativeControlRecovery(f.arguments_);let settled=false;work.finally(()=>{settled=true;});
    for(let i=0;i<8;i++)await Promise.resolve();
    assert.equal(settled,false);assert.equal(f.events.some(x=>x.startsWith('open:')),false);
    f.releaseReserve();await work;assert.equal(f.report.passed,true);
});

test('each unchanged config, current authority, original reference and exact current history proof is mandatory',async()=>{
    const cases=[{config:'reserve.config'},{auth:'replacement-1'},{transport:'replacement-1'},
        {reference:true},{mutateReference:true},{conversation:true},{membership:true},{history:true}];
    for(const fault of cases){
        const f=fixture(fault);await nativeControlRecovery(f.arguments_);
        assert.equal(f.report.passed,false,JSON.stringify(fault));assert.ok(f.report.failure,JSON.stringify(fault));
        assert.equal(f.checks.includes(markerRecovery),false,JSON.stringify(fault));
        assert.equal(f.clients.has('replacement-1'),false,'Original recovery owner joins even when its proof fails');
        assert.equal(f.live.size,4,'Only the four actual participant owners may reach the independent media phase');
        assert.equal(f.report.creator_current_authority_verified,true,'A failed recovery need not erase independently authenticated media');
        if(fault.auth)assert.equal(f.report.failure.kind,'authentication');
    }
});

test('a real old-helper close refusal or surviving helper prevents all media ownership and cannot pass recovery',async()=>{
    for(const fault of [{close:5},{close:1},{close:6},{survive:5},{survive:1},{survive:6}]){
        const f=fixture(fault);await assert.rejects(nativeControlRecovery(f.arguments_));
        assert.equal(f.report.passed,false,JSON.stringify(fault));
        assert.equal(f.events.some(x=>x==='open:participant-1:7'),false,'No creator reactivation across unjoined cleanup');
    }
});

test('creator reactivation also requires the unchanged original configuration and current authority',async()=>{
    for(const fault of [{config:'creator.config'},{auth:'participant-1'},{transport:'participant-1'}]){
        const f=fixture(fault);await assert.rejects(nativeControlRecovery(f.arguments_));
        assert.equal(f.report.passed,false);assert.equal(f.report.creator_current_authority_verified,undefined);
        assert.equal(f.clients.has('replacement-1'),false,'The original successful recovery helper is already joined');
    }
});


test('each original helper-close refusal reaches the caller as the exact original error object',async()=>{
    for(const pid of [5,1,6]){
        const f=fixture({close:pid});
        await assert.rejects(nativeControlRecovery(f.arguments_),error=>error===f.errors[0]);
        assert.equal(f.errors.length,1,'Never repeat the failed original close or substitute another error');
        assert.equal(f.errors[0].kind,'transport-uncertain');assert.equal(f.errors[0].certainty,'read-only');
        assert.equal(f.report.passed,false);assert.equal(f.report.creator_current_authority_verified,undefined);
        const original=pid===6?f.report.cleanup_failure:f.report.failure;
        assert.equal(original.kind,f.errors[0].kind);assert.equal(original.certainty,f.errors[0].certainty);
        assert.equal(f.events.some(value=>value==='open:participant-1:7'),false);
    }
});
