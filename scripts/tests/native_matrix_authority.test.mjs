import assert from 'node:assert/strict';
import {test} from 'node:test';
import {nativeAuthorityCase} from '../lib/native_matrix_authority.mjs';

function fixture({freshKind='authentication',liveKind='cancelled',closeFailure=false,changeReference=false}={}){
    let expired=false,now=100,mutations=0,opens=0,closes=0,reads=0,saved;
    const reference={operationId:'original',command:'conversation.create',scope:{conversationId:'room'}};
    const snapshot={state:'completed',recordId:'record'};
    const open=()=>{
        const index=++opens;
        const status=async()=>{reads++;if(expired)throw {kind:index===1?liveKind:freshKind,certainty:'read-only'};return snapshot;};
        const operation={referenceJson:()=>structuredClone(reference),wait:async()=>({snapshot,value:{id:'room'}}),status};
        return {capabilities:{refresh:async()=>({selectedTransport:'quic'})},
            operationKinds:{conversation_create:{}},
            conversations:{create:async(_,options)=>{mutations++;assert.equal(options.operationId,'original');return operation;}},
            operations:{restore:original=>{assert.deepEqual(original,reference);return {...operation,
                referenceJson:()=>changeReference?{...original,operationId:'other'}:structuredClone(original)};}},
            close:async()=>{closes++;if(closeFailure)throw Error('unjoined');}};
    };
    return {args:{operationId:'original',expiresAt:112,open,
        saveReference:async value=>{saved=structuredClone(value);},observeHelpers:async()=>{},helpersJoined:async()=>true,
        now:()=>now,sleep:async ms=>{now+=ms/1000;expired=true;},boundary:async()=>{expired=true;return {revoked:true};}},
        stats:()=>({mutations,opens,closes,reads,saved})};
}

for(const kind of ['expiry','revocation'])test(`native ${kind} uses one mutation and the same original receipt before and after the boundary`,async()=>{
    const f=fixture(),value=await nativeAuthorityCase({kind,...f.args});
    assert.equal(value.passed,true);assert.equal(value.original_reference_preserved,true);
    assert.equal(value.original_owners_joined,true);assert.equal(f.stats().mutations,1);
    assert.equal(f.stats().opens,2);assert.equal(f.stats().closes,2);assert.equal(f.stats().reads,3);
});
test('a fresh transport timeout is not native current-authority refusal',async()=>{
    const f=fixture({freshKind:'deadline'}),value=await nativeAuthorityCase({kind:'expiry',...f.args});
    assert.equal(value.passed,false);assert.equal(value.fresh_original_refusal.kind,'deadline');assert.equal(f.stats().mutations,1);
    assert.equal(value.original_owners_joined,true);
});
test('an expired optional configuration is not a fresh native authentication refusal',async()=>{
    const f=fixture({freshKind:'configuration'}),value=await nativeAuthorityCase({kind:'expiry',...f.args});
    assert.equal(value.passed,false);assert.equal(value.fresh_original_refusal.kind,'configuration');
    assert.equal(value.original_owners_joined,true);assert.equal(f.stats().mutations,1);
    assert.equal(f.stats().opens,2);assert.equal(f.stats().closes,2);
});
test('a live request deadline is not a positive lifetime refusal',async()=>{
    const f=fixture({liveKind:'deadline'}),value=await nativeAuthorityCase({kind:'revocation',...f.args});
    assert.equal(value.passed,false);assert.equal(f.stats().opens,1);assert.equal(f.stats().closes,1);
});
test('an unrelated live application failure cannot substitute for current authority refusal',async()=>{
    const f=fixture({liveKind:'application'}),value=await nativeAuthorityCase({kind:'revocation',...f.args});
    assert.equal(value.passed,false);assert.equal(f.stats().opens,1);assert.equal(f.stats().closes,1);
});
test('changed recovery reference refuses before crossing authority boundary',async()=>{
    const f=fixture({changeReference:true}),value=await nativeAuthorityCase({kind:'expiry',...f.args});
    assert.equal(value.passed,false);assert.equal(value.current_original_read_before_boundary,undefined);
    assert.equal(f.stats().mutations,1);assert.equal(f.stats().closes,1);
});
test('otherwise correct refusal cannot pass when original SDK owner fails to join',async()=>{
    const f=fixture({closeFailure:true}),value=await nativeAuthorityCase({kind:'revocation',...f.args});
    assert.equal(value.passed,false);assert.equal(value.original_owners_joined,false);
    assert.equal(f.stats().closes,2);
});
