import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError,sdkRevision} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500},mutation={...opts,operationId:'original'};
const saved=()=>({schema:'connect-operation-reference/1',operation_id:'original',command:'message.send',scope:{conversation_id:'room'}});
const input=()=>({messageId:'message',to:{first:'participant',rest:[]},text:'original'});
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=(kind,certainty)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);return true;};
async function fixture(config,run) {
 const base=join(root,'.local/gate-6/node-sdk-unit');await mkdir(base,{recursive:true,mode:0o700});
 const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});
 const client=new NodeClient({config:path,executable});
 try{await run(client,dir,path);}finally{
  await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));
  if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});
 }
}
test('typed restore and recover are local, immutable and bound to the declared result/scope',async()=>{
 await fixture({},async(client,dir)=>{
  const source=saved(),op=client.operations.restore(source,client.operationKinds.message_send);source.scope.conversation_id='foreign';
  assert.equal(op.reference.scope.conversationId,'room');assert.throws(()=>op.reference.scope.conversationId='foreign');assert.throws(()=>op.reference={});
  const json=op.referenceJson();json.operation_id='foreign';assert.equal(op.referenceJson().operation_id,'original');
  assert.deepEqual(JSON.parse(JSON.stringify(client.operations.recover(op.reference).referenceJson())),saved());
  assert.throws(()=>client.operations.recover({...op.reference}),error('validation','not-submitted'));
  for(const value of [null,{}, {...saved(),extra:true},{...saved(),scope:{}},{...saved(),scope:{session_id:'sid'}},{...saved(),scope:{conversation_id:'room',connection_id:'conn'}},{...saved(),scope:{conversation_id:'room',session_id:'sid'}},{...saved(),continuation:{}},{...saved(),operation_id:'invalid!'}])
   assert.throws(()=>client.operations.restore(value,client.operationKinds.message_send),error('validation','not-submitted'));
  assert.throws(()=>client.operations.restore(saved(),client.operationKinds.conversation_create),error('validation','not-submitted'));
  assert.throws(()=>client.operations.restore(saved(),{command:'message.send'}),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,0);
  await client.close(opts);assert.deepEqual(JSON.parse(JSON.stringify(client.operations.restore(saved(),client.operationKinds.message_send).referenceJson())),saved());
 });
});
test('typed mutations capture original intent; handles complete through a separate SDK journal wait',async()=>{
 await fixture({},async(client,dir,path)=>{
  const room=client.conversations.byId('room'),body=input(),options={...mutation};
  const promise=room.send(body,options);body.text='changed';options.operationId='changed';
  const op=await promise;assert.equal(op.reference.operationId,'original');assert.equal(op.reference.command,'message.send');
  assert.deepEqual((await op.status(opts)).key,op.reference);assert.equal((await op.waitUntilReady(opts)).state,'completed');
  const completed=await op.wait(opts);assert.equal(completed.value.messageId,'message');assert.equal(completed.value.boundary,'local-mutation');
  assert.equal(completed.snapshot.recordId,'record-original');assert.throws(()=>completed.snapshot.state='ready');
  const requests=await calls(dir);assert.equal(requests[0].request.action.text,'original');assert.equal(requests[0].request.action.mutation.operation_id,'original');
  assert.deepEqual(requests.map(r=>r.request.action.type),['conversation_send','operation','operation','operation']);
  assert.deepEqual(JSON.parse(await readFile(join(dir,'argv.json'),'utf8')),['sdk-stdio','--config',path]);
 });
});
test('typed Conversation creation resolves a handle and refresh remains current',async()=>{
 await fixture({},async client=>{
  const operation=await client.conversations.create({participants:['identity'],policy:'persistent'},mutation);
  const room=(await operation.wait(opts)).value;assert.equal(room.id,'room');assert.equal((await room.refresh(opts)).id,'room');assert.throws(()=>room.id='foreign');
 });
});
test('decisions retain their own IDs and scope separately from the target reference',async()=>{
 await fixture({},async(client,dir)=>{
  const target=client.operations.restore(saved(),client.operationKinds.message_send);
  const cancel=await target.cancel({...opts,operationId:'cancel'});assert.equal(cancel.reference.command,'operation.cancel');
  assert.equal((await cancel.wait(opts)).value.target,'original');
  const reconciliation=await target.reconcile({outcome:'completed',evidenceId:'evidence'},{...opts,operationId:'reconcile'});
  assert.equal((await reconciliation.wait(opts)).value.evidenceId,'evidence');
  const requests=await calls(dir);assert.equal(requests[0].request.action.reference.operation_id,'original');
  assert.equal(requests[0].request.action.action.mutation.operation_id,'cancel');
 });
});
test('structured SDK errors retain trust, missing receipts and recorded certainty without raw diagnostics',async()=>{
 for(const [kind,certainty]of [['trust','not-submitted'],['receipt-unavailable','recorded'],['authentication','refused']])await fixture({mode:'refuse',kind,certainty},async client=>{
  const operation=client.operations.restore(saved(),client.operationKinds.message_send);
  await assert.rejects(operation.wait(opts),e=>{error(kind,certainty)(e);assert.equal(e.operation.operationId,'original');assert.equal(e.problem.reason,'receipt-expired');return true;});
  assert.equal((await operation.status(opts)).state,'completed');
 });
});
test('invalid typed inputs and already aborted mutations never write the helper',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room');
  for(const value of [null,{...input(),messageId:'bad!'},{...input(),to:{first:'bad!',rest:[]}},{...input(),text:undefined}])
   assert.throws(()=>room.send(value,mutation),error('validation','not-submitted'));
  await assert.rejects(client.conversations.create({participants:['bad!'],policy:'persistent'},mutation),error('validation','not-submitted'));
  assert.throws(()=>room.send(input(),{...mutation,signal:AbortSignal.abort()}),error('cancelled','not-submitted'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('queued typed deadline removes only unsent work and passes the remaining deadline to Rust',async()=>{
 await fixture({delayMs:100},async(client,dir)=>{
  const room=client.conversations.byId('room');await room.refresh(opts);
  const started=performance.now();
  const first=room.send(input(),mutation),second=room.send(input(),{...opts,timeoutMs:20,operationId:'expired'}),third=room.send(input(),{...mutation,operationId:'third'});
  await assert.rejects(second,error('deadline','not-submitted'));
  // A 100ms fixture timer may fire slightly early. Assert the independently
  // observed queue residence, rather than assuming an exact timer duration.
  const queuedFor=performance.now()-started;await first;await third;
  const requests=await calls(dir);assert.equal(requests.length,3);assert.ok(queuedFor>0);assert.ok(requests[2].request.timeout_ms<=Math.ceil(opts.timeoutMs-queuedFor)+2);assert.ok(requests.every(r=>r.request.action.mutation?.operation_id!=='expired'));
 });
});
test('submitted typed deadline fences the original helper; saved recovery never resubmits',async()=>{
 await fixture({mode:'partial'},async(client,dir)=>{
  await assert.rejects(client.conversations.byId('room').send(input(),{...mutation,timeoutMs:100}),error('deadline','uncertain'));
  const restored=client.operations.restore(saved(),client.operationKinds.message_send);
  assert.throws(()=>restored.status(opts),error('configuration','read-only'));assert.equal((await calls(dir)).length,1);
 });
});
test('corrupt helper framing, cross-operation observations and wrong typed results fail closed',async()=>{
 for(const mode of ['wrong-id','profile','duplicate','extra','utf8','oversize','wrong-scope','wrong-result'])await fixture({mode},async client=>{
  const op=client.operations.restore(saved(),client.operationKinds.message_send);
  await assert.rejects(op.wait(opts),error('protocol','read-only'));
  assert.throws(()=>op.status(opts),error('configuration','read-only'));
 });
 await fixture({mode:'wrong-reference'},async client=>{
  await assert.rejects(client.conversations.byId('room').send(input(),mutation),error('protocol','uncertain'));
 });
});
test('readiness/completion never accept an arbitrary status as a completed result',async()=>{
 for(const state of ['accepted','preparing','interrupted','unknown'])await fixture({state},async client=>{
  const operation=client.operations.restore(saved(),client.operationKinds.message_send);
  await assert.rejects(operation.wait(opts),error('protocol','read-only'));
 });
});
test('missing executable is not submitted and explicit close joins; hanging close reports incomplete cleanup',async()=>{
 const missing=new NodeClient({config:'/nonexistent',executable:'/nonexistent/connect'});
 await assert.rejects(missing.conversations.byId('room').send(input(),mutation),error('configuration','not-submitted'));await missing.close(opts);
 await fixture({mode:'hang-close'},async client=>{
  await client.conversations.byId('room').refresh(opts);
  await assert.rejects(client.close({timeoutMs:30}),error('cleanup-incomplete','read-only'));await client.close(opts);
 });
});

test('one owned operation rejects a changed host record identity across separate API calls',async()=>{
 await fixture({mode:'record-change'},async client=>{
  const op=client.operations.restore(saved(),client.operationKinds.message_send);
  assert.equal((await op.status(opts)).recordId,'record-original');
  await assert.rejects(op.wait(opts),error('protocol','read-only'));
  assert.throws(()=>op.status(opts),error('configuration','read-only'));
 });
});

test('typed participants preserve Conversation scope and independent mutation/revision identity',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room'),revision=sdkRevision(1);
  const add=await room.participants.add({identityId:'identity',kind:'human',role:'participant',expectedRevision:revision},mutation);
  const savedAdd=add.referenceJson(),participant=(await add.wait(opts)).value;
  assert.equal(participant.id,'participant');assert.equal(participant.conversationId,'room');assert.throws(()=>participant.id='foreign');
  assert.equal((await participant.refresh(opts)).identityId,'identity');
  const update=await participant.update({role:'observer',expectedConversationRevision:revision},{...opts,operationId:'role-update'});
  assert.equal((await update.wait(opts)).value.id,participant.id);
  const removed=await participant.remove({expectedConversationRevision:revision},{...opts,operationId:'remove'});
  assert.equal((await removed.wait(opts)).value.id,participant.id);
  const restored=client.operations.restore(savedAdd,client.operationKinds.conversation_participant_add);
  assert.equal((await restored.wait(opts)).value.id,participant.id);
  const requests=await calls(dir);const mutations=requests.filter(r=>r.request.action.type.startsWith('participant_')&&r.request.action.type!=='participant_refresh');
  assert.deepEqual(mutations.map(r=>r.request.action.mutation.operation_id),['original','role-update','remove']);
  assert.ok(mutations.every(r=>r.request.action.expected_revision===1&&r.request.action.conversation_id==='room'));
 });
});
test('invalid participant scopes, identity kinds and noninteger revisions never write',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room'),participant=room.participants.byId('participant');
  for(const expected of [-1,NaN,Infinity,1.5,Number.MAX_SAFE_INTEGER+1])assert.throws(()=>participant.remove({expectedConversationRevision:expected},mutation),error('validation','not-submitted'));
  assert.throws(()=>room.participants.add({identityId:'identity',kind:'robot',role:'participant',expectedRevision:1},mutation),error('validation','not-submitted'));
  assert.throws(()=>room.participants.byId('bad!'),error('validation','not-submitted'));
  for(const kind of [client.operationKinds.conversation_participant_add,client.operationKinds.conversation_participant_update,client.operationKinds.conversation_participant_remove])
   assert.throws(()=>client.operations.restore({...saved(),command:kind.command,scope:{conversation_id:'room',session_id:'session'}},kind),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('participant results and refresh observations cannot cross Conversation scope',async()=>{
 await fixture({mode:'wrong-participant'},async client=>{
  const participant=client.conversations.byId('room').participants.byId('participant');
  await assert.rejects(participant.refresh(opts),error('protocol','read-only'));
 });
 await fixture({mode:'wrong-participant'},async client=>{
  const op=client.operations.restore({...saved(),command:'conversation.participant.add'},client.operationKinds.conversation_participant_add);
  await assert.rejects(op.wait(opts),error('protocol','read-only'));
 });
});

test('Node capabilities reflect exposed interfaces and immutable feature sets, never helper media ownership',async()=>{
 await fixture({},async(client,dir)=>{
  const snapshot=await client.capabilities.refresh(opts);
  assert.equal(snapshot.profile,'connect/2');assert.equal(snapshot.runtimeRole,'node');assert.equal(snapshot.selectedTransport,'quic');assert.equal(snapshot.fallbackReason,undefined);
  assert.deepEqual([...snapshot.features],['original-id-operations','raw-canonical','message-receipts']);assert.equal(snapshot.features.size,3);
  assert.equal(snapshot.features.add,undefined);assert.throws(()=>snapshot.features.size=99);assert.throws(()=>snapshot.runtimeRole='rust');
  const seen=[];snapshot.features.forEach(function(value,key,owner){assert.equal(this,'context');assert.equal(owner,snapshot.features);seen.push([value,key]);},'context');assert.deepEqual(seen,[['original-id-operations','original-id-operations'],['raw-canonical','raw-canonical'],['message-receipts','message-receipts']]);
  await client.capabilities.require('original-id-operations',opts);
  for(const feature of ['native-programmatic-media','durable-events','incoming-invitations','unknown'])await assert.rejects(client.capabilities.require(feature,opts),error('unsupported','read-only'));
  const before=(await calls(dir)).length;
  for(const feature of ['',null,'UPPER','bad space','x'.repeat(129)])await assert.rejects(client.capabilities.require(feature,opts),error('validation','read-only'));
  assert.throws(()=>client.capabilities.refresh({...opts,signal:AbortSignal.abort()}),error('cancelled','read-only'));
  assert.equal((await calls(dir)).length,before);
 });
});
test('capability profile, original runtime, transport and duplicate feature violations fence the helper',async()=>{
 for(const mode of ['caps-foreign','caps-transport','caps-duplicate'])await fixture({mode},async(client)=>{
  await assert.rejects(client.capabilities.refresh(opts),error('protocol','read-only'));
  assert.throws(()=>client.capabilities.refresh(opts),error('configuration','read-only'));
 });
});

test('typed status, decision and participant observations reject coercible nonstring enum values',async()=>{
 for(const mode of ['state-array','decision-state-array','participant-kind-array','conversation-kind-array'])await fixture({mode},async(client)=>{
  const room=client.conversations.byId('room');
  const read=()=>mode==='state-array'?client.operations.restore(saved(),client.operationKinds.message_send).status(opts):mode==='decision-state-array'?client.operations.restore({...saved(),operation_id:'cancel',command:'operation.cancel'},client.operationKinds.operation_cancel).wait(opts):mode==='participant-kind-array'?room.participants.byId('participant').refresh(opts):room.refresh(opts);
  await assert.rejects(read(),error('protocol','read-only'));
  assert.throws(()=>room.refresh(opts),error('configuration','read-only'));
 });
});

test('malformed SDK error kind and certainty fence the original helper and preserve submission certainty',async()=>{
 for(const mode of ['error-kind-array','error-certainty-array'])for(const write of [false,true])await fixture({mode},async(client,dir)=>{
  const room=client.conversations.byId('room');
  const request=write?room.send(input(),mutation):client.operations.restore(saved(),client.operationKinds.message_send).status(opts);
  await assert.rejects(request,e=>{error('protocol',write?'uncertain':'read-only')(e);assert.equal(e.operation.operationId,'original');return true;});
  assert.throws(()=>room.refresh(opts),error('configuration','read-only'));
  assert.equal((await calls(dir)).length,1);
 });
});


test('participant helper startup context proves no logical mutation was submitted and keeps read certainty',async()=>{
 await fixture({mode:'startup',kind:'authentication',certainty:'read-only'},async(client,dir)=>{
  const room=client.conversations.byId('room');
  await assert.rejects(room.send(input(),mutation),e=>{error('authentication','not-submitted')(e);assert.equal(e.operation.operationId,'original');return true;});
  const original=client.operations.restore(saved(),client.operationKinds.message_send);
  await assert.rejects(original.status(opts),e=>{error('authentication','read-only')(e);assert.equal(e.operation.operationId,'original');return true;});
  assert.equal((await calls(dir)).length,2);
 });
 for(const mode of ['phase-array','phase-missing','profile-old','startup-success','startup-uncertain','startup-observation'])await fixture({mode},async(client,dir)=>{
  const operation=client.operations.restore(saved(),client.operationKinds.message_send);await assert.rejects(operation.status(opts),error('protocol','read-only'));
  assert.throws(()=>client.conversations.byId('room').refresh(opts),error('configuration','read-only'));assert.equal((await calls(dir)).length,1);
 });
});
