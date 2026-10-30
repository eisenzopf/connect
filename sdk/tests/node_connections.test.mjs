import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500},mutation={...opts,operationId:'original'};
const reference=(command='connection.offer',scope={conversation_id:'room',session_id:'session',connection_id:'connection'})=>({schema:'connect-operation-reference/1',operation_id:'original',command,scope});
const offer=()=>({connectionId:'connection',streams:{first:{id:'audio',kind:'audio',direction:'sendrecv',codecs:{first:'g.711-mu',rest:[]}},rest:[]}});
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=(kind,certainty)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);return true;};
const session=client=>client.conversations.byId('room').sessions.byId('session');
test('historical ICE receipt recovery has no native control or media effect',async()=>{
 await fixture({iceReceipt:3},async(client,dir)=>{
  const op=client.operations.restore(reference('connection.update'),client.operationKinds.connection_update);
  const value=(await op.wait(opts)).value;
  assert.deepEqual(value.outcome,{kind:'ice-restart',iceGeneration:3});assert.equal(value.connection.id,'connection');assert.equal(value.connectionGeneration,1);
  assert.ok((await calls(dir)).every(r=>r.request.action.type==='operation'));
 });
});
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-connection-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});const client=new NodeClient({config:path,executable});
 try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
test('native control handles preserve full scopes, captured receipt revisions and separate current observations',async()=>{
 await fixture({clock:true},async(client,dir)=>{
  const s=session(client),input=offer(),submitted=s.connections.offer(input,mutation);input.streams.first.codecs.first='changed';
  const operation=await submitted,connection=(await operation.wait(opts)).value;assert.equal(connection.receiptStreamRevision,1);assert.throws(()=>connection.id='foreign');
  assert.equal((await connection.refresh(opts)).streamRevision,undefined);assert.equal(s.connections.byId(connection.id).receiptStreamRevision,undefined);
  const stream=connection.streams.byId('audio'),view=await stream.refresh(opts);assert.equal(view.ownership,'observation');assert.equal(view.resourceRevision,7);assert.equal(view.clock.unitsPerSecond,18446744073709551615n);assert.equal(view.clock.sourceId,'source:cname');
  const updates=[{action:'quality',intervalMs:250},{action:'mute',streams:{first:'audio',rest:[]}},{action:'unmute',streams:{first:'audio',rest:[]}},{action:'hold',direction:'inactive'},{action:'resume'},{action:'codec-renegotiate',codecPreferences:{first:'g.711-a',rest:[]}}];
  for(const [i,input]of updates.entries()){
   const receipt=(await (await connection.update(input,1,{...opts,operationId:'update-'+i})).wait(opts)).value;assert.equal(receipt.connection.id,connection.id);assert.equal(receipt.streamRevision,2);assert.ok(['quality','gates','codecs'].includes(receipt.outcome.kind));
  }
  assert.equal(connection.receiptStreamRevision,1);
  assert.equal((await (await connection.end({reasonCode:1000,reason:'done'},{...opts,operationId:'end'})).wait(opts)).value.id,'connection');
  assert.equal((await client.operations.restore(operation.referenceJson(),client.operationKinds.connection_offer).wait(opts)).value.id,'connection');
  const requests=await calls(dir);assert.equal(requests[0].request.action.action.streams[0].codecs[0],'g.711-mu');assert.ok(requests.filter(r=>r.request.action.type==='connection').every(r=>r.request.action.conversation_id==='room'&&r.request.action.session_id==='session'&&r.request.action.connection_id==='connection'));
 });
});
test('receive-selection bindings preserve unavailable versus known empty without granting local ownership',async()=>{
 for(const capture of ['unavailable','empty','present'])await fixture({capture},async client=>{
  const c=session(client).connections.byId('connection');
  const subscribing=await c.streams.subscribe({selectors:{first:{participant:'participant',kinds:{first:'audio',rest:[]}},rest:[]},expectedStreamRevision:1},mutation);
  const receipt=(await subscribing.wait(opts)).value;assert.equal(receipt.connectionGeneration,1);
  if(capture==='unavailable')assert.equal(receipt.bindings,null);else assert.equal(receipt.bindings.length,capture==='empty'?0:1);
  assert.equal((await c.streams.byId('audio').refresh(opts)).ownership,'observation');
  assert.equal((await (await c.streams.unsubscribe({streams:{first:'audio',rest:[]},expectedStreamRevision:2},{...opts,operationId:'unsubscribe'})).wait(opts)).value.connection.id,'connection');
 });
});
test('invalid native scope, stream identity, revisions and selector shapes never write helper input',async()=>{
 await fixture({},async(client,dir)=>{
  const s=session(client),c=s.connections.byId('connection');
  assert.throws(()=>s.connections.offer({...offer(),connectionId:'bad!'},mutation),error('validation','not-submitted'));
  assert.throws(()=>s.connections.offer({connectionId:'connection',streams:{first:{id:'bad!',kind:'audio',direction:'sendrecv',codecs:{first:'pcmu',rest:[]}},rest:[]}},mutation),error('validation','not-submitted'));
  assert.throws(()=>c.update({action:'quality',intervalMs:1.5},1,mutation),error('validation','not-submitted'));
  assert.throws(()=>c.end({reasonCode:65536,reason:'bad'},mutation),error('validation','not-submitted'));
  assert.throws(()=>c.streams.subscribe({selectors:{first:{stream:'audio',participant:'participant'},rest:[]},expectedStreamRevision:1},mutation),error('validation','not-submitted'));
  assert.throws(()=>c.streams.unsubscribe({streams:{first:'audio',rest:[]},expectedStreamRevision:-1},mutation),error('validation','not-submitted'));
  for(const scope of [{},{conversation_id:'room'},{conversation_id:'room',session_id:'session'}])assert.throws(()=>client.operations.restore(reference('connection.offer',scope),client.operationKinds.connection_offer),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('foreign native identities, malformed binding captures, generations and clock integers fail closed',async()=>{
 for(const mode of ['wrong-connection','missing-bindings','binding-generation','binding-local-handle'])await fixture({mode},async client=>{
  const kind=mode==='wrong-connection'?client.operationKinds.connection_update:client.operationKinds.stream_subscribe,op=client.operations.restore(reference(kind.command),kind);
  await assert.rejects(op.wait(opts),error('protocol','read-only'));assert.throws(()=>op.status(opts),error('configuration','read-only'));
 });
 await fixture({mode:'wrong-connection'},async client=>{await assert.rejects(session(client).connections.byId('connection').refresh(opts),error('protocol','read-only'));});
 await fixture({mode:'bad-clock',clock:true},async client=>{await assert.rejects(session(client).connections.byId('connection').streams.byId('audio').refresh(opts),error('protocol','read-only'));});
});

test('coercible stream gate state cannot become a typed native binding receipt',async()=>{
 await fixture({mode:'binding-state-array'},async(client,dir)=>{
  const c=session(client).connections.byId('connection');
  const operation=await c.update({action:'mute',streams:{first:'audio',rest:[]}},1,mutation);
  await assert.rejects(operation.wait(opts),error('protocol','read-only'));
  assert.throws(()=>c.refresh(opts),error('configuration','read-only'));
  assert.equal((await calls(dir)).length,2);
 });
});
