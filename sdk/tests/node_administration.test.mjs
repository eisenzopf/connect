import {inspect} from 'node:util';
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {join} from 'node:path';
import {NodeAdministrativeClient,NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),helper=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500},mutation={...opts,operationId:'original'};
const draft=()=>({type:'identity',operation_id:'original',alias:' local ',kind:'human',display_name:'private backend label'});
const saved=()=>({schema:'connect-operation-reference/1',operation_id:'original',command:'identity.provision',scope:{}});
const error=(kind,certainty,id)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);if(id)assert.equal(e.operation.operationId,id);assert.ok(!e.message.includes('private backend label'));return true;};
const lines=async(dir,name)=>(await readFile(join(dir,name),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-admin-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json'),executable=join(dir,'connect.mjs');
 await writeFile(path,JSON.stringify(config),{mode:0o600});await writeFile(executable,'#!/usr/bin/env node\nprocess.env.CONNECT_SDK_FIXTURE_CONFIG='+JSON.stringify(path)+';\nawait import('+JSON.stringify(pathToFileURL(helper).href)+');\n',{mode:0o700});
 const original={config:path,executable},client=new NodeAdministrativeClient(original);original.executable='/missing-after-capture';
 try{await run(client,dir);}finally{
  await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');
  for(const v of await lines(dir,'admin-validation.jsonl'))assert.throws(()=>process.kill(v.pid,0),e=>e.code==='ESRCH');
  await rm(dir,{recursive:true,force:true});
 }
}
test('trusted backend uses a separate owned profile and local normalized immutable command without participant APIs',async()=>{
 await fixture({},async(client,dir)=>{
  assert.equal(client.conversations,undefined);assert.equal(client.raw,undefined);assert.equal(NodeClient.prototype.provision,undefined);
  const value=draft(),command=client.validateProvisioning(value);value.alias='changed';value.display_name='changed';
  for(const text of [inspect(command),JSON.stringify(command),inspect(client),JSON.stringify(client)])assert.ok(!text.includes('private backend label'));
  assert.equal(command.value.alias,'local');assert.equal(command.value.display_name,'private backend label');assert.equal(command.operationId,'original');assert.throws(()=>command.value.alias='other');assert.deepEqual(JSON.parse(JSON.stringify(command.referenceJson())),saved());
  const exported=command.referenceJson();exported.operation_id='other';assert.equal(command.referenceJson().operation_id,'original');
  assert.equal((await lines(dir,'requests.jsonl')).length,0);const checks=await lines(dir,'admin-validation.jsonl');assert.deepEqual(checks[0].argv,['sdk-validate-provisioning']);
  const operation=await client.provision(command,mutation),result=await operation.wait(opts);assert.deepEqual(JSON.parse(JSON.stringify(result.value)),{kind:'identity',identity_id:'created',created:true});assert.throws(()=>result.value.identity_id='foreign');
  const calls=await lines(dir,'requests.jsonl');assert.equal(calls[0].profile,'connect-sdk-administration/2');assert.equal(calls[0].request.action.type,'administrative_provision');assert.equal(calls[0].request.action.command.alias,'local');assert.equal(calls[0].request.action.command.operation_id,'original');
  assert.equal(JSON.parse(await readFile(join(dir,'argv.json'),'utf8'))[0],'sdk-admin-stdio');
 });
});
test('administrative operations preserve original record, metadata-only recovery and separate cancel/reconcile decision types',async()=>{
 await fixture({},async(client,dir)=>{
  const operation=await client.provision(client.validateProvisioning(draft()),mutation),first=await operation.status(opts),copy=operation.referenceJson();
  assert.equal(first.key.command,'identity.provision');assert.deepEqual(first.key.scope,{});assert.equal((await operation.waitUntilReady(opts)).state,'completed');
  const before=(await lines(dir,'requests.jsonl')).length,restored=client.operations.restore(copy),recovered=client.operations.recover(operation.reference);assert.equal((await lines(dir,'requests.jsonl')).length,before);
  assert.equal((await restored.status(opts)).recordId,first.recordId);assert.equal((await recovered.wait(opts)).snapshot.recordId,first.recordId);
  const cancel=await operation.cancel({...opts,operationId:'cancel'}),cancelled=await cancel.wait(opts);assert.equal(cancelled.value.target,'original');assert.equal(cancelled.value.decision.operationId,'cancel');assert.equal(cancel.reference.command,'operation.cancel');
  const reconciled=await operation.reconcile({outcome:'completed',evidenceId:'evidence'},{...opts,operationId:'reconcile'});assert.equal((await reconciled.wait(opts)).value.evidenceId,'evidence');
  assert.equal((await client.operations.recover(cancel.reference).status(opts)).key.command,'operation.cancel');
  const checks=await lines(dir,'admin-validation.jsonl');assert.ok(checks.some(v=>v.argv.includes('--reference')));
  const submits=(await lines(dir,'requests.jsonl')).filter(v=>v.request.action.type==='administrative_provision');assert.equal(submits.length,1);
 });
});
test('forged administrative command, mismatched original ID and extensions refuse locally without submission',async()=>{
 await fixture({},async(client,dir)=>{
  const command=client.validateProvisioning(draft());
  assert.throws(()=>client.provision({value:command.value},mutation),error('validation','not-submitted'));
  assert.throws(()=>client.provision(command,{...opts,operationId:'different'}),error('validation','not-submitted','different'));
  for(const extensions of [null,[],7])assert.throws(()=>client.provision(command,{...mutation,extensions}),error('validation','not-submitted','original'));
  assert.throws(()=>client.provision(command,{...mutation,extensions:{extra:true}}),error('unsupported','not-submitted','original'));
  assert.throws(()=>client.provision(command,{...mutation,signal:AbortSignal.abort()}),error('cancelled','not-submitted','original'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
 });
});
test('invalid trusted command and foreign/scoped/continued saved references are local and grant no authority',async()=>{
 await fixture({},async(client,dir)=>{
  for(const value of [null,{}, {...draft(),type:'not-a-command'}, {...draft(),operation_id:'bad!'}])assert.throws(()=>client.validateProvisioning(value),error('validation','not-submitted'));
  for(const value of [{...saved(),scope:{conversation_id:'room'}},{...saved(),continuation:{}},{...saved(),command:'message.send'},{...saved(),operation_id:'bad!'}])assert.throws(()=>client.operations.restore(value),error('validation','not-submitted'));
  assert.throws(()=>client.operations.recover({operationId:'original'}),error('validation','not-submitted'));assert.equal((await lines(dir,'requests.jsonl')).length,0);
 });
});
test('malformed trusted local validator profile, scope, kind or identity fences the original administrative owner',async()=>{
 for(const adminValidator of ['profile','scope','kind-array','foreign-id'])await fixture({adminValidator},async(client,dir)=>{
  assert.throws(()=>client.validateProvisioning(draft()),error('protocol','not-submitted'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
 });
});
test('administrative response corruption preserves submitted uncertainty and stable record fences',async()=>{
 for(const mode of ['profile','wrong-id','wrong-reference','duplicate','extra','error-kind-array'])await fixture({mode},async(client,dir)=>{
  const command=client.validateProvisioning(draft());await assert.rejects(client.provision(command,mutation),error('protocol','uncertain','original'));
  assert.throws(()=>client.provision(command,mutation),error('configuration','not-submitted','original'));assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
 for(const mode of ['wrong-scope','state-array','record-change'])await fixture({mode},async(client)=>{
  const operation=client.operations.restore(saved());if(mode==='record-change')await operation.status(opts);
  await assert.rejects(operation.status(opts),error('protocol','read-only','original'));
 });
});
test('administrative sent timeout fences original owner, keeps mutation identity and never retries',async()=>{
 await fixture({mode:'partial'},async(client,dir)=>{
  const command=client.validateProvisioning(draft());await assert.rejects(client.provision(command,{...mutation,timeoutMs:50}),error('deadline','uncertain','original'));
  assert.throws(()=>client.provision(command,mutation),error('configuration','not-submitted','original'));assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
});
test('trusted local validator input and runtime stay bounded and wrapper accommodates a captured64KiB command',async()=>{
 await fixture({},async(client,dir)=>{
  assert.throws(()=>client.validateProvisioning({...draft(),display_name:'x'.repeat(65536)}),error('validation','not-submitted'));assert.equal((await lines(dir,'admin-validation.jsonl')).length,0);
  const command=client.validateProvisioning({...draft(),display_name:'x'.repeat(65000)});const operation=await client.provision(command,mutation);assert.equal(operation.reference.operationId,'original');
 });
 await fixture({adminValidator:'hang'},async(client)=>{const start=performance.now();assert.throws(()=>client.validateProvisioning(draft()),error('configuration','not-submitted'));assert.ok(performance.now()-start<3000);});
});

test('helper startup failures preserve original intent and distinguish unsent mutations from reads',async()=>{
 for(const kind of ['authentication','configuration','trust','deadline'])for(const certainty of ['read-only','not-submitted'])await fixture({mode:'startup',kind,certainty},async(client,dir)=>{
  const command=client.validateProvisioning(draft());await assert.rejects(client.provision(command,mutation),error(kind,'not-submitted','original'));
  const operation=client.operations.restore(saved());await assert.rejects(operation.status(opts),error(kind,'read-only','original'));
  const calls=await lines(dir,'requests.jsonl');assert.equal(calls.length,2);assert.equal(calls[0].request.action.mutation.operation_id,'original');
 });
 for(const mode of ['phase-array','phase-missing','profile-old','startup-success','startup-uncertain','startup-observation'])await fixture({mode},async(client,dir)=>{
  const command=client.validateProvisioning(draft());await assert.rejects(client.provision(command,mutation),error('protocol','uncertain','original'));
  assert.throws(()=>client.provision(command,mutation),error('configuration','not-submitted','original'));assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
});

test('trusted sent cancellation gracefully joins its original backend helper without mutation replay',async()=>{
 await fixture({mode:'partial',signalDrainMs:60},async(client,dir)=>{
  const command=client.validateProvisioning(draft()),abort=new AbortController(),pending=client.provision(command,{...mutation,signal:abort.signal});
  while(!(await lines(dir,'requests.jsonl')).length)await new Promise(resolve=>setTimeout(resolve,5));
  abort.abort();await assert.rejects(pending,error('cancelled','uncertain','original'));await client.close(opts);assert.deepEqual(JSON.parse(await readFile(join(dir,'drained.json'),'utf8')),{drained:true});assert.deepEqual(JSON.parse(await readFile(join(dir,'exit.json'),'utf8')),{code:0});assert.deepEqual(await lines(dir,'signals.jsonl'),['SIGTERM']);assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
});
