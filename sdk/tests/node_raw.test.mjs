import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),helper=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500};
const error=(kind,certainty,id)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);if(id)assert.equal(e.operation.operationId,id);return true;};
const lines=async(dir,name)=>(await readFile(join(dir,name),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
const get=()=>({v:2,id:'saved-attempt',ts:'2026-10-28T00:00:00Z',type:'operation.get',cid:'room',payload:{operation_id:'original'}});
const mutation=()=>({...get(),type:'conversation.close',operation_id:'original-close',payload:{reason:'done'}});
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-raw-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json'),executable=join(dir,'connect.mjs');
 await writeFile(path,JSON.stringify(config),{mode:0o600});
 await writeFile(executable,'#!/usr/bin/env node\nprocess.env.CONNECT_SDK_FIXTURE_CONFIG='+JSON.stringify(path)+';\nawait import('+JSON.stringify(pathToFileURL(helper).href)+');\n',{mode:0o700});
 const originalConfig={config:path,executable},client=new NodeClient(originalConfig);originalConfig.executable='/missing-after-capture';
 try{await run(client,dir);}finally{
  await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');
  for(const v of await lines(dir,'validation.jsonl'))assert.throws(()=>process.kill(v.pid,0),e=>e.code==='ESRCH');
  await rm(dir,{recursive:true,force:true});
 }
}
test('raw local validation captures immutable original intent without a helper request or configuration argument',async()=>{
 await fixture({},async(client,dir)=>{
  const value=get();value.payload.vendor={optional:'preserved'};const checked=client.raw.validate(value);
  value.cid='foreign';value.payload.vendor.optional='changed';
  assert.equal(checked.value.cid,'room');assert.equal(checked.value.payload.vendor.optional,'preserved');assert.throws(()=>checked.value.payload.vendor.optional='changed');
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
  const validations=await lines(dir,'validation.jsonl');assert.equal(validations.length,1);assert.deepEqual(validations[0].argv,['sdk-validate-envelope']);
  assert.throws(()=>client.raw.request({value:checked.value},opts),error('validation','not-submitted'));
  assert.throws(()=>client.raw.request(null,opts),error('validation','not-submitted'));
  const replies=await client.raw.request(checked,opts);assert.equal(replies.at(-1).value.payload.details.original,'preserved');assert.throws(()=>replies.push(checked));assert.throws(()=>replies[0].value.signature.alg='ES256');
  assert.throws(()=>client.raw.request(replies[0],opts),error('unsupported','read-only'));
  const calls=await lines(dir,'requests.jsonl');assert.equal(calls.length,1);assert.equal(calls[0].request.action.envelope.cid,'room');assert.deepEqual(calls[0].request.action.envelope.payload.vendor,{optional:'preserved'});
 });
});
test('raw full-schema invalid and local JSON bounds are refused before wire; finite-lifecycle policy comes from Rust descriptor',async()=>{
 await fixture({},async(client,dir)=>{
  for(const value of [null,{}, {...get(),payload:{operation_id:7}}, {...get(),payload:{operation_id:'x',oversize:'x'.repeat(1048576)}}])assert.throws(()=>client.raw.validate(value),error('validation','not-submitted'));
  let deep={};for(let i=0;i<34;i++)deep={nested:deep};assert.throws(()=>client.raw.validate({...get(),payload:{...get().payload,deep}}),error('validation','not-submitted'));
  const checked=client.raw.validate({...get(),type:'conversation.subscribe',payload:{after:'cursor',live:true}});assert.throws(()=>client.raw.request(checked,opts),error('unsupported','read-only'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
 });
});
test('raw preserves envelope payload and signed reply beyond typed 64 KiB bound without enlarging typed writes',async()=>{
 await fixture({rawSize:80000},async(client,dir)=>{
  const value=mutation();value.payload.vendor='x'.repeat(80000);const checked=client.raw.validate(value);
  const replies=await client.raw.request(checked,opts);assert.equal(replies[0].value.payload.details.vendor.length,80000);assert.equal(replies[0].value.operation_id,'original-close');
  assert.throws(()=>client.raw.request(replies[0],opts),error('unsupported','not-submitted','original-close'));
  assert.throws(()=>client.conversations.byId('room').send({messageId:'m',to:{first:'participant',rest:[]},text:'x'.repeat(80000)},{...opts,operationId:'typed-too-large'}),error('validation','not-submitted','typed-too-large'));
  assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
 for(const config of [{rawDepth:29},{rawNodes:99960}])await fixture(config,async(client)=>{
  const value=get();let vendor='leaf';for(let i=0;i<30;i++)vendor={nested:vendor};value.payload.vendor=vendor;
  const replies=await client.raw.request(client.raw.validate(value),opts);let result=replies[0].value.payload.details.vendor;
  if(config.rawDepth){for(let i=0;i<29;i++)result=result.nested;assert.equal(typeof result,'string');}
  else assert.equal(result.length,99960);
 });
});
test('malformed local validator profile, duplicate keys and descriptors fence the original owner before raw I/O',async()=>{
 for(const validator of ['profile','boolean','foreign-key','error-array','duplicate'])await fixture({validator},async(client,dir)=>{
  assert.throws(()=>client.raw.validate(mutation()),error('protocol','not-submitted'));
  assert.throws(()=>client.conversations.byId('room').refresh(opts),error('configuration','read-only'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
 });
});
test('raw malformed signed batch never escapes and preserves read versus submitted mutation certainty',async()=>{
 for(const mode of ['raw-correlation','raw-scope','raw-logical','raw-alg-array','raw-bad-signature','raw-error-frame','raw-no-ack','raw-duplicate','raw-after-ack','raw-empty','raw-wrong-descriptor'])await fixture({mode},async(client,dir)=>{
  const checked=client.raw.validate(mutation());await assert.rejects(client.raw.request(checked,opts),error('protocol','uncertain','original-close'));
  assert.throws(()=>client.conversations.byId('room').refresh(opts),error('configuration','read-only'));assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
 await fixture({mode:'raw-correlation'},async(client)=>await assert.rejects(client.raw.request(client.raw.validate(get()),opts),error('protocol','read-only')));
});
test('raw cancellation before submission writes nothing and active uncertainty retains original operation without retry',async()=>{
 for(const finish of ['deadline','cancelled'])await fixture({mode:'partial'},async(client,dir)=>{
  const checked=client.raw.validate(mutation());assert.throws(()=>client.raw.request(checked,{...opts,signal:AbortSignal.abort()}),error('cancelled','not-submitted','original-close'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
  // A write to the owned helper's pipe is already uncertain, even before
  // synthetic Node startup reaches its line reader. Observe the actual request
  // before awaiting its original deadline or cancelling; a cold50ms timer
  // cannot prove fixture-side receipt. Neither branch renews its request budget.
  const abort=new AbortController();let settled=false;
  const pending=client.raw.request(checked,{...opts,signal:abort.signal}).then(
   value=>{settled=true;return {value};},failure=>{settled=true;return {failure};});
  const until=performance.now()+opts.timeoutMs;
  try{
   while(!(await lines(dir,'requests.jsonl')).length){
    assert.equal(settled,false,'Original helper settled before its pending request was observed');
    assert.ok(performance.now()<until,'Original request observation exceeded its existing fixture budget');
    await new Promise(resolve=>setTimeout(resolve,5));
   }
   const calls=await lines(dir,'requests.jsonl');assert.equal(calls.length,1);
   assert.equal(calls[0].request.action.envelope.operation_id,'original-close');
   if(finish==='cancelled')abort.abort();
   const result=await pending;
   error(finish,'uncertain','original-close')(result.failure);
   assert.throws(()=>client.raw.request(checked,opts),error('configuration','not-submitted','original-close'));
   assert.equal((await lines(dir,'requests.jsonl')).length,1);
  }finally{abort.abort();await pending;}
 });
});
test('local validation subprocess output and time are bounded and original child is joined',async()=>{
 for(const validator of ['oversize','hang'])await fixture({validator},async(client)=>{
  const start=performance.now();assert.throws(()=>client.raw.validate(get()),error('configuration','not-submitted'));assert.ok(performance.now()-start<3000);
 });
});
test('raw reads keep timers responsive and capture intent through the original Rust owner without a validator child',async()=>{
 await fixture({validator:'hang',delayMs:100},async(client,dir)=>{
  const value=get();value.payload.vendor={optional:'preserved'};
  let ticked=false;const timer=setTimeout(()=>{ticked=true;},10);
  try{
   const pending=client.raw.read(value,opts);value.cid='foreign';value.payload.vendor.optional='changed';
   const replies=await pending;assert.equal(ticked,true);assert.equal(replies.at(-1).value.payload.details.original,'preserved');
   const calls=await lines(dir,'requests.jsonl');assert.equal(calls.length,1);assert.equal(calls[0].request.action.envelope.cid,'room');
   assert.deepEqual(calls[0].request.action.envelope.payload.vendor,{optional:'preserved'});
   assert.equal((await lines(dir,'validation.jsonl')).length,0);
   assert.throws(()=>replies.push(replies[0]));assert.throws(()=>replies[0].value.signature.alg='ES256');
  }finally{clearTimeout(timer);}
 });
});
test('raw read refuses mutations, replies, live subscriptions and ambiguous mutation scope before helper I/O',async()=>{
 await fixture({},async(client,dir)=>{
  for(const value of [mutation(),{...get(),type:'ack',payload:{}},{...get(),type:'conversation.subscribe',payload:{live:true}},
   {...get(),type:'conversation.subscribe',payload:{}},{...get(),operation_id:'unexpected'},null,{},
   {...get(),payload:{oversize:'x'.repeat(1048576)}}])await assert.rejects(client.raw.read(value,opts),error('validation','read-only'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);assert.equal((await lines(dir,'validation.jsonl')).length,0);
  await client.raw.read({...get(),type:'conversation.subscribe',payload:{live:false}},opts);
  assert.equal((await lines(dir,'requests.jsonl')).length,1);
 });
});
test('raw read cancellation and deadline preserve read-only certainty and never retry',async()=>{
 await fixture({mode:'partial'},async(client,dir)=>{
  await assert.rejects(client.raw.read(get(),{...opts,signal:AbortSignal.abort()}),error('cancelled','read-only'));
  assert.equal((await lines(dir,'requests.jsonl')).length,0);
  await assert.rejects(client.raw.read(get(),{timeoutMs:1000}),error('deadline','read-only'));
  assert.equal((await lines(dir,'requests.jsonl')).length,1);assert.equal((await lines(dir,'validation.jsonl')).length,0);
 });
 for(const mode of ['raw-correlation','raw-bad-signature','raw-empty'])await fixture({mode},async(client)=>{
  // Separate synthetic Node startup from the signed-response parser deadline.
  await client.capabilities.refresh({timeoutMs:5000});
  await assert.rejects(client.raw.read(get(),opts),error('protocol','read-only'));
 });
});
