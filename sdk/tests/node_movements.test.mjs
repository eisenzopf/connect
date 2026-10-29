import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500},mutation={...opts,operationId:'movement-original'};
const handoff=()=>({via:'controller',retain:'retained',from:'from',to:'to',candidate:'candidate',outgoing:'leave',role:{scope:'session'},expectedTopologyRevision:2});
const session=c=>c.conversations.byId('room').sessions.byId('session');
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=(kind,certainty)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);return true;};
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-movement-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});const client=new NodeClient({config:path,executable});
 try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
test('handoff captures original intent, role scope and topology without granting native ownership',async()=>{
 for(const variant of ['session','conversation','observe','handoff-no-native'])await fixture({mode:variant},async(client,dir)=>{
  const input=handoff();if(variant==='conversation')input.role={scope:'conversation',expectedConversationRevision:7};if(variant==='observe')input.outgoing='observe';
  const expected=structuredClone(input),pending=session(client).handoff(input,mutation);input.retain='changed';input.role.scope='changed';
  const op=await pending,saved=op.referenceJson(),done=await op.wait(opts);assert.equal(op.reference.scope.connectionId,'controller');assert.equal(saved.continuation.retained_connection_id,'retained');assert.equal(done.value.retained[0].id,'retained');assert.equal(done.value.admitted[0].id,'candidate');assert.equal(done.value.topologyRevision,3);assert.equal(done.value.conversationRevision,variant==='conversation'?8:undefined);assert.deepEqual(done.value.retired,variant==='observe'?[]:['controller']);assert.equal(done.value.roleChange?.targetParticipant,variant==='handoff-no-native'?undefined:'to');assert.throws(()=>done.value.retained.push('foreign'));assert.equal(done.value.admitted[0].receiptStreamRevision,undefined);
  const before=(await calls(dir)).length,recovered=client.operations.recover(op.reference);assert.equal((await calls(dir)).length,before);assert.equal((await recovered.wait(opts)).value.admitted[0].id,'candidate');
  const sent=(await calls(dir))[0].request.action;assert.equal(sent.input.retain,expected.retain);assert.equal(sent.input.role.scope,expected.role.scope);assert.equal(sent.input.mutation.operation_id,mutation.operationId);
 });
});
test('handoff carries the branded original consultation record and rejects injected or foreign continuations before I/O',async()=>{
 await fixture({},async(client,dir)=>{
  const saved={schema:'connect-operation-reference/1',operation_id:'consult-original',command:'session.consult',scope:{conversation_id:'room',session_id:'session',connection_id:'controller'},continuation:{kind:'consultation',retained_connection_id:'retained',record_id:'record-original'}};
  const consult=client.operations.restore(saved,client.operationKinds.session_consult);
  const op=await session(client).handoff({...handoff(),consultation:consult},mutation);await op.wait(opts);
  const sent=(await calls(dir))[0].request.action.input;assert.equal(sent.consultation.operation_id,'consult-original');assert.equal(sent.consultation.continuation.record_id,'record-original');assert.equal(sent.mutation.operation_id,mutation.operationId);
  const count=(await calls(dir)).length;
  for(const value of [null,{}, {reference:{...consult.reference}},client.operations.restore({...saved,scope:{...saved.scope,conversation_id:'foreign'}},client.operationKinds.session_consult),client.operations.restore({...saved,continuation:{...saved.continuation,retained_connection_id:'foreign'}},client.operationKinds.session_consult)])assert.throws(()=>session(client).handoff({...handoff(),consultation:value},mutation),error('validation','not-submitted'));
  assert.throws(()=>session(client).handoff({...handoff(),consultation:consult},{...opts,operationId:'consult-original'}),error('validation','not-submitted'));
  assert.throws(()=>session(client).handoff({...handoff(),consultation:consult},{...mutation,extensions:{consultation_record_id:'invented'}}),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,count);
 });
});
test('device move distinguishes endpoint and candidate targets with original scope and captured binding observations',async()=>{
 for(const target of [{endpoint:'endpoint'},{candidate:'candidate'}])await fixture({},async(client,dir)=>{
  const source=session(client).connections.byId('controller'),input={retain:'retained',target:{...target},expectedTopologyRevision:2},pending=source.replaceDevice(input,mutation);Object.assign(input.target,{endpoint:'changed'});
  const op=await pending,saved=op.referenceJson(),done=await op.wait(opts);assert.equal(op.reference.command,'connection.replace');assert.equal(saved.continuation.target.kind,'endpoint' in target?'endpoint':'candidate');assert.equal(done.value.binding.endpointId,'endpoint');assert.equal(done.value.binding.generation,3);assert.equal(done.value.binding.membershipEpoch,2);assert.equal(done.value.connection.id,'candidate' in target?'candidate':'admitted');assert.equal(done.value.connection.receiptStreamRevision,undefined);assert.deepEqual(done.value.retired,['controller']);assert.throws(()=>done.value.binding.endpointId='foreign');
  const restored=client.operations.restore(saved,client.operationKinds.connection_replace);assert.equal((await restored.wait(opts)).value.connection.id,done.value.connection.id);assert.equal((await calls(dir))[0].request.action.input.target.id,Object.values(target)[0]);
 });
});
test('invalid movement targets, role assertions, scopes and saved context shapes never write helper input',async()=>{
 await fixture({},async(client,dir)=>{
  for(const input of [null,{...handoff(),via:'bad!'},{...handoff(),retain:'controller'},{...handoff(),to:'from'},{...handoff(),candidate:'controller'},{...handoff(),candidate:'retained'},{...handoff(),outgoing:'invented'},{...handoff(),role:{scope:'conversation',expectedConversationRevision:Number.MAX_SAFE_INTEGER}},{...handoff(),role:{scope:'session',expectedConversationRevision:8}},{...handoff(),expectedTopologyRevision:-1}])assert.throws(()=>session(client).handoff(input,mutation),error('validation','not-submitted'));
  const c=session(client).connections.byId('controller');for(const input of [null,{retain:'controller',target:{endpoint:'endpoint'},expectedTopologyRevision:2},{retain:'retained',target:{endpoint:'endpoint',candidate:'candidate'},expectedTopologyRevision:2},{retain:'retained',target:{candidate:'controller'},expectedTopologyRevision:2},{retain:'retained',target:{candidate:'retained'},expectedTopologyRevision:2},{retain:'retained',target:{endpoint:'bad!'},expectedTopologyRevision:2}])assert.throws(()=>c.replaceDevice(input,mutation),error('validation','not-submitted'));
  const saved={schema:'connect-operation-reference/1',operation_id:'movement-original',command:'session.handoff',scope:{conversation_id:'room',session_id:'session',connection_id:'controller'},continuation:{kind:'handoff',retained_connection_id:'retained',source_participant:'from',target_participant:'to',candidate:null,role:{scope:'session'},outgoing:'observe'}};
  for(const v of [{...saved,scope:{conversation_id:'room',session_id:'session'}},{...saved,continuation:null},{...saved,continuation:{...saved.continuation,source_participant:'to'}},{...saved,continuation:{...saved.continuation,role:{scope:'session',expected_conversation_revision:8}}},{...saved,continuation:{...saved.continuation,extra:true}},{...saved,continuation:{...saved.continuation,outgoing:['observe']}}])assert.throws(()=>client.operations.restore(v,client.operationKinds.session_handoff),error('validation','not-submitted'));
  assert.throws(()=>client.operations.restore(saved,client.operationKinds.connection_replace),error('validation','not-submitted'));
  const moved={...saved,command:'connection.replace',continuation:{kind:'replacement',retained_connection_id:'retained',target:{kind:'endpoint',id:'endpoint'}}};
  for(const continuation of [{...moved.continuation,retained_connection_id:'controller'},{...moved.continuation,target:{kind:'candidate',id:'controller'}},{...moved.continuation,target:{kind:'candidate',id:'retained'}},{...moved.continuation,target:{kind:'endpoint',id:'bad!'}},{...moved.continuation,target:{kind:'endpoint',id:'endpoint',candidate:'extra'}},{...moved.continuation,kind:'handoff'}])assert.throws(()=>client.operations.restore({...moved,continuation},client.operationKinds.connection_replace),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('changed movement contexts, foreign topology, roles and binding observations fail closed',async()=>{
 const modes=['movement-context','movement-scope','movement-duplicate','movement-local-receipt','handoff-candidate','handoff-role','handoff-revision','handoff-outgoing','handoff-cleanup','move-endpoint','move-generation','move-foreign-connection','move-unretired','move-candidate'];
 for(const mode of modes)await fixture({mode},async(client)=>{
  const moving=mode.startsWith('move-'),submit=()=>moving?session(client).connections.byId('controller').replaceDevice({retain:'retained',target:mode==='move-candidate'?{candidate:'candidate'}:{endpoint:'endpoint'},expectedTopologyRevision:2},mutation):session(client).handoff({...handoff(),role:{scope:'conversation',expectedConversationRevision:7}},mutation);
  if(['movement-context','movement-scope'].includes(mode))await assert.rejects(submit(),error('protocol','uncertain'));
  else{const op=await submit();await assert.rejects(op.wait(opts),error('protocol','read-only'));assert.throws(()=>op.status(opts),error('configuration','read-only'));}
 });
});
