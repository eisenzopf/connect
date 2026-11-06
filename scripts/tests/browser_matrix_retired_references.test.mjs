import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixRetiredReferences} from '../lib/browser_matrix_retired_references.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const proof={renderer:{joined:true,playback_passed:false},source:{joined:true,released:true},transport:{joined:true},keepalive:{stopped:true,original_batch_joined:true,retained_client_references:0,failed:true}};
function fixture(){
    const clients=[{name:'original-a'},{name:'original-b'}],actors={a:{identityId:'a',client:clients[0]},b:{identityId:'b',client:clients[1]}};
    const admission={checks:['actual-original-proof'],conflict:{passed:true}},conversation={id:'conversation',client:clients[0]};
    const state={conversation,sessions:[{client:clients[0]}],connections:clients.map(client=>({client})),report:admission,
        createReference:{operationId:'original-operation'}};
    const root={matrixActors:actors},owner=browserMatrixRetiredReferences({actors,state,root});
    return {clients,actors,state,root,owner,admission};
}
function hasOwner(value,owners){return owners.includes(value)||(value&&typeof value==='object'&&Object.values(value).some(child=>hasOwner(child,owners)));}

test('pending actual original client joins retain roots; fulfilled joins release only SDK wrappers and preserve exact dynamic evidence',async()=>{
    const f=fixture(),pending=deferred(),called=[];
    const original=f.owner.closeClients(async client=>{called.push(client);if(client===f.clients[0])await pending.promise;return {deadlineMet:true};});
    for(let i=0;i<5;i++)await Promise.resolve();
    const early=f.owner.release(proof);assert.equal(early.released,false);
    assert.equal(f.actors.a.client,f.clients[0]);assert.equal(f.root.matrixActors,f.actors);
    pending.resolve();const joined=await original;assert.deepEqual(joined.map(row=>row.status),['fulfilled','fulfilled']);
    const released=f.owner.release(proof);
    assert.equal(released.released,true);assert.equal(f.root.matrixActors,undefined);
    assert.equal(f.actors.a.client,undefined);assert.equal(f.actors.b.client,undefined);
    assert.equal(f.state.conversation,undefined);assert.deepEqual(f.state.sessions,[]);assert.deepEqual(f.state.connections,[]);
    assert.equal(f.state.report,f.admission,'Keep original admission/current-authority report references');
    assert.deepEqual(f.state.createReference,{operationId:'original-operation'});
    assert.equal(released.counts.authenticated_owners,2);assert.equal(released.counts.actual_connections,2,'Capture actual counts, never7/6constants');
    assert.deepEqual(released.counts.original_client_closes.map(row=>row.role),['a','b']);
    assert.ok(Object.isFrozen(released)&&Object.isFrozen(released.counts)&&Object.isFrozen(released.counts.original_client_closes));
    assert.equal(hasOwner(released,f.clients),false);assert.equal(f.owner.release(proof),released);
    assert.equal(f.owner.closeClients(()=>{throw Error('Must not close again');}),original);
    await f.owner.closeClients(()=>{throw Error('Must not close again');});assert.equal(called.length,2);
    assert.deepEqual(f.owner.facts(),released.counts,'After-release observation still exposes original closed facts');
    assert.equal(proof.renderer.playback_passed,false,'Semantic playback/reachability failures are never rewritten');
});

test('actual close refusal and any missing physical proof retain original roots and original error',async()=>{
    const f=fixture(),original=Object.assign(Error('Original SDK close refusal'),{kind:'deadline',certainty:'read-only'});
    const results=await f.owner.closeClients(client=>client===f.clients[0]?Promise.reject(original):Promise.resolve());
    assert.equal(results[0].reason,original);
    assert.equal(f.owner.release(proof).released,false);assert.equal(f.root.matrixActors,f.actors);assert.equal(f.actors.a.client,f.clients[0]);
    assert.equal(f.owner.facts().original_client_closes[0].kind,'deadline');
    for(const missing of ['renderer','source','transport','keepalive']){
        const one=fixture();await one.owner.closeClients(async()=>{});
        const incomplete={...proof,[missing]:missing==='keepalive'?{retained_client_references:1}:{joined:false}};
        assert.equal(one.owner.release(incomplete).released,false,missing);assert.equal(one.actors.a.client,one.clients[0],missing);
        assert.equal(one.state.report,one.admission);
    }
});

test('foreign actor/global aliases and unavailable state counts are failures without erasing original owners',async()=>{
    for(const mode of ['global','actor','unknown-state']){
        const f=fixture();if(mode==='unknown-state')delete f.state.connections;
        await f.owner.closeClients(async()=>{});
        if(mode==='global')f.root.matrixActors={foreign:true};
        if(mode==='actor')f.actors.a={identityId:'foreign',client:{foreign:true}};
        const before=f.actors.b.client,global=f.root.matrixActors,result=f.owner.release(proof);
        assert.equal(result.released,false,mode);assert.equal(f.actors.b.client,before,mode);assert.equal(f.root.matrixActors,global,mode);
        if(mode==='unknown-state')assert.equal(result.counts.actual_connections,null,'Unavailable count never becomes0');
    }
});

test('a sampler failure after reference release keeps original counts and joined receipts without another client call',async()=>{
    const f=fixture();let closes=0;
    const original=f.owner.closeClients(async()=>{closes++;});await original;f.owner.release(proof);
    const samplerFailure=Error('Original sampler failed');let failureReport;
    try{throw samplerFailure;}catch(error){
        const results=await f.owner.closeClients(()=>{throw Error('No resubmission after original joins');});
        failureReport={passed:false,counts:f.owner.counts(),closes:results.map(row=>row.status),admission:f.state.report,error};
    }
    assert.equal(closes,2);assert.equal(failureReport.error,samplerFailure);assert.equal(failureReport.admission,f.admission);
    assert.deepEqual(failureReport.counts,{authenticated_owners:2,actual_connections:2});
    assert.deepEqual(failureReport.closes,['fulfilled','fulfilled']);assert.equal(failureReport.passed,false);
});


test('added or deleted actor roles after original capture refuse release and retain original roots and facts',async()=>{
    for(const mode of ['added','deleted','hidden-added','symbol-added']){
        const f=fixture(),originalActor=f.actors.a,originalConversation=f.state.conversation;
        await f.owner.closeClients(async()=>{});const facts=f.owner.facts();
        const symbol=Symbol('foreign');
        if(mode==='added')f.actors.foreign={client:{unjoined:true}};
        else if(mode==='hidden-added')Object.defineProperty(f.actors,'foreign',{configurable:true,value:{client:{unjoined:true}}});
        else if(mode==='symbol-added')f.actors[symbol]={client:{unjoined:true}};
        else delete f.actors.a;
        const foreign=f.actors.foreign,result=f.owner.release(proof);
        assert.equal(result.released,false);assert.ok(result.failures.includes('foreign-original-actor-role-set'));
        assert.equal(f.root.matrixActors,f.actors);assert.equal(f.actors.b.client,f.clients[1]);
        assert.equal(f.state.conversation,originalConversation);assert.equal(f.state.connections.length,2);
        if(mode==='added')assert.equal(f.actors.foreign,foreign,'Never delete or claim retirement of a foreign owner');
        assert.deepEqual(f.owner.facts(),facts,'Actual original captured counts and close receipts stay unchanged');
        assert.equal(result.counts.authenticated_owners,2);assert.equal(result.counts.actual_connections,2);
        if(mode==='added'||mode==='hidden-added')delete f.actors.foreign;
        else if(mode==='symbol-added')delete f.actors[symbol];else f.actors.a=originalActor;
        assert.equal(f.owner.release(proof).released,true,'Original identity anchors remain available after a refused release');
    }
});

test('foreign conversation, array or element changes after capture cannot be erased or called retired',async()=>{
    for(const mode of ['conversation','session-array','connection-array','session-added','connection-added',
        'session-replaced','connection-replaced','session-deleted','connection-deleted','connection-hole','connection-property','session-property']){
        const f=fixture(),conversation=f.state.conversation,sessions=f.state.sessions,connections=f.state.connections,
            sessionHandles=sessions.slice(),connectionHandles=connections.slice();
        await f.owner.closeClients(async()=>{});const facts=f.owner.facts(),foreign={client:{unjoined:true}};
        if(mode==='conversation')f.state.conversation={...conversation,client:foreign.client};
        if(mode==='session-array')f.state.sessions=sessions.slice();
        if(mode==='connection-array')f.state.connections=connections.slice();
        if(mode==='session-added')sessions.push(foreign);
        if(mode==='connection-added')connections.push(foreign);
        if(mode==='session-replaced')sessions[0]=foreign;
        if(mode==='connection-replaced')connections[0]=foreign;
        if(mode==='session-deleted')sessions.pop();
        if(mode==='connection-deleted')connections.pop();
        if(mode==='connection-hole')delete connections[1];
        if(mode==='connection-property')connections.foreign=foreign;
        if(mode==='session-property')sessions.foreign=foreign;
        const before={conversation:f.state.conversation,sessions:f.state.sessions,connections:f.state.connections,
            sessionHandles:Object.assign(f.state.sessions.slice(),f.state.sessions),connectionHandles:Object.assign(f.state.connections.slice(),f.state.connections)};
        const result=f.owner.release(proof);
        assert.equal(result.released,false,mode);assert.ok(result.failures.includes('foreign-original-state-owner'),mode);
        assert.equal(f.root.matrixActors,f.actors,mode);assert.equal(f.actors.a.client,f.clients[0],mode);
        assert.equal(f.state.conversation,before.conversation,mode);assert.equal(f.state.sessions,before.sessions,mode);
        assert.equal(f.state.connections,before.connections,mode);
        assert.deepEqual(f.state.sessions,before.sessionHandles,mode);assert.deepEqual(f.state.connections,before.connectionHandles,mode);
        assert.deepEqual(f.owner.facts(),facts,mode);
        assert.equal(result.counts.actual_connections,2,'Keep actual original captured count despite foreign/current count');
        delete sessions.foreign;delete connections.foreign;
        sessions.splice(0,sessions.length,...sessionHandles);connections.splice(0,connections.length,...connectionHandles);
        Object.assign(f.state,{conversation,sessions,connections});
        assert.equal(f.owner.release(proof).released,true,mode+' original anchors remain intact and clear only after proven release');
    }
});


test('missing, running, unjoined or unavailable keepalive observation cannot qualify quality reference release',async()=>{
    for(const keepalive of [undefined,
        {stopped:false,original_batch_joined:true,retained_client_references:0},
        {stopped:true,original_batch_joined:false,retained_client_references:0},
        {stopped:true,original_batch_joined:true},
        {stopped:true,original_batch_joined:true,retained_client_references:1}]){
        const f=fixture();await f.owner.closeClients(async()=>{});const facts=f.owner.facts();
        const result=f.owner.release({...proof,keepalive});
        assert.equal(result.released,false);assert.ok(result.failures.includes('original-local-owner-unjoined'));
        assert.equal(f.root.matrixActors,f.actors);assert.equal(f.actors.a.client,f.clients[0]);
        assert.equal(f.state.connections.length,2);assert.deepEqual(f.owner.facts(),facts);
    }
});
