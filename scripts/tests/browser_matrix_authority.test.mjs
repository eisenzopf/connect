import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixAuthority} from '../lib/browser_matrix_authority.mjs';

async function authorityControl(mode){
    const originals={crypto:Object.getOwnPropertyDescriptor(globalThis,'crypto'),
        performance:Object.getOwnPropertyDescriptor(globalThis,'performance'),timer:globalThis.setTimeout,now:Date.now};
    let wall=1000,monotonic=0,currentKind,revoked=false;
    const calls={expiry:{opens:0,reads:0,closes:0},revocation:{opens:0,reads:0,closes:0}},waits=[];
    const refusal=kind=>Object.assign(Error('Synthetic original refusal'),{kind,certainty:'read-only'});
    Object.defineProperty(globalThis,'crypto',{configurable:true,value:{subtle:{importKey:async()=>({})}}});
    Object.defineProperty(globalThis,'performance',{configurable:true,value:{now:()=>monotonic}});
    Date.now=()=>wall;
    globalThis.setTimeout=(resolve,wait)=>{
        waits.push(wait);monotonic+=wait;
        wall=mode==='frozen'||(mode==='early'&&waits.length===1)?12999:13100;
        if(mode==='frozen'&&waits.length>1)monotonic=13000;
        queueMicrotask(resolve);return 1;
    };
    const actor={identityId:'identity',device:{id:'device'},credential:'synthetic',
        signing:{jwk:{},algorithm:'EdDSA',keyId:'signer'},
        server:{jwk:{},algorithm:'EdDSA',keyId:'server',notBefore:0,expiresAt:100000}};
    const provision=async kind=>{
        if(kind==='revoke'){revoked=true;return {revoked:true};}
        currentKind=kind;return {actor,issued_at:1,expires_at:kind==='expiry'?13:120};
    };
    const Sdk={async connect(){
        const kind=currentKind,counter=calls[kind];counter.opens++;
        if(counter.opens>1)throw refusal(kind==='expiry'&&mode==='fresh-timeout'?'transport-uncertain':'authentication');
        return {conversations:{byId:()=>({async refresh(){counter.reads++;
            if(counter.reads>1&&(kind==='expiry'||revoked))throw refusal('authentication');
        }})},async close(){counter.closes++;}};
    }};
    const report={};
    try{
        await browserMatrixAuthority({Sdk,provision,endpoints:{},selected:'wss-webrtc',pin:new Uint8Array(32),
            state:{conversation:{id:'conversation'},report:{conflict:{passed:true}}},report});
        return {report,calls,waits};
    }finally{
        Object.defineProperty(globalThis,'crypto',originals.crypto??{configurable:true,value:undefined});
        Object.defineProperty(globalThis,'performance',originals.performance);
        globalThis.setTimeout=originals.timer;Date.now=originals.now;
    }
}

test('an early browser timer wake rechecks the original wall-clock expiry',async()=>{
    const {report,calls,waits}=await authorityControl('early');
    assert.deepEqual(waits,[12100,1]);
    assert.equal(report.passed,true);assert.equal(report.expiry.original_expiry_elapsed,true);
    assert.equal(report.expiry.original_expires_at,13);
    assert.equal(report.expiry.authentication_refusal.kind,'authentication');
    assert.deepEqual(calls.expiry,{opens:2,reads:2,closes:1});
});

test('the original browser expiry budget fails closed if the wall clock never reaches it',async()=>{
    const {report,calls,waits}=await authorityControl('frozen');
    assert.deepEqual(waits,[12100,1]);
    assert.equal(report.passed,false);assert.equal(report.expiry.passed,false);
    assert.equal(report.expiry.original_expiry_elapsed,false);
    assert.equal(report.expiry.failure.stage,'original-expiry-boundary');
    assert.equal(report.expiry.failure.message,'Original credential expiry has not elapsed');
    assert.equal(report.expiry.expiry_boundary.original_issued_at,1);
    assert.equal(report.expiry.expiry_boundary.original_expires_at,13);
    assert.equal(report.expiry.expiry_boundary.observed_date_ms,12999);
    assert.equal(report.expiry.expiry_boundary.observed_wall_elapsed_ms,11999);
    assert.equal(report.expiry.expiry_boundary.observed_monotonic_elapsed_ms,13000);
    assert.deepEqual(calls.expiry,{opens:1,reads:1,closes:1},'No post-expiry claim or new open before actual expiry');
    assert.equal(report.expiry.original_owners_joined,true);
    assert.equal(report.revocation.passed,true,'Continue the independent original authority case');
});

test('elapsed original expiry still requires explicit fresh authentication refusal',async()=>{
    const {report,calls,waits}=await authorityControl('fresh-timeout');
    assert.deepEqual(waits,[12100]);assert.equal(report.expiry.original_expiry_elapsed,true);
    assert.equal(report.expiry.passed,false);assert.equal(report.passed,false);
    assert.equal(report.expiry.failure.stage,'original-fresh-open');
    assert.equal(report.expiry.authentication_refusal.kind,'transport-uncertain');
    assert.deepEqual(calls.expiry,{opens:2,reads:2,closes:1},'Never retry a fresh original open');
    assert.equal(report.revocation.passed,true);
});

test('one elapsed original boundary and both exact authentication refusals qualify authority',async()=>{
    const {report,calls,waits}=await authorityControl('elapsed');
    assert.deepEqual(waits,[12100]);assert.equal(report.passed,true);
    assert.equal(report.expiry.original_expiry_elapsed,true);
    assert.equal(report.expiry.original_issued_at,1);assert.equal(report.expiry.original_expires_at,13);
    assert.equal(report.expiry.expiry_boundary.observed_date_ms,13100);
    assert.equal(report.expiry.authentication_refusal.kind,'authentication');
    assert.deepEqual(calls.expiry,{opens:2,reads:2,closes:1});
});
