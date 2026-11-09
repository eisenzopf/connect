import assert from "node:assert/strict";
import {test} from "node:test";
import {browserMatrixAdmission,browserMatrixPlan,retireBrowserMatrixAdmission,browserMatrixCycleSummary,browserMatrixCycleOutcome} from "../lib/browser_matrix_admission.mjs";
function fixture(create) {
    const roles=["participant-1","participant-2","participant-3","participant-4","replacement-1","controller-1","controller-2"];
    return Object.fromEntries(roles.map((role,index)=>[role,{identityId:"identity-"+(index<4?index:0),deviceId:"device-"+index,
        client:{role:"browser",capabilities:{refresh:async()=>({selectedTransport:"webtransport"})},conversations:{create}}}]));
}
// Public SDK-shaped control model only. It proves initial-owner ordering and
// failure custody, never real authorization, decoder or transport quality.
function initialMediaAdmissionFixture() {
    let serial=0;
    const plan=browserMatrixPlan(()=>String(++serial)),events=[],captured=new Set(),ready=new Set();
    const state={},connections=new Map(),sent=new Set(),hooks={};
    let fourthActive=false,revision=1;
    const denied=(code,reason)=>Object.assign(Error(reason),{problem:{code,reason},certainty:'refused'});
    const operation=(value,wait)=>{
        const reference={operation_id:'original-'+serial++};
        return {referenceJson:()=>reference,async wait(){
            if(wait)await wait();
            return {snapshot:{state:'completed'},value};
        }};
    };
    const actors=fixture();
    const participants=()=>['participant-1','participant-2','participant-3','participant-4']
        .filter(role=>role!=='participant-4'||fourthActive)
        .map(role=>({id:'member-'+role,identityId:actors[role].identityId,active:true}));
    const session=id=>({id,refresh:async()=>({topologyRevision:1}),
        accept:async()=>operation({}),
        update:async input=>{
            if(input.media.first==='screen')throw denied(501,'unsupported_feature');
            return operation({});
        },connections:{
            byId:id=>connections.get(id),
            offer:async input=>{
                const route=plan.routes.find(value=>value.connection===input.connectionId);
                events.push(['offer',route.index]);
                hooks.offer?.(route.index);
                const connection={id:input.connectionId,refresh:async()=>({streamRevision:1}),
                    streams:{subscribe:async()=>operation({})}};
                connections.set(connection.id,connection);
                return operation(connection,async()=>{
                    assert.ok(captured.has(route.index),'Actual source start precedes original offer readiness');
                    ready.add(route.index);events.push(['ready',route.index]);
                });
            }
        }});
    const conversation={id:'original-room',refresh:async()=>({revision,participants:participants()}),
        send:async input=>{sent.add(input.messageId);return operation({});},
        participants:{
            add:async()=>{fourthActive=true;revision++;return operation({});},
            byId:()=>({
                update:async input=>{
                    if(input.role==='participant')throw denied(409,'operation_conflict');
                    revision++;return operation({});
                },
                remove:async()=>{fourthActive=false;revision++;return operation({});}
            })
        },sessions:{invite:async input=>operation(session(input.sessionId)),byId:session}};
    for(const [role,actor] of Object.entries(actors)){
        const client=actor.client;
        client.capabilities.refresh=async()=>({selectedTransport:'wss-webrtc'});
        client.conversations={create:async()=>operation(conversation),byId:()=>({
            ...conversation,
            refresh:async()=>{
                if(role==='participant-4'&&!fourthActive)throw denied(403,'forbidden');
                return conversation.refresh();
            }
        })};
        client.operations={restore:()=>operation(conversation)};
        client.operationKinds={conversationCreate:'conversation-create'};
        client.raw={validate:value=>value,request:async()=>[
            {value:{type:'ack',payload:{next_cursor:null}}},
            ...(sent.has(plan.messages.included)
                ?[{value:{type:'message.send',payload:{msg_id:plan.messages.included}}}]:[])
        ]};
    }
    return {plan,actors,state,events,ready,hooks,async startMedia({route,connection}){
        assert.equal(connection.id,route.connection);captured.add(route.index);events.push(['capture',route.index]);
    }};
}

test('each initial browser renderer starts before the next offer without waiting on its later receive source',async()=>{
    const f=initialMediaAdmissionFixture(),rendered=[];
    let resolveFirst;
    const firstPlay=new Promise(resolve=>{resolveFirst=resolve;});
    f.hooks.offer=index=>{if(index===1)resolveFirst();};
    const pending=[];
    await browserMatrixAdmission({...f,selectedTransport:'wss-webrtc',journal:async()=>{},
        afterMediaOffer({route,connection,options}){
            assert.ok(f.ready.has(route.index));
            assert.equal(connection,f.state.connections[route.index],'Retain the actual completed Connection');
            assert.equal(options.timeoutMs,8000,'Keep original initial-render options');
            f.events.push(['render-start',route.index]);
            const work=route.index===0?firstPlay:Promise.resolve();
            pending.push(work.then(()=>{rendered.push(route.index);}));
        }});
    assert.deepEqual(f.events,Array.from({length:6},(_,index)=>[
        ['offer',index],['capture',index],['ready',index],['render-start',index]]).flat());
    await Promise.all(pending);
    assert.deepEqual(rendered.sort(),[0,1,2,3,4,5]);
});

test('initial renderer refusal retains completed Connections for cleanup and submits no later offer',async()=>{
    const f=initialMediaAdmissionFixture(),failure=Error('Original initial renderer refused');
    await assert.rejects(browserMatrixAdmission({...f,selectedTransport:'wss-webrtc',journal:async()=>{},
        async afterMediaOffer({route}){
            assert.ok(f.ready.has(route.index));throw failure;
        }}),error=>error===failure);
    assert.deepEqual(f.events,[['offer',0],['capture',0],['ready',0]]);
    assert.equal(f.state.connections.length,1,'The original cleanup owner retains every completed Connection');
    assert.equal(f.state.connections[0].id,f.plan.routes[0].connection);
});

test("an uncertain browser mutation preserves its original scoped operation and admits no later root",async()=>{
    const plan=browserMatrixPlan(),events=[];let submissions=0;
    const original={operationId:plan.operations.create,command:"conversation.create",scope:{}};
    const error=Object.assign(Error("uncertain"),{certainty:"uncertain",operation:original});
    const actors=fixture(async(_input,options)=>{submissions++;assert.equal(options.operationId,original.operationId);throw error;});
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async value=>events.push(value)}),e=>e===error);
    assert.equal(submissions,1);
    assert.deepEqual(events.map(value=>value.kind),["plan","intent","failure"]);
    assert.deepEqual(events.at(-1).operation,original);
    assert.equal(events.at(-1).operationId,plan.operations.create);
});
test("failure to persist the original browser intent prevents its mutation submission",async()=>{
    const plan=browserMatrixPlan();let submissions=0;
    const actors=fixture(async()=>{submissions++;throw Error("must not submit");});
    const persist=Error("intent could not be saved");
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async event=>{
        if(event.kind==="intent")throw persist;
    }}),e=>e===persist);
    assert.equal(submissions,0);
});
test("an accepted browser root retains the exact reference when its original wait is uncertain",async()=>{
    const plan=browserMatrixPlan(),events=[];let submissions=0;
    const reference={schema:"connect-operation-reference/1",operation_id:plan.operations.create,command:"conversation.create",scope:{}};
    const error=Object.assign(Error("wait uncertain"),{certainty:"uncertain"});
    const actors=fixture(async()=>{submissions++;return {referenceJson:()=>reference,wait:async()=>{throw error;}};});
    await assert.rejects(browserMatrixAdmission({actors,plan,selectedTransport:"webtransport",journal:async event=>events.push(event)}),e=>e===error);
    assert.equal(submissions,1);
    assert.deepEqual(events.map(value=>value.kind),["plan","intent","accepted","failure"]);
    assert.deepEqual(events.at(-1).reference,reference);
});

test("missed browser terminal deadline still joins original cleanup and cannot turn green afterward",async()=>{
    let clock=0,closeCalls=0;
    const plan=browserMatrixPlan(),actors={owner:{client:{close:async()=>{closeCalls++;if(closeCalls===1)throw Error("deadline");}}}};
    const connection={id:plan.routes[0].connection,refresh:async()=>({streamRevision:1}),end:async()=>({
        referenceJson:()=>({operation_id:plan.routes[0].end}),wait:async()=>{clock=6000;return {snapshot:{state:"completed"}};}})};
    const result=await retireBrowserMatrixAdmission({state:{connections:[connection],sessions:[]},actors,plan,journal:async()=>{},now:()=>clock});
    assert.equal(closeCalls,2);assert.equal(result.passed,false);assert.equal(result.originalClientsClosed,1);assert.equal(result.elapsedMs,6000);
});

test("browser terminal ends both original Sessions before client closure without resubmitting uncertainty",async()=>{
    for(const uncertain of [false,true]){
        const plan=browserMatrixPlan(),events=[],submitted=[];let clock=100,closed=false;
        const sessions=plan.sessions.map((s,index)=>({id:s.id,refresh:async()=>({topologyRevision:10+index}),
            end:async(input,options)=>{
                assert.equal(input.expectedTopologyRevision,10+index);assert.equal(options.operationId,s.end);
                assert.ok(options.timeoutMs<=4900);assert.equal(closed,false);submitted.push(s.end);
                return {referenceJson:()=>({operation_id:s.end}),wait:async()=>{
                    if(index===0&&uncertain)throw Object.assign(Error('uncertain'),{certainty:'uncertain'});
                    return {snapshot:{state:'completed'}};
                }};
            }}));
        const actors={owner:{client:{close:async()=>{assert.equal(submitted.length,2);closed=true;clock=400;}}}};
        const result=await retireBrowserMatrixAdmission({state:{connections:[],sessions},actors,plan,
            journal:async event=>events.push(event),now:()=>clock,terminalMs:0});
        assert.deepEqual(submitted,plan.sessions.map(s=>s.end));assert.ok(closed);
        assert.equal(result.originalSessionsEnded,uncertain?1:2);assert.equal(result.passed,!uncertain);
        assert.equal(events.filter(e=>e.kind==='session-end-accepted').length,2);
    }
});


test('reference ownership joins the original cleanup callback and preserves a missed teardown deadline',async()=>{
    const {browserMatrixRetiredReferences}=await import('../lib/browser_matrix_retired_references.mjs');
    const plan=browserMatrixPlan(),options=[];let clock=0,finish;
    const pending=new Promise(resolve=>{finish=resolve;});
    const client={async close(value){options.push(value.timeoutMs);if(options.length===1)throw Error('Original deadline');await pending;}};
    const actors={owner:{client}},connection={id:plan.routes[0].connection,refresh:async()=>({streamRevision:1}),
        end:async()=>({referenceJson:()=>({operation_id:plan.routes[0].end}),wait:async()=>{clock=6000;return {snapshot:{state:'completed'}};}})};
    const state={conversation:{id:'original-conversation'},sessions:[],connections:[connection],report:{checks:['original']}},root={matrixActors:actors};
    const refs=browserMatrixRetiredReferences({actors,state,root}),proof={renderer:{joined:true},source:{joined:true,released:true},transport:{joined:true},
        keepalive:{stopped:true,original_batch_joined:true,retained_client_references:0}};
    const original=retireBrowserMatrixAdmission({state,actors,plan,journal:async()=>{},now:()=>clock,
        closeOriginalClients:perform=>refs.closeClients(perform)});
    for(let i=0;i<20;i++)await Promise.resolve();
    assert.deepEqual(options,[1,60000],'Preserve existing missed-deadline original cleanup waits exactly');
    assert.equal(refs.release(proof).released,false);assert.equal(actors.owner.client,client);
    finish();const result=await original;
    assert.equal(result.passed,false);assert.equal(result.originalClientsClosed,1);assert.equal(result.elapsedMs,6000);
    assert.equal(refs.release(proof).released,true,'Actual local cleanup can release refs without changing the failed deadline');
    assert.equal(result.passed,false);assert.deepEqual(refs.counts(),{authenticated_owners:1,actual_connections:1});
    await refs.closeClients(()=>{throw Error('No new close after release');});assert.deepEqual(options,[1,60000]);
});

test('three original cycle plans keep distinct stable operation and Session namespaces',()=>{
    const plans=[1,2,3].map(index=>{let next=0;return browserMatrixPlan(()=>String(++next),{index,workload_id:'original-cycle-'+index});});
    const ids=plans.flatMap(plan=>[...Object.values(plan.operations),...plan.sessions.map(session=>session.id),...plan.routes.map(route=>route.connection)]);
    assert.equal(new Set(ids).size,ids.length);
    assert.ok(plans.every(plan=>plan.programmeMs===62000&&plan.moves.length===4));
    assert.deepEqual(plans.map(plan=>plan.cycle.index),[1,2,3]);
    assert.throws(()=>browserMatrixPlan(()=> 'original',{index:4,workload_id:'unknown'}),/cycle identity/);
});
test('cycle summaries retain first actual failure and no full AV or owner reference graph',()=>{
    const source={privateLargeAvData:[1,2,3]},failure={stage:'decoded-media-quality',name:'OriginalQualityFailure'};
    const summaries=[1,2,3].map(index=>{
        const cycle={index,workload_id:'original-cycle-'+index};
        const value={passed:index!==2,workload_started:true,workload_complete:true,programme:source,...(index===2?{failure}:{})};
        return browserMatrixCycleSummary(cycle,value,{...cycle,observation_sha256:String(index).repeat(64),
            workload_evidence_sha256:String(index+3).repeat(64),join_evidence_sha256:String(index+6).repeat(64)});
    });
    source.privateLargeAvData.push(4);
    assert.ok(summaries.every(value=>!('programme' in value)&&!('decoded_media' in value)));
    const result=browserMatrixCycleOutcome(summaries);
    assert.equal(result.passed,false);assert.equal(result.first_failed_cycle,2);assert.deepEqual(result.failure,failure);
    assert.equal(result.cycles.length,3);
    assert.throws(()=>browserMatrixCycleOutcome(summaries.slice(0,2)),/Three distinct/);
    const incomplete=structuredClone(summaries);incomplete[1].passed=true;incomplete[1].workload_complete=false;
    assert.equal(browserMatrixCycleOutcome(incomplete).passed,false);
});

// Real fixture helpers, public SDK-shaped status/completion and actual teardown.
// No transport, decoder or matrix qualification is simulated by these controls.
async function cleanupMoveFixture({ordinal=0,sourceGeneration=1,status='completed',prepared=true}={}){
    const {browserMatrixCleanupMoves}=await import('../lib/browser_matrix_programme.mjs');
    let serial=0;const plan=browserMatrixPlan(()=>String(++serial)),route=plan.routes[0];
    const intent=plan.moves.find(value=>value.source===0&&value.ordinal===ordinal),events=[],calls={restore:0,status:0,wait:0,end:0,targetFactory:0,close:0,otherEnds:0,sessionEnds:0,options:[]};
    const f={clock:1000,plan,route,intent,events,calls,status,sourceGeneration,sourceState:status==='completed'?'retired':'ready',onStatus(){},onWait(){},onSourceRead(){},onJournal(){},endFailure:false};
    const actors=Object.fromEntries(['participant-1','participant-2','participant-3','participant-4','replacement-1','controller-1','controller-2'].map(role=>[role,
        {deviceId:'device-'+role,endpointId:'endpoint-'+role,client:{transport:{physicalEpoch:1},operations:{},async close(){calls.close++;}}}]));
    const sourceId=ordinal?'replacement_opr-outbound-original':route.connection,record='opr-cleanup-'+ordinal,targetId='replacement_'+record;
    const sid=plan.sessions[route.session].id,cid='original-cleanup-room',participant='original-participant';
    const snapshot=(id,generation,state)=>({id,conversationId:cid,sessionId:sid,participantId:participant,generation,state,streamRevision:3});
    f.targetSnapshot=snapshot(targetId,sourceGeneration+1,'ready');
    const end=(id,owner)=>async(input,options)=>{
        calls.end++;calls.options.push(options.timeoutMs);assert.equal(options.operationId,route.end);assert.equal(input.reasonCode,0);
        f.actualEnd={id,owner,operationId:options.operationId};
        return {referenceJson:()=>({operation_id:route.end}),async wait(value){calls.options.push(value.timeoutMs);
            if(f.endFailure)throw Object.assign(Error('Original end response uncertain'),{kind:'deadline',certainty:'uncertain'});
            return {snapshot:{state:'completed'}};}};
    };
    const source={id:sourceId,conversationId:cid,sessionId:sid,
        async refresh(value){calls.options.push(value.timeoutMs);f.onSourceRead();return snapshot(sourceId,sourceGeneration+(f.sourceState==='retired'?1:0),f.sourceState);},
        end:end(sourceId,ordinal?'replacement-1':'participant-1')};
    const target={id:targetId,conversationId:cid,sessionId:sid,
        async refresh(value){calls.options.push(value.timeoutMs);return {...f.targetSnapshot};},end:end(targetId,intent.targetRole)};
    const connections=plan.routes.map((r,index)=>index===0?source:{id:r.connection,conversationId:cid,sessionId:plan.sessions[r.session].id,
        async refresh(value){calls.options.push(value.timeoutMs);return {streamRevision:3};},
        async end(_input,options){calls.otherEnds++;assert.equal(options.operationId,r.end);
            return {referenceJson:()=>({operation_id:r.end}),wait:async()=>({snapshot:{state:'completed'}})};}});
    const sessions=plan.sessions.map(s=>({id:s.id,refresh:async()=>({topologyRevision:1}),async end(_input,options){
        calls.sessionEnds++;assert.equal(options.operationId,s.end);return {referenceJson:()=>({operation_id:s.end}),wait:async()=>({snapshot:{state:'completed'}})};}}));
    const state={conversation:{id:cid},sessions,connections},targetSession={connections:{byId(id){calls.targetFactory++;assert.equal(id,targetId);return target;}}};
    let reference;
    const key=()=>({command:'connection.replace',operationId:intent.operation,
        scope:{conversationId:cid,sessionId:sid,connectionId:sourceId}});
    f.statusSnapshot={key:key(),state:status,revision:7,...(status==='unknown'?{}:{recordId:record})};
    f.completion={snapshot:{key:key(),state:'completed',recordId:record,revision:8},value:{activationState:'ready',
        connection:{id:targetId,conversationId:cid,sessionId:sid},binding:{generation:sourceGeneration+1,participantId:participant,membershipEpoch:1,
            deviceId:actors[intent.targetRole].deviceId,endpointId:actors[intent.targetRole].endpointId},
        retired:[sourceId],retained:[{id:plan.routes[1].connection}],admitted:[{id:targetId}]}};
    const operation={referenceJson:()=>structuredClone(reference),async status(options){calls.status++;calls.options.push(options.timeoutMs);f.onStatus();return structuredClone(f.statusSnapshot);},
        async wait(options){calls.wait++;calls.options.push(options.timeoutMs);f.onWait();return structuredClone(f.completion);}};
    const sourceRole=ordinal?'replacement-1':'participant-1',kind={command:'connection.replace'};
    actors[sourceRole].client.operations.restore=(saved,actualKind)=>{calls.restore++;assert.equal(actualKind,kind);reference=structuredClone(saved);return operation;};
    const registry=browserMatrixCleanupMoves({state,actors,plan,journal:async row=>{events.push(row);f.onJournal(row);},operationKind:kind});
    const entry=registry.track({intent,route,before:{connection:source,role:sourceRole},targetSession,retain:plan.routes[1].connection,
        sourceSnapshot:snapshot(sourceId,sourceGeneration,'ready')});
    if(prepared)registry.prepared(entry,target);
    const left=()=>{const timeoutMs=6000-f.clock;assert.ok(timeoutMs>0,'Original browser teardown deadline');return {timeoutMs};};
    f.run=()=>retireBrowserMatrixAdmission({state,actors,plan,journal:async event=>events.push(event),cleanupMoves:registry,now:()=>f.clock,terminalMs:1000});
    return Object.assign(f,{registry,entry,operation,state,actors,source,target,targetSession,left,sourceRole,reference});
}

test('lost original replacement completion recovers only its target for cleanup without changing the failed move',async()=>{
    const f=await cleanupMoveFixture({prepared:false});
    const programme={passed:false,moves:[],failures:[{operation_id:f.intent.operation,kind:'deadline',certainty:'read-only'}]},before=structuredClone(programme);
    const result=await f.run();
    assert.equal(result.passed,true);assert.equal(result.originalEndsCompleted,6);
    assert.equal(result.originalSessionsEnded,2);assert.equal(result.originalClientsClosed,7);assert.equal(f.calls.otherEnds,5);
    assert.equal(f.calls.restore,1,'The public original reference exists even without a returned operation handle');
    assert.equal(f.calls.status,1);assert.equal(f.calls.wait,1);assert.equal(f.calls.targetFactory,1);assert.equal(f.calls.end,1);
    assert.deepEqual(f.actualEnd,{id:f.target.id,owner:f.intent.targetRole,operationId:f.route.end});
    assert.equal(result.originalConnectionEnds[0].original_connection_id,f.source.id);
    assert.equal(result.originalConnectionEnds[0].connection_id,f.target.id);
    assert.equal(f.events.filter(row=>row.kind==='end-intent').length,6);
    assert.deepEqual(f.events.filter(row=>row.kind==='end-intent').map(row=>row.operationId).sort(),f.plan.routes.map(route=>route.end).sort());
    assert.equal(f.events.filter(row=>row.kind==='cleanup-move-recovered').length,1);
    assert.deepEqual(programme,before,'Cleanup recovery cannot qualify the original8second movement');
    assert.equal(result.cleanupRecovery.retained_move_records,0);assert.equal(result.cleanupRecovery.retained_sdk_references,0);
});

test('unknown or preparing original moves never supply an inferred replacement cleanup owner',async()=>{
    for(const status of ['unknown','accepted','preparing','failed','cancelled']){
        const f=await cleanupMoveFixture({status});const result=await f.run();
        assert.equal(result.passed,true);assert.equal(f.actualEnd.id,f.source.id);assert.equal(f.calls.wait,0);assert.equal(f.calls.targetFactory,0);
        assert.equal(result.cleanupRecovery.observations[0].recovered,false);
    }
    for(const hook of ['status','source-read']){
        const f=await cleanupMoveFixture({status:'unknown'});
        const change=()=>{f.actors[f.sourceRole].client.transport.physicalEpoch++;};
        if(hook==='status')f.onStatus=change;else f.onSourceRead=change;
        const result=await f.run();
        assert.equal(result.passed,false);assert.equal(f.calls.end,0);assert.equal(f.calls.wait,0);
        assert.equal(f.calls.targetFactory,0);assert.equal(f.state.connections[0],f.source);
        assert.equal(result.cleanupRecovery.retained_sdk_references,0);
    }
    const f=await cleanupMoveFixture({status:'preparing'});f.sourceState='retired';const result=await f.run();
    assert.equal(result.passed,false);assert.equal(f.calls.end,0);assert.equal(f.calls.wait,0);
    assert.equal(result.originalConnectionEnds[0].failure.stage,'original-current-owner');
});

test('foreign original record candidate generation or physical owner cannot select a cleanup target',async()=>{
    for(const change of [f=>{f.statusSnapshot.recordId='foreign-record';},f=>{f.completion.snapshot.recordId='foreign-record';},
        f=>{f.completion.value.connection.id='foreign-candidate';},f=>{f.completion.value.binding.generation++;},
        f=>{f.targetSnapshot.generation++;},f=>{f.actors[f.intent.targetRole].client.transport.physicalEpoch++;},
        f=>{f.completion.value.retained[0].id='foreign-retained';},
        f=>{f.onSourceRead=()=>{f.actors[f.sourceRole].client.transport.physicalEpoch++;};},
        f=>{f.onJournal=()=>{f.actors[f.intent.targetRole].client.transport.physicalEpoch++;};}]){
        const f=await cleanupMoveFixture();change(f);const result=await f.run();
        assert.equal(result.passed,false);assert.equal(f.calls.end,0);assert.equal(f.events.some(row=>row.kind==='end-accepted'&&row.operationId===f.route.end),false);
        assert.equal(result.cleanupRecovery.released,true);assert.equal(result.cleanupRecovery.retained_sdk_references,0);
    }
});

test('return movement cleanup uses its original source generation and actual target actor',async()=>{
    const f=await cleanupMoveFixture({ordinal:1,sourceGeneration:2});const result=await f.run();
    assert.equal(result.passed,true);assert.equal(f.actualEnd.owner,'participant-1');
    assert.equal(f.actualEnd.operationId,f.route.end);assert.equal(result.cleanupRecovery.observations[0].generation,3);
    assert.equal(result.cleanupRecovery.observations[0].source_connection_id,'replacement_opr-outbound-original');
    assert.equal(f.calls.end,1);assert.equal(f.calls.restore,1);
});

test('original cleanup deadline and uncertain end response never trigger another mutation or retained registry',async()=>{
    const late=await cleanupMoveFixture();late.onStatus=()=>{late.clock+=4900;};late.onWait=()=>{late.clock+=101;};
    const missed=await late.run();assert.equal(missed.passed,false);assert.equal(late.calls.end,0);
    assert.ok(late.calls.options.every(value=>value>0&&value<=5000));assert.equal(missed.cleanupRecovery.retained_sdk_references,0);
    const f=await cleanupMoveFixture();f.endFailure=true;const result=await f.run();
    assert.equal(result.passed,false);assert.equal(f.calls.end,1,'An uncertain accepted end is not submitted again');
    assert.equal(f.events.filter(row=>row.kind==='end-accepted'&&row.operationId===f.route.end).length,1);
    assert.equal(result.originalConnectionEnds[0].failure.stage,'original-end-wait');
    assert.equal(result.originalConnectionEnds[0].failure.certainty,'uncertain');
    assert.equal(f.registry.release(),result.cleanupRecovery,'Reference release is idempotent');
    await assert.rejects(f.registry.resolve(f.target,0,f.left),/released/);
});
