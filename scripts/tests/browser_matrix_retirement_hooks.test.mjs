import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixRetirementHooks,finishBrowserMatrixRetirement} from '../lib/browser_matrix_retirement_hooks.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
function fixture(){
    const calls=[];
    class Peer {
        constructor(work=Promise.resolve(),values={}){this.work=work;this.values=values;this.reads=0;}
        shutdown(...args){calls.push({owner:this,args});if(this.throw_)throw this.throw_;return this.work;}
        beginReplacementConnection(){return {completed:Promise.resolve(),owner:this};}
        get diagnostics(){this.reads++;return {sent:1,received:2,rejected:0,decodedAudio:4,decodedVideo:5,
            queuedPackets:0,queuedBytes:0,droppedPackets:0,activeBindings:0,borrowedOwner:this,...this.values};}
    }
    class Rtc {
        constructor(work=Promise.resolve(),values={}){this.work=work;this.values=values;this.reads=0;}
        shutdown(...args){calls.push({owner:this,args});return this.work;}
        get diagnostics(){this.reads++;return {closed:true,peerConnectionState:'closed',remoteTracks:0,liveRemoteTracks:0,
            captureActive:false,renderActive:false,borrowedOwner:this,...this.values};}
    }
    const originals={native:Peer.prototype.shutdown,rtc:Rtc.prototype.shutdown,begin:Peer.prototype.beginReplacementConnection};
    const hooks=browserMatrixRetirementHooks(Peer,Rtc,{log(){}});
    return {Peer,Rtc,originals,hooks,calls};
}
function hasReference(value,owners){
    if(owners.includes(value))return true;
    return value&&typeof value==='object'&&Object.values(value).some(child=>hasReference(child,owners));
}

test('exact delayed original joins precede immutable snapshots, hook restoration and diagnostic owner release',async()=>{
    const f=fixture(),a=deferred(),b=deferred(),native=new f.Peer(a.promise),rtc=new f.Rtc(b.promise);
    assert.equal(native.shutdown('original-argument'),a.promise);assert.equal(rtc.shutdown(),b.promise);
    const closing=f.hooks.finish();assert.equal(f.hooks.finish(),closing,'One original diagnostic release owns its joins');
    let settled=false;closing.then(()=>{settled=true;});
    for(let i=0;i<5;i++)await Promise.resolve();
    assert.equal(settled,false);assert.equal(native.reads,0);assert.equal(rtc.reads,0);
    assert.notEqual(f.Peer.prototype.shutdown,f.originals.native);
    a.resolve();for(let i=0;i<5;i++)await Promise.resolve();assert.equal(settled,false);
    b.resolve();const result=await closing;
    assert.equal(result.joined,true);assert.equal(result.original_hooks_restored,true);
    assert.equal(result.diagnostic_owner_references_released,true);assert.equal(result.retained_diagnostic_owners,0);
    assert.equal(f.Peer.prototype.shutdown,f.originals.native);assert.equal(f.Rtc.prototype.shutdown,f.originals.rtc);
    assert.equal(f.Peer.prototype.beginReplacementConnection,f.originals.begin);
    assert.equal(native.reads,1);assert.equal(rtc.reads,1);assert.deepEqual(f.calls[0].args,['original-argument']);
    assert.equal(result.peers[0].received,2);assert.equal(result.peers[0].original_shutdown_joined,true);
    assert.equal(result.rtc[0].closed,true);assert.ok(Object.isFrozen(result)&&Object.isFrozen(result.peers)&&Object.isFrozen(result.peers[0]));
    assert.equal(hasReference(result,[native,rtc,a.promise,b.promise]),false,'Evidence never retains an SDK owner or original promise');
    assert.equal('borrowedOwner' in result.peers[0],false);assert.equal('borrowedOwner' in result.rtc[0],false);
});

test('original shutdown rejection is unchanged and cannot release diagnostic owners or turn into a successful join',async()=>{
    const f=fixture(),original=deferred(),error=Object.assign(Error('Private original detail'),
        {name:'OriginalShutdownFailure',kind:'transport-uncertain',certainty:'read-only'}),native=new f.Peer(original.promise);
    const call=native.shutdown(),closing=f.hooks.finish();original.reject(error);
    await assert.rejects(call,actual=>actual===error);const result=await closing;
    assert.equal(result.joined,false);assert.equal(result.retained_diagnostic_owners,1);
    assert.equal(result.original_hooks_restored,false);assert.notEqual(f.Peer.prototype.shutdown,f.originals.native);
    assert.equal(result.peers[0].original_shutdown_joined,false);
    assert.equal(result.shutdown_failures[0].kind,'transport-uncertain');assert.equal(result.shutdown_failures[0].certainty,'read-only');
    assert.equal(JSON.stringify(result).includes(error.message),false);
});

test('a foreign prototype hook is retained and reported instead of overwritten after fulfilled original joins',async()=>{
    const f=fixture(),rtc=new f.Rtc();await rtc.shutdown();
    const foreign=function(){};f.Rtc.prototype.shutdown=foreign;
    const result=await f.hooks.finish();
    assert.equal(result.joined,false);assert.ok(result.failures.includes('foreign-diagnostic-hook'));
    assert.equal(f.Rtc.prototype.shutdown,foreign);assert.equal(result.retained_diagnostic_owners,1);
    assert.equal(result.diagnostic_owner_references_released,false);
});

test('fulfilled shutdown with active queues or media ownership cannot release backing diagnostic references',async()=>{
    for(const rtcCase of [false,true]){
        const f=fixture(),owner=rtcCase?new f.Rtc(Promise.resolve(),{renderActive:true}):new f.Peer(Promise.resolve(),{queuedBytes:1});
        await owner.shutdown();const result=await f.hooks.finish();
        assert.equal(result.joined,false);assert.ok(result.failures.includes('original-media-owner-still-active'));
        assert.equal(result.retained_diagnostic_owners,1);assert.equal(result.original_hooks_restored,false);
    }
});

test('synchronous and nonpromise original shutdowns remain failures without invented join evidence',async()=>{
    for(const mode of ['throw','nonpromise']){
        const f=fixture(),native=new f.Peer(undefined);if(mode==='throw')native.throw_=Object.assign(Error('Original failure'),{code:'original'});
        else native.work=undefined;
        if(mode==='throw')assert.throws(()=>native.shutdown(),error=>error===native.throw_);
        else assert.equal(native.shutdown(),undefined);
        const result=await f.hooks.finish();assert.equal(result.joined,false);assert.equal(result.retained_diagnostic_owners,1);
        assert.equal(result.peers[0].original_shutdown_joined,false);
    }
});

test('foreign receiver and original bounded-owner overflow remain explicit failures',async()=>{
    const foreign=fixture();assert.throws(()=>foreign.Peer.prototype.shutdown.call({}),/Foreign original shutdown owner/);
    assert.ok((await foreign.hooks.finish()).failures.includes('foreign-original-shutdown-owner'));
    const f=fixture();for(let i=0;i<7;i++)await new f.Peer().shutdown();
    assert.throws(()=>new f.Peer().shutdown(),/Bounded original diagnostic owners/);
    const result=await f.hooks.finish();assert.equal(result.joined,false);
    assert.ok(result.failures.includes('original-diagnostic-owner-bound'));assert.equal(result.retained_diagnostic_owners,7);
});


test('finalization joins the original pending shutdown then preserves the exact earlier local-close error',async()=>{
    const f=fixture(),work=deferred(),native=new f.Peer(work.promise);
    const shutdownError=Object.assign(Error('Actual original shutdown refusal'),{kind:'transport-uncertain',certainty:'read-only'});
    const originalCloseError=Object.assign(Error('Actual original local graph refusal'),{kind:'cancelled',certainty:'refused'});
    const shutdown=native.shutdown(),shutdownRefusal=assert.rejects(shutdown,error=>error===shutdownError);
    const finalizing=finishBrowserMatrixRetirement({hooks:f.hooks,closed:{failure:originalCloseError}});
    const originalRefusal=assert.rejects(finalizing,error=>error===originalCloseError);let settled=false;
    finalizing.then(()=>{settled=true;},()=>{settled=true;});
    for(let i=0;i<6;i++)await Promise.resolve();assert.equal(settled,false,'An original failure cannot skip pending actual cleanup');
    work.reject(shutdownError);await shutdownRefusal;await originalRefusal;
    const observed=await f.hooks.finish();assert.equal(observed.joined,false);
    assert.equal(observed.shutdown_failures[0].kind,shutdownError.kind);
    assert.equal(observed.diagnostic_owner_references_released,false);
});

test('an unexpected diagnostic exception cannot replace the exact original local-close failure',async()=>{
    const original=Error('Original local close'),later=Error('Later diagnostic error');let calls=0;
    await assert.rejects(finishBrowserMatrixRetirement({hooks:{async finish(){calls++;throw later;}},closed:{failure:original}}),
        error=>error===original);
    assert.equal(calls,1);
    await assert.rejects(finishBrowserMatrixRetirement({hooks:{async finish(){throw later;}},closed:{}}),error=>error===later);
});

test('a diagnostic-only failed join remains failed unless its original body already retained that failure',async()=>{
    const diagnostic=Object.freeze({joined:false});const hooks={finish:async()=>diagnostic};
    await assert.rejects(finishBrowserMatrixRetirement({hooks,closed:{}}),/Original diagnostic hooks did not retire/);
    assert.equal(await finishBrowserMatrixRetirement({hooks,closed:{},reported:true}),diagnostic);
    assert.equal(await finishBrowserMatrixRetirement({hooks,closed:{},previousFailure:true}),diagnostic);
});
