import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserWebRtcReplacement} from '../typescript/dist/browser_webrtc_replacement.js';
import {BrowserDeadline} from '../typescript/dist/browser_deadline.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
// Signed control/custody models only; no network or decoded-media quality credit.
const auth={code:503,category:'authorization',reason:'authority_unavailable'};
const storage={code:503,category:'storage',reason:'storage_unavailable'};
const scope={cid:'room',sid:'session',connid:'source'};
const tick=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const requests=f=>f.sent.filter(frame=>frame.type==='operation.get');
const read=(client,options={},body={operation_id:'move'},saved=scope)=>client.requestReplacementContinuation('operation.get',body,saved,{timeoutMs:500,...options});
const refuse=(peer,frame,problem=auth)=>peer.emit('error',problem,frame.id,frame);
async function status(peer,frame,state='completed',overrides={}){
 await peer.emit('operation.updated',{operation_id:frame.payload.operation_id??frame.operation_id,record_id:'record',command_type:'connection.replace',state,revision:3,native_replacement_prepared:true,...overrides},frame.id,frame);
 await peer.emit('ack',{},frame.id,frame);
}
async function control(run,handler,{transport='websocket',algorithm='EdDSA'}={}){
 await fixture(async f=>{
  const client=await BrowserClient.open({...f.options,transport,url:transport==='webtransport'?'https://fixture.invalid/connect':f.options.url});
  try{await run({client,f});}finally{await client.shutdown();}
 },{algorithm,wtMode:transport==='webtransport'?'ok':undefined,handler});
}
for(const transport of ['websocket','webtransport'])for(const algorithm of ['EdDSA','ES256'])for(const problem of [auth,storage])
 test(`R01 ${algorithm} ${transport} exact ${problem.reason} retries only same original signed read`,async()=>{
 let count=0;
 await control(async({client,f})=>{
  const result=await read(client);assert.equal(result[0].payload.state,'completed');
  const sent=requests(f);assert.equal(sent.length,2);assert.equal(new Set(sent.map(frame=>frame.id)).size,2);
  assert.ok(sent.every(frame=>frame.payload.operation_id==='move'&&frame.cid==='room'&&frame.sid==='session'&&frame.connid==='source'&&frame.signature));
  assert.equal(f.sent.filter(frame=>frame.type==='connection.replace'||frame.type==='connection.offer').length,0);
  assert.equal(f.sent.filter(frame=>frame.type==='auth.response').length,1);assert.equal(f.sockets.length+f.transports.length,1);assert.equal(client.closed,false);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;if(++count===1)await refuse(peer,frame,problem);else await status(peer,frame);return true;},{transport,algorithm});
});

test('R03 low allocation retries status before its sole original submission',async()=>{
 let gets=0,offers=0,attempted=0;
 await control(async({client,f})=>{
  const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'source',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'wss_device'};
  const owner=new BrowserWebRtcReplacement(client,root),budget=new BrowserDeadline(500);
  await owner.allocate({by_participant:'participant',substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp:'v=0\r\n'},capabilities:{},streams_offered:[]},
    {cid:'room',sid:'session',connid:'target',operation_id:'move',expected_revision:0},{timeoutMs:budget.remaining()},()=>attempted++,budget);
  assert.equal(gets,2);assert.equal(offers,1);assert.equal(attempted,1);
  assert.equal(f.sent.filter(frame=>frame.type==='connection.replace').length,0);
 },async({peer,frame})=>{
  if(frame.type==='operation.get'){assert.equal(attempted,0);assert.equal(offers,0);if(++gets===1)await refuse(peer,frame);else await status(peer,frame,'preparing');return true;}
  if(frame.type==='connection.offer'){offers++;assert.equal(attempted,1);assert.equal(frame.payload.replacement_preparation.profile,'connect-replacement-preparation-v1');await status(peer,frame,'preparing');return true;}
  return false;
 });
});

for(const code of [401,403])test(`R04 current signed${code} after503 stops fresh read recovery`,async()=>{
 let count=0;
 await control(async({client,f})=>{
  await assert.rejects(read(client),error=>error.code==='refused'&&error.problem?.code===code);
  assert.equal(requests(f).length,2);assert.equal(client.closed,false);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame,++count===1?auth:{code,category:'auth',reason:'forbidden'});return true;});
});

test('R04 actual credential withdrawal is rechecked without reenrollment',async()=>{
 let withdraw;
 await control(async({client,f})=>{
  withdraw=()=>f.setCredential('withdrawn');
  await assert.rejects(read(client),error=>error.code==='refused'&&error.problem?.code===401);
  assert.equal(requests(f).length,2);assert.equal(f.sent.filter(frame=>frame.type==='auth.response').length,1);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;withdraw();await refuse(peer,frame);return true;});
});

for(const type of ['connection.offer','connection.replace'])test(`R05 signed503 never retries ${type}`,async()=>{
 let count=0;
 await control(async({client,f})=>{
  const action=type==='connection.offer'
   ? client.requestReplacementContinuation(type,{replacement_preparation:{profile:'connect-replacement-ready-v1',operation_record_id:'record',readiness:{nonce:'nonce'}}},{...scope,connid:'target',operation_id:'move',expected_revision:0},{timeoutMs:200})
   : client.request(type,{},{...scope,operation_id:'move'},{timeoutMs:200});
  await assert.rejects(action,error=>error.code==='refused'&&error.problem?.code===503);
  assert.equal(count,1);assert.equal(f.sent.filter(frame=>frame.type===type).length,1);assert.equal(requests(f).length,0);
 },async({peer,frame})=>{if(frame.type!==type)return false;count++;await refuse(peer,frame);return true;});
});

for(const problem of [
 {code:401,category:'auth',reason:'authentication_failed'},
 {code:403,category:'auth',reason:'forbidden'},
 {code:409,category:'state',reason:'reconciliation_required'},
 {code:429,category:'capacity',reason:'capacity_exceeded'},
 {code:500,category:'storage',reason:'storage_unavailable'},
 {code:502,category:'authorization',reason:'authority_unavailable'},
 {code:504,category:'storage',reason:'storage_unavailable'},
 {code:503,category:'storage',reason:'other_unavailable'},
 {code:503,category:'other',reason:'authority_unavailable'},
])test(`N01 exact code/category/reason required: ${problem.code}/${problem.category}/${problem.reason}`,async()=>{
 await control(async({client,f})=>{
  await assert.rejects(read(client),error=>error.code==='refused'&&JSON.stringify(error.problem)===JSON.stringify(problem));
  assert.equal(requests(f).length,1);assert.equal(client.closed,false);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame,problem);return true;});
});

test('N02 missing public problem fields do not select retries by private text',async()=>{
 await control(async({client,f})=>{
  await assert.rejects(read(client),error=>error.code==='refused'&&error.problem===undefined);
  assert.equal(requests(f).length,1);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame,{code:503,reason:'authority_unavailable',message:'storage_unavailable'});return true;});
});

test('N02 unsigned unavailable response is rejected without retry',async()=>{
 await control(async({client,f})=>{
  await assert.rejects(read(client));assert.equal(requests(f).length,1);assert.equal(client.closed,true);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;peer.text(JSON.stringify(envelope('error',auth,{...scope,in_reply_to:frame.id})));return true;});
});

test('N03 ordinary operation.get remains one-shot outside original continuation',async()=>{
 await control(async({client,f})=>{
  await assert.rejects(client.request('operation.get',{operation_id:'move'},scope,{timeoutMs:200}),error=>error.problem?.code===503);
  assert.equal(requests(f).length,1);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame);return true;});
});

test('B01 repeated503 cannot renew original budget or retire physical media',async()=>{
 await control(async({client,f})=>{
  const abort=new AbortController(),watchdog=setTimeout(()=>abort.abort(),1200);
  try{await assert.rejects(read(client,{timeoutMs:160,signal:abort.signal}),{code:'timeout'});}finally{clearTimeout(watchdog);}
  assert.ok(requests(f).length>=2);assert.equal(abort.signal.aborted,false);assert.equal(client.closed,false);
  assert.equal(f.sent.filter(frame=>frame.type==='connection.offer'||frame.type==='connection.end').length,0);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame);return true;});
});

test('B02 already-aborted read sends nothing',async()=>{
 await control(async({client,f})=>{
  const abort=new AbortController();abort.abort();await assert.rejects(read(client,{signal:abort.signal}),{code:'cancelled'});
  assert.equal(requests(f).length,0);assert.equal(client.closed,false);
 },async()=>false);
});

test('B02 cancellation after503 prevents every later read and keeps physical open',async()=>{
 let seen;const first=new Promise(resolve=>seen=resolve);
 await control(async({client,f})=>{
  const abort=new AbortController(),pending=read(client,{signal:abort.signal});void pending.catch(()=>{});
  await first;abort.abort();await assert.rejects(pending,{code:'cancelled'});await tick(40);
  assert.equal(requests(f).length,1);assert.equal(client.closed,false);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame);seen();return true;});
});

for(const when of ['before','after503'])test(`B03 closed physical ${when} permits no later read`,async()=>{
 let seen;const first=new Promise(resolve=>seen=resolve);
 await control(async({client,f})=>{
  const failure=new ConnectError('transport','original physical ended');
  if(when==='before')client.close(failure);
  const pending=read(client);void pending.catch(()=>{});
  if(when==='after503'){await first;client.close(failure);}
  await assert.rejects(pending,{code:'transport'});await tick(40);assert.equal(requests(f).length,when==='before'?0:1);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;await refuse(peer,frame);seen();return true;});
});

test('B04 mutation of caller arguments cannot change original logical read',async()=>{
 let mutate,count=0;
 await control(async({client,f})=>{
  const payload={operation_id:'move'},saved={...scope},options={timeoutMs:500};
  mutate=()=>{payload.operation_id='foreign';saved.connid='foreign';options.timeoutMs=60000;};
  await read(client,options,payload,saved);
  assert.ok(requests(f).every(frame=>frame.payload.operation_id==='move'&&frame.connid==='source'));
  assert.equal(requests(f).length,2);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;if(++count===1){mutate();await refuse(peer,frame);}else await status(peer,frame);return true;});
});

test('S01 unavailable read followed by current failed state never fabricates completion',async()=>{
 let count=0;
 await control(async({client,f})=>{
  const result=await read(client);assert.equal(result[0].payload.state,'failed');assert.equal(requests(f).length,2);
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;if(++count===1)await refuse(peer,frame);else await status(peer,frame,'failed');return true;});
});

test('C01 timeout and late signed reply remain tied to original request IDs',async()=>{
 let count=0,late;
 await control(async({client,f})=>{
  let events=0;const stop=client.onEvent(()=>events++);
  try{
   await assert.rejects(read(client,{timeoutMs:100}),{code:'timeout'});assert.ok(late);assert.equal(client.closed,false);
   await status(late.peer,late.frame,'preparing');await tick(0);assert.equal(events,0);
   const current=await read(client);assert.equal(current[0].payload.state,'completed');assert.equal(client.closed,false);
   assert.equal(requests(f).length,3);assert.equal(new Set(requests(f).map(frame=>frame.id)).size,3);
  }finally{stop();}
 },async({peer,frame})=>{if(frame.type!=='operation.get')return false;if(++count===1)await refuse(peer,frame);else if(count===2)late={peer,frame};else await status(peer,frame);return true;});
});
