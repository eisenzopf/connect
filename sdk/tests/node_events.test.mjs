import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdtemp,readFile,writeFile,mkdir,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
import {inspect} from 'node:util';
import {NodeClient,NodeSdkError,sdkCursor} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../..',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url)),opts={timeoutMs:1500};
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'' )).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=kind=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,'read-only');return true;};
async function fixture(config,run) {
 const base=join(root,'.local/gate-6/node-events-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});const client=new NodeClient({config:path,executable});
 try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>0));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
async function sent(dir){for(let i=0;i<200;i++){if((await calls(dir)).some(v=>v.request.action.type==='events_poll'))return;await sleep(5);}assert.fail('original poll not observed');}
const events=(client,input={},options=opts)=>client.conversations.byId('room').events(input,options);
const fact=async stream=>{const next=await stream.next();assert.equal(next.done,false);return next.value;};
test('events bootstrap captures options, exposes complete immutable scoped facts and acknowledges only returned facts',async()=>{
 await fixture({},async(client,dir)=>{
  const options={...opts},input={},stream=events(client,input,options);options.signal=AbortSignal.abort();options.timeoutMs=1;input.after=sdkCursor('foreign');
  assert.equal(await stream.ready(),'baseline-original');const first=await fact(stream);assert.equal(first.eventId,'event-1');assert.equal(first.cursor,'cursor-1');assert.equal(first.conversationId,'room');assert.equal(first.operationId,'original');assert.equal(first.data.data.record_id,'record-original');assert.equal(first.audience[0].membershipEpoch,2);assert.equal(first.source.correlationId,'original');
  assert.throws(()=>first.data.data.record_id='foreign');assert.throws(()=>first.audience.push(first.audience[0]));assert.throws(()=>first.source.kind='foreign');assert.ok(!inspect(first).includes('synthetic'));assert.ok(!inspect(first).includes('cursor-1'));assert.equal(JSON.parse(JSON.stringify(first)).data.private,'synthetic');
  const second=await fact(stream);assert.equal(second.eventId,'event-2');const written=(await calls(dir)).filter(v=>v.request.action.type==='events_poll').map(v=>v.request.action);assert.deepEqual(written.map(v=>v.acknowledged),[null,null,{event_id:'event-1',cursor:'cursor-1'}]);assert.ok(written.every(v=>v.after===null));await stream.close();
 });
});
test('events explicit opaque resume survives detach without granting scope or inventing a new cursor',async()=>{
 await fixture({},async(client,dir)=>{const resume=sdkCursor('persisted:opaque'),stream=events(client,{after:resume});assert.equal(await stream.ready(),resume);const first=await fact(stream);await stream.close();const fresh=events(client,{after:first.cursor});assert.equal(await fresh.ready(),first.cursor);await fresh.close();assert.equal((await calls(dir)).filter(v=>v.request.action.type==='events_poll').at(-1).request.action.after,'cursor-1');});
});
test('timed-out event wait keeps the SDK fact unacknowledged and revalidates before returning it',async()=>{
 await fixture({eventsDelayMs:200,eventsLaterDelayMs:0},async(client,dir)=>{const stream=events(client,{}, {timeoutMs:80});await assert.rejects(stream.next(),error('deadline'));await client.capabilities.refresh(opts);assert.equal((await fact(stream)).eventId,'event-1');const written=(await calls(dir)).filter(v=>v.request.action.type==='events_poll');assert.equal(written.length,2);assert.ok(written.every(v=>v.request.action.acknowledged===null));await stream.close();});
});
test('abandoned event reply cannot bypass current authentication after revocation',async()=>{
 await fixture({mode:'events-revoked-after-first',eventsDelayMs:200,eventsLaterDelayMs:0},async(client,dir)=>{const stream=events(client,{}, {timeoutMs:80});await assert.rejects(stream.next(),error('deadline'));await client.capabilities.refresh(opts);await assert.rejects(stream.next(),error('authentication'));assert.ok((await calls(dir)).filter(v=>v.request.action.type==='events_poll').every(v=>v.request.action.acknowledged===null));await stream.close();});
});
test('event cancellation and reader close join only the local reader and retain control',async()=>{
 await fixture({eventsDelayMs:100},async(client,dir)=>{const abort=new AbortController(),stream=events(client,{}, {...opts,signal:abort.signal}),next=stream.next();await sent(dir);abort.abort();await assert.rejects(next,error('cancelled'));await stream.close();assert.deepEqual(await stream.next(),{done:true,value:undefined});assert.equal((await client.capabilities.refresh(opts)).runtimeRole,'node');});
});
test('events invalid cursor, input, options and pre-cancellation do not write helper input',async()=>{
 await fixture({},async(client,dir)=>{for(const value of ['', 'bad cursor','x'.repeat(2049),'\n',null])assert.throws(()=>sdkCursor(value),error('validation'));for(const input of [null,[],{extra:1},{after:4}])assert.throws(()=>events(client,input),error('validation'));for(const options of [undefined,null,{}, {timeoutMs:0},{timeoutMs:100,signal:{}}])assert.throws(()=>client.conversations.byId('room').events({},options),error('validation'));const stream=events(client,{}, {...opts,signal:AbortSignal.abort()});await assert.rejects(stream.next(),error('cancelled'));await stream.close();assert.equal((await calls(dir)).length,0);});
});
test('malformed event scopes, opaque cursors, audience, resources and fields fence the original helper',async()=>{
 for(const mode of ['events-foreign','events-nested','events-resource','events-audience','events-cursor','events-extra','events-reader'])await fixture({mode},async(client,dir)=>{await assert.rejects(events(client).next(),error('protocol'));assert.equal((await calls(dir)).length,1);assert.throws(()=>client.capabilities.refresh(opts),error('configuration'));});
});
test('event source protocol and resync failures preserve unrelated control on the original helper',async()=>{
 for(const [mode,kind] of [['events-protocol','protocol'],['events-resync','resync-required']])await fixture({mode},async(client)=>{const stream=events(client);await assert.rejects(stream.next(),error(kind));await stream.close();assert.equal((await client.capabilities.refresh(opts)).runtimeRole,'node');});
});
test('event reply budget accepts large facts without widening ordinary outgoing input limits',async()=>{
 for(const config of [{eventSize:600000},{eventNodes:200000}])await fixture(config,async(client)=>{const stream=events(client);assert.ok((await fact(stream)).data.private.length>=200000);await stream.close();await client.capabilities.refresh(opts);});
});
test('event async iteration closes once on break or application failure and rejects concurrent reads',async()=>{
 for(const fail of [false,true])await fixture({eventsDelayMs:100},async(client,dir)=>{const stream=events(client),next=stream.next();await sent(dir);await assert.rejects(stream.ready(),error('conflict'));assert.equal((await next).done,false);const consume=async()=>{for await(const value of stream){assert.equal(value.eventId,'event-2');if(fail)throw new Error('application failure');break;}};if(fail)await assert.rejects(consume(),/application failure/);else await consume();await stream.close();assert.equal((await calls(dir)).filter(v=>v.request.action.type==='events_close').length,1);await client.capabilities.refresh(opts);});
});
test('events enforce the original eight-reader budget and reject duplicate returned facts',async()=>{
 await fixture({},async(client)=>{const readers=Array.from({length:9},()=>events(client));for(const stream of readers.slice(0,8))await stream.ready();await assert.rejects(readers[8].ready(),error('configuration'));await readers[0].close();await readers[8].ready();for(const stream of readers)await stream.close();});
 await fixture({mode:'events-duplicate'},async(client)=>{const stream=events(client);await fact(stream);await assert.rejects(stream.next(),error('protocol'));});
});
