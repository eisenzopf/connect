import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
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
  const first=room.send(input(),mutation),second=room.send(input(),{...opts,timeoutMs:20,operationId:'expired'}),third=room.send(input(),{...mutation,operationId:'third'});
  await assert.rejects(second,error('deadline','not-submitted'));await first;await third;
  const requests=await calls(dir);assert.equal(requests.length,3);assert.ok(requests[2].request.timeout_ms<1400);assert.ok(requests.every(r=>r.request.action.mutation?.operation_id!=='expired'));
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
