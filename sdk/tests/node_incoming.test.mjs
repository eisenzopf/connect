import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdtemp,readFile,writeFile,mkdir,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
import {NodeClient,NodeSdkError} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../..',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url)),opts={timeoutMs:1500};
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'' )).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=(kind)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,'read-only');return true;};
async function fixture(config,run) {
 const base=join(root,'.local/gate-6/node-incoming-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});const client=new NodeClient({config:path,executable});
 try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>0));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
async function sent(dir,type) {for(let i=0;i<200;i++){if((await calls(dir)).some(v=>v.request.action.type===type))return;await sleep(5);}assert.fail('original request not observed');}
async function hint(incoming) {const next=await incoming.next();assert.equal(next.done,false);return next.value;}
test('incoming metadata preserves original source and scoped Session without copied callee IDs or native ownership',async()=>{
 await fixture({incomingPendingPolls:2},async(client,dir)=>{
  const options={...opts},incoming=client.incomingInvitations(options);options.timeoutMs=1;options.signal=AbortSignal.abort();const received=await hint(incoming);
  assert.equal(received.operationId,'original-invite');assert.equal(received.source.correlationId,received.operationId);assert.equal(received.source.kind,'session.invite');assert.equal(received.session.id,'session');assert.equal(received.session.conversationId,'room');assert.equal(received.from,'caller');assert.equal(received.to,'callee');assert.equal(received.medium,'voice');assert.throws(()=>received.source.kind='foreign');assert.throws(()=>received.capabilities.audio.push('foreign'));
  assert.equal((await received.session.refresh(opts)).id,'session');assert.equal(received.nativeOwner,undefined);await incoming.close();assert.deepEqual(await incoming.next(),{done:true,value:undefined});assert.equal((await client.capabilities.refresh(opts)).features.has('incoming-invitations'),true);
  const written=await calls(dir);assert.equal(written.filter(v=>v.request.action.type==='incoming_poll').length,3);assert.equal(written.at(-2).request.action.acknowledged,'delivery-original');
 });
});
test('cancelled incoming wait keeps the original in-flight reply unacknowledged and leaves control usable',async()=>{
 await fixture({incomingDelayMs:150},async(client,dir)=>{
  const abort=new AbortController(),incoming=client.incomingInvitations({...opts,signal:abort.signal}),next=incoming.next();await sent(dir,'incoming_poll');abort.abort();await assert.rejects(next,error('cancelled'));
  await incoming.close();await client.capabilities.refresh(opts);const replacement=client.incomingInvitations(opts);assert.equal((await hint(replacement)).operationId,'original-invite');assert.equal((await calls(dir)).filter(v=>v.request.action.type==='incoming_poll').length,2);await replacement.close();
 });
});
test('timed-out incoming wait consumes no delivery and does not fence or restart the helper',async()=>{
 await fixture({incomingDelayMs:200,incomingLaterDelayMs:0},async(client,dir)=>{
  const incoming=client.incomingInvitations({timeoutMs:80});await assert.rejects(incoming.next(),error('deadline'));await client.capabilities.refresh(opts);assert.equal((await hint(incoming)).operationId,'original-invite');assert.equal((await calls(dir)).filter(v=>v.request.action.type==='incoming_poll').length,2);await incoming.close();
 });
});
test('incoming invalid options and pre-cancellation are local and an unopened reader closes without I/O',async()=>{
 await fixture({},async(client,dir)=>{
  for(const value of [undefined,null,{}, {timeoutMs:0},{timeoutMs:1.5},{timeoutMs:120001},{timeoutMs:100,signal:{}}])assert.throws(()=>client.incomingInvitations(value),error('validation'));
  const incoming=client.incomingInvitations({...opts,signal:AbortSignal.abort()});await assert.rejects(incoming.next(),error('cancelled'));await incoming.close();assert.equal((await calls(dir)).length,0);
 });
});
test('only one incoming reader owns the inbox and detach retains an unread delivery for a new reader',async()=>{
 await fixture({incomingDelayMs:100},async(client,dir)=>{
  const abort=new AbortController(),first=client.incomingInvitations({...opts,signal:abort.signal}),second=client.incomingInvitations(opts),next=first.next();await sent(dir,'incoming_poll');abort.abort();await assert.rejects(next,error('cancelled'));await assert.rejects(second.next(),error('conflict'));await first.close();assert.equal((await hint(second)).operationId,'original-invite');await second.close();
 });
});
test('closing an incoming reader cancels only its local wait and preserves native control',async()=>{
 await fixture({incomingDelayMs:100},async(client,dir)=>{
  const incoming=client.incomingInvitations(opts),next=incoming.next();await sent(dir,'incoming_poll');const rejected=assert.rejects(next,error('cancelled'));await incoming.close();await rejected;assert.equal((await client.capabilities.refresh(opts)).runtimeRole,'node');assert.deepEqual(await incoming.next(),{done:true,value:undefined});
 });
});
test('foreign or malformed invitation correlation, enums, participants and metadata fence the original helper',async()=>{
 for(const mode of ['incoming-source','incoming-medium','incoming-participants','incoming-intent','incoming-extra','incoming-reader','incoming-null'])await fixture({mode},async(client,dir)=>{
  const incoming=client.incomingInvitations(opts);await assert.rejects(incoming.next(),error('protocol'));assert.equal((await calls(dir)).filter(v=>v.request.action.type==='incoming_poll').length,1);
 });
});
test('an abandoned hint is revalidated and cannot bypass current authentication failure',async()=>{
 await fixture({mode:'incoming-revoked-after-first',incomingDelayMs:200,incomingLaterDelayMs:0},async(client,dir)=>{
  const incoming=client.incomingInvitations({timeoutMs:80});await assert.rejects(incoming.next(),error('deadline'));await client.capabilities.refresh(opts);await assert.rejects(incoming.next(),error('authentication'));const polls=(await calls(dir)).filter(v=>v.request.action.type==='incoming_poll');assert.equal(polls.length,2);assert.equal(polls[0].request.action.acknowledged,null);assert.equal(polls[1].request.action.acknowledged,null);await incoming.close();
 });
});
test('approved for-await iteration releases the reader on break and application failure without stopping control',async()=>{
 for(const fail of [false,true])await fixture({},async(client,dir)=>{
  const incoming=client.incomingInvitations(opts);assert.equal(incoming[Symbol.asyncIterator](),incoming);
  const consume=async()=>{for await(const received of incoming){assert.equal(received.from,'caller');if(fail)throw new Error('application failure');break;}};
  if(fail)await assert.rejects(consume(),/application failure/);else await consume();
  assert.deepEqual(await incoming.next(),{done:true,value:undefined});assert.equal((await calls(dir)).filter(v=>v.request.action.type==='incoming_close').length,1);await incoming.close();assert.equal((await client.capabilities.refresh(opts)).runtimeRole,'node');
 });
});
