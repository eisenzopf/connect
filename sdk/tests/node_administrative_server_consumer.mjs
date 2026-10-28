// Installed public trusted backend. All endpoints are explicit isolated fixtures.
import assert from 'node:assert/strict';
import {readFile,realpath} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {NodeAdministrativeClient,NodeSdkError} from '@rudeless/connect';
const spec=JSON.parse(await readFile(process.argv[2],'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project+'/node_modules/'));
const client=new NodeAdministrativeClient({config:spec.config,executable:spec.executable}),opts={timeoutMs:4000};
const emit=value=>process.stdout.write(JSON.stringify(value)+'\n'),lines=createInterface({input:process.stdin});
let checkpoint='ready',observedKind=null,observedCertainty=null;
const capture=error=>{observedKind=error instanceof NodeSdkError?error.kind:null;observedCertainty=error instanceof NodeSdkError?error.certainty:null;};
try {
 emit({phase:'ready'});
 for await(const line of lines) {
  assert.ok(line.length<=128*1024);const request=JSON.parse(line);checkpoint=request.action;observedKind=null;observedCertainty=null;
  if(request.action==='apply') {
   const command=client.validateProvisioning(request.command),saved=command.referenceJson();
   if(typeof request.command.alias==='string'){const original=command.value.alias;request.command.alias='mutated-after-capture';assert.equal(command.value.alias,original);}
   const mutation={...opts,operationId:command.operationId},operation=await client.provision(command,mutation),first=await operation.wait(opts);
   assert.deepEqual(operation.referenceJson(),saved);assert.equal(first.snapshot.state,'completed');
   assert.throws(()=>command.value.operation_id='foreign');assert.throws(()=>operation.reference.scope.conversationId='foreign');
   const restored=client.operations.restore(saved),again=await restored.wait(opts);assert.equal(again.snapshot.recordId,first.snapshot.recordId);assert.deepEqual(again.value,first.value);
   assert.equal((await client.operations.recover(operation.reference).status(opts)).recordId,first.snapshot.recordId);
   const replay=await client.provision(command,mutation);assert.equal((await replay.wait(opts)).snapshot.recordId,first.snapshot.recordId);
   emit({phase:'applied',reference:saved,value:first.value,record_id:first.snapshot.recordId});
  } else if(request.action==='read') {
   const operation=client.operations.restore(request.reference),read=await operation.wait(opts);
   if(request.expected_record!==null)assert.equal(read.snapshot.recordId,request.expected_record);
   emit({phase:'read',value:read.value,record_id:read.snapshot.recordId});
  } else if(request.action==='read-refused') {
   const operation=client.operations.restore(request.reference);
   await assert.rejects(operation.status(opts),error=>{
    capture(error);assert.ok(error instanceof NodeSdkError);
    assert.ok(error.kind==='authentication'&&['refused','read-only'].includes(error.certainty)||request.revoked===true&&['cancelled','transport-uncertain'].includes(error.kind)&&error.certainty==='read-only');
    assert.equal(error.operation?.operationId,operation.reference.operationId);return true;
   });emit({phase:'read-refused',kind:observedKind,certainty:observedCertainty});
  } else if(request.action==='refuse') {
   const command=client.validateProvisioning(request.command);
   await assert.rejects(client.provision(command,{...opts,operationId:command.operationId}),error=>{
    capture(error);assert.ok(error instanceof NodeSdkError);assert.equal(error.kind,request.kind);assert.equal(error.certainty,request.certainty);assert.equal(error.operation.operationId,command.operationId);return true;
   });emit({phase:'refused'});
  } else {
   assert.equal(request.action,'close');await client.close(opts);
   const pids=async path=>(await readFile(path,'utf8').catch(()=>'' )).trim().split('\n').filter(Boolean).map(Number);
   const helpers=await pids(spec.pid_file),validators=await pids(spec.pid_file+'.validators');assert.equal(helpers.length,1);assert.equal(new Set([...helpers,...validators]).size,helpers.length+validators.length);
   for(const pid of [...helpers,...validators]) {assert.ok(Number.isSafeInteger(pid)&&pid>0);assert.throws(()=>process.kill(pid,0),error=>error.code==='ESRCH');}
   emit({phase:'closed',helpers_joined:helpers.length,validators_joined:validators.length});break;
  }
 }
} catch(error) {
 const at=/admin-consume\.mjs:(\d+):(\d+)/.exec(String(error?.stack));
 emit({phase:'failed',checkpoint:['ready','apply','read','read-refused','refuse','close'].includes(checkpoint)?checkpoint:'invalid',kind:error instanceof NodeSdkError?error.kind:'consumer-assertion',certainty:error instanceof NodeSdkError?error.certainty:null,observedKind,observedCertainty,line:at?Number(at[1]):null,column:at?Number(at[2]):null});
 process.exitCode=1;
} finally {await client.close(opts).catch(()=>{process.exitCode=1;});lines.close();}
