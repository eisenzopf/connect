import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500};
const error=(kind,certainty)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);return true;};
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-receipt-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});const client=new NodeClient({config:path,executable});
 try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
test('receipt reads preserve independent recipient channel epoch and provenance evidence, including an empty result',async()=>{
 for(const receipts of [undefined,[]])await fixture({receipts},async(client,dir)=>{
  const facts=await client.conversations.byId('room').receipts('message',opts);
  assert.throws(()=>facts.push({}));
  if(receipts===undefined){assert.deepEqual(facts,[{participantId:'participant',membershipEpoch:1,channel:'native',deliveryId:'delivery',state:'delivered',provenance:'client_reported',observedAt:1}]);assert.throws(()=>facts[0].state='read');}
  else assert.deepEqual(facts,[]);
  const requests=await calls(dir);assert.equal(requests.length,1);assert.deepEqual(requests[0].request.action,{type:'message_receipts',conversation_id:'room',message_id:'message'});
 });
 await fixture({mode:'caps-no-receipts'},async(client)=>{
  assert.equal((await client.capabilities.refresh(opts)).features.has('message-receipts'),false);
  await assert.rejects(client.capabilities.require('message-receipts',opts),error('unsupported','read-only'));
  await assert.rejects(client.conversations.byId('room').receipts('message',opts),error('unsupported','read-only'));
 });
});
test('invalid receipt message identity and an already-cancelled read write no helper request',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room');
  for(const id of ['',null,'bad!'])assert.throws(()=>room.receipts(id,opts),error('validation','not-submitted'));
  assert.throws(()=>room.receipts('message',{...opts,signal:AbortSignal.abort()}),error('cancelled','read-only'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('foreign receipt scope, duplicate evidence, malformed facts and collection overflow fence the original helper',async()=>{
 for(const mode of ['receipt-cid','receipt-mid','receipt-duplicate','receipt-state-array','receipt-provenance','receipt-epoch','receipt-time','receipt-channel','receipt-extra','receipt-overflow'])await fixture({mode},async(client,dir)=>{
  const room=client.conversations.byId('room');await assert.rejects(room.receipts('message',opts),error('protocol','read-only'));
  assert.throws(()=>room.refresh(opts),error('configuration','read-only'));assert.equal((await calls(dir)).length,1);
 });
});
