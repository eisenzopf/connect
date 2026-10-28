import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserTransportClient, BrowserClient, BrowserClientOpenError} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';

const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const deferred=()=>{let resolve;const promise=new Promise(yes=>{resolve=yes;});return {promise,resolve};};
function config(f,extra={}) {
  return {identityId:f.options.identityId,deviceId:f.options.device.id,
    endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},
    timeoutMs:500,attemptTimeoutMs:100,resolveEnrollment:async()=>f.options,...extra};
}
const reference=(extra={})=>({operationId:'original-intent',command:'session.handoff',scope:{conversationId:'room',sessionId:'session'},...extra});
const snapshot=(extra={})=>({cursor:'cursor-current',revision:4,items:[],has_more:false,...extra});
const status=(extra={})=>({operation_id:'original-intent',record_id:'record',command_type:'session.handoff',state:'unknown',revision:3,...extra});
async function respond(peer,frame,type,payload,scope=frame) {
  await peer.emit(type,payload,frame.id,scope);await peer.emit('ack',{},frame.id,frame);return true;
}

for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} owner selects authenticated preferred WT without claiming media readiness`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));
  assert.deepEqual(owner.status,{state:'connected',preferredTransport:'webtransport',selectedTransport:'webtransport',fallbackReason:undefined,physicalEpoch:1});
  assert.equal(f.wtCalls.length,1);assert.equal(f.sockets.length,0);assert.ok(owner.client instanceof BrowserClient);
  owner.close();assert.equal(f.transports[0].didClose,true);assert.equal(owner.status.state,'closed');
  assert.throws(()=>owner.client,{code:'closed'});
},{algorithm,wtMode:'ok'}));

for(const [mode,reason] of [[undefined,'preferred-unsupported'],['timeout','preferred-transport-timeout'],['transport-refused','preferred-transport-failed']])test(`owner falls back once for ${mode??'absent WT API'} and retires failed physical`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f,{attemptTimeoutMs:25}));
  assert.equal(owner.status.selectedTransport,'wss-webrtc');assert.equal(owner.status.fallbackReason,reason);
  assert.equal(f.sockets.length,1);assert.ok(f.transports.every(t=>t.didClose));owner.close();
},{wtMode:mode}));

test('owner reports unconfigured preferred contact and accepts explicit WSS preference',()=>fixture(async f=>{
  const first=await BrowserTransportClient.connect(config(f,{endpoints:{'wss-webrtc':'wss://fixture.invalid/connect'}}));
  assert.equal(first.status.fallbackReason,'preferred-not-configured');first.close();
  const next=await BrowserTransportClient.connect(config(f,{preferredTransport:'wss-webrtc'}));
  assert.equal(next.status.selectedTransport,'wss-webrtc');assert.equal(next.status.fallbackReason,undefined);next.close();
}));

for(const [mode,code] of [['security-refused','authentication'],['auth-refused','refused'],['auth-timeout','timeout']])test(`owner never falls back from ${mode}`,()=>fixture(async f=>{
  await assert.rejects(()=>BrowserTransportClient.connect(config(f,{attemptTimeoutMs:30})),error=>error instanceof BrowserClientOpenError&&error.code===code);
  assert.equal(f.wtCalls.length,1);assert.equal(f.sockets.length,0);assert.ok(f.transports.every(t=>t.didClose));
},{wtMode:mode}));

test('owner does not fall back from signed-session tampering',()=>fixture(async f=>{
  await assert.rejects(()=>BrowserTransportClient.connect(config(f)),{code:'authentication'});assert.equal(f.sockets.length,0);
},{wtMode:'ok',tamper:true}));

test('strict preference makes no alternate contact and two failed contacts remain bounded',()=>fixture(async f=>{
  await assert.rejects(()=>BrowserTransportClient.connect(config(f,{allowFallback:false})),{code:'transport'});assert.equal(f.sockets.length,0);
  await assert.rejects(()=>BrowserTransportClient.connect(config(f)),{code:'transport'});assert.equal(f.wtCalls.length,2);assert.equal(f.sockets.length,1);
},{wtMode:'transport-refused',wsMode:'transport-refused'}));

test('opening preserves the causal preferred failure when the alternate API is unavailable',()=>fixture(async f=>{
  Object.defineProperty(globalThis,'WebSocket',{configurable:true,value:undefined});
  await assert.rejects(()=>BrowserTransportClient.connect(config(f)),{code:'transport'});assert.equal(f.wtCalls.length,1);
},{wtMode:'transport-refused'}));

test('invalid contacts, invalid budgets and pre-abort do not resolve enrollment or open a socket',()=>fixture(async f=>{
  let resolutions=0;const base=config(f,{resolveEnrollment:async()=>{resolutions++;return f.options;}});
  for(const url of ['ws://fixture.invalid/connect','wss://user:pass@fixture.invalid/connect','wss://fixture.invalid/connect?q=1','wss://fixture.invalid/connect#x','wss://fixture.invalid:0/connect','wss://fixture.invalid'])
    await assert.rejects(()=>BrowserTransportClient.connect({...base,endpoints:{'wss-webrtc':url}}),{code:'invalid-request'});
  for(const timeoutMs of [0,Infinity,60001])await assert.rejects(()=>BrowserTransportClient.connect({...base,timeoutMs}),{code:'invalid-request'});
  const abort=new AbortController();abort.abort();await assert.rejects(()=>BrowserTransportClient.connect(base,{signal:abort.signal}),{code:'cancelled'});
  assert.equal(resolutions,0);assert.equal(f.sockets.length,0);
}));

test('insecure browser context refuses before resolving fresh credentials',()=>fixture(async f=>{
  Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:false});let resolved=false;
  await assert.rejects(()=>BrowserTransportClient.connect(config(f,{resolveEnrollment:async()=>{resolved=true;return f.options;}})),{code:'unsupported-transport'});
  assert.equal(resolved,false);assert.equal(f.sockets.length,0);
}));

test('one original deadline covers resolver work and both physical attempts',()=>fixture(async f=>{
  const start=performance.now();
  await assert.rejects(()=>BrowserTransportClient.connect(config(f,{timeoutMs:90,attemptTimeoutMs:45,resolveEnrollment:async()=>{
    await wait(25);return f.options;
  }})),{code:'timeout'});
  const elapsed=performance.now()-start;assert.ok(elapsed>=70&&elapsed<250,`bounded elapsed ${elapsed}`);
  assert.equal(f.wtCalls.length,1);assert.equal(f.sockets.length,1);assert.ok(f.transports.every(t=>t.didClose));assert.equal(f.sockets[0].readyState,3);
  // An attempt's timeout closes that physical; no extra round or resolver runs.
},{wtMode:'timeout',wsMode:'timeout'}));

test('expired or cancelled enrollment cannot adopt a late resolver result',()=>fixture(async f=>{
  for(const cancel of [false,true]){
    const late=deferred(),abort=new AbortController();let resolverSignal;
    const pending=BrowserTransportClient.connect(config(f,{timeoutMs:20,resolveEnrollment:({signal})=>{resolverSignal=signal;return late.promise;}}),{signal:abort.signal});
    if(cancel)abort.abort();await assert.rejects(()=>pending,{code:cancel?'cancelled':'timeout'});
    assert.equal(resolverSignal.aborted,true);late.resolve(f.options);await wait(5);
  }
  assert.equal(f.wtCalls.length,0);assert.equal(f.sockets.length,0);
},{wtMode:'ok'}));

test('low-level invalid budget and pre-abort do not allocate a physical transport',()=>fixture(async f=>{
  const abort=new AbortController();abort.abort();
  await assert.rejects(()=>BrowserClient.open({...f.options,timeoutMs:0}),{code:'invalid-request'});
  await assert.rejects(()=>BrowserClient.open({...f.options,signal:abort.signal}),{code:'cancelled'});
  assert.equal(f.sockets.length,0);
}));

test('reconnect resolves fresh credentials, retires media lifetime, and keeps enrolled actor/device',()=>fixture(async f=>{
  const rounds=[];let credential='fixture-only';
  const owner=await BrowserTransportClient.connect(config(f,{resolveEnrollment:async context=>{rounds.push(context);return {...f.options,credential};}}));
  const old=owner.client;let retired=0;old.onClose(()=>{retired++;});
  credential='fresh-bearer';f.setCredential(credential);
  await owner.reconnect();assert.equal(old.closed,true);assert.equal(retired,1);
  assert.deepEqual(rounds.map(c=>[c.reason,c.physicalEpoch]),[['initial',1],['reconnect',2]]);
  assert.equal(owner.status.physicalEpoch,2);assert.notEqual(owner.client,old);
  assert.equal(f.sent.filter(f=>f.type==='auth.response').at(-1).payload.credential,'fresh-bearer');
  assert.equal(f.sent.filter(f=>f.type==='conversation.create').length,0);owner.close();
}));

test('repeated reconnect reauthorizes after rejected stale credentials and changes path only explicitly',()=>fixture(async f=>{
  let credential='fixture-only',resolutions=0;
  const owner=await BrowserTransportClient.connect(config(f,{resolveEnrollment:async()=>{resolutions++;return {...f.options,credential};}}));
  f.setCredential('refreshed');const initial=owner.client;
  await assert.rejects(()=>owner.reconnect(),{code:'refused'});
  assert.equal(initial.closed,true);assert.equal(f.sockets.length,0);assert.equal(owner.status.state,'disconnected');
  credential='refreshed';await owner.reconnect({preferredTransport:'wss-webrtc'});
  assert.equal(owner.status.selectedTransport,'wss-webrtc');const websocket=owner.client;
  await owner.reconnect({preferredTransport:'webtransport'});assert.equal(websocket.closed,true);
  assert.equal(owner.status.selectedTransport,'webtransport');assert.equal(owner.status.physicalEpoch,4);assert.equal(resolutions,4);
  assert.equal(f.sent.filter(f=>!f.type.startsWith('auth.')).length,0);owner.close();
},{wtMode:'ok'}));

for(const changed of ['actor','device'])test(`reconnect refuses ${changed} replacement without opening a new physical`,()=>fixture(async f=>{
  let replace=false;const owner=await BrowserTransportClient.connect(config(f,{resolveEnrollment:async()=>!replace?f.options:{...f.options,...(changed==='actor'?{identityId:'other'}:{device:{...f.options.device,id:'other'}})}}));
  replace=true;const old=owner.client;await assert.rejects(()=>owner.reconnect(),{code:'authentication'});
  assert.equal(old.closed,true);assert.equal(owner.status.state,'disconnected');assert.equal(f.sockets.length,1);owner.close();
}));

test('physical outage clears selected path and does not automatically refresh or reconnect',()=>fixture(async f=>{
  let resolutions=0;const owner=await BrowserTransportClient.connect(config(f,{resolveEnrollment:async()=>{resolutions++;return f.options;}}));
  f.socket().close();await wait(0);assert.equal(owner.status.state,'disconnected');assert.equal(owner.status.selectedTransport,undefined);assert.equal(owner.status.fallbackReason,undefined);
  await wait(10);assert.equal(resolutions,1);assert.equal(f.sockets.length,1);assert.throws(()=>owner.client,{code:'transport'});
  await owner.reconnect();assert.equal(resolutions,2);owner.close();
}));

test('one reconnect round owns enrollment and close prevents late adoption',()=>fixture(async f=>{
  const late=deferred(),started=deferred();let calls=0;
  const owner=await BrowserTransportClient.connect(config(f,{resolveEnrollment:async()=>{if(++calls===1)return f.options;started.resolve();return late.promise;}}));
  const pending=owner.reconnect();await started.promise;
  await assert.rejects(()=>owner.reconnect(),{code:'conflict'});owner.close();await assert.rejects(()=>pending,{code:'cancelled'});
  late.resolve(f.options);await wait(5);assert.equal(owner.status.state,'closed');assert.equal(f.sockets.length,1);
}));

test('fallback captures one enrollment even when the caller edits its draft during the failed attempt',()=>fixture(async f=>{
  let calls=0;const ownerPromise=BrowserTransportClient.connect(config(f,{attemptTimeoutMs:30,resolveEnrollment:async()=>{calls++;return f.options;}}));
  while(f.wtCalls.length===0)await wait(1);
  f.options.identityId='different';f.options.device.id='different';f.options.server.keyId='different';f.options.signing.keyId='different';f.options.credential='different';
  const owner=await ownerPromise;assert.equal(calls,1);assert.equal(owner.status.selectedTransport,'wss-webrtc');
  assert.equal(f.sent.find(f=>f.type==='auth.hello').payload.device.id,'wss_device');owner.close();
},{wtMode:'timeout'}));

test('explicit development certificate pins are captured before the next fallback attempt',()=>fixture(async f=>{
  const pin=new Uint8Array(32).fill(7);f.options.serverCertificateHashes=[{algorithm:'sha-256',value:pin}];
  const owner=await BrowserTransportClient.connect(config(f));pin.fill(9);
  assert.deepEqual(new Uint8Array(f.wtCalls[0].config.serverCertificateHashes[0].value),new Uint8Array(32).fill(7));owner.close();
},{wtMode:'ok'}));

for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} dropped accepted ACK recovers the original operation without resubmitting it`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));
  await assert.rejects(()=>owner.client.createConversation('original-uncertain',{}, {timeoutMs:20}),{code:'timeout'});
  assert.equal(owner.status.state,'disconnected');await owner.reconnect();
  const recovered=await owner.recoverOperation({operationId:'original-uncertain',command:'create',scope:{conversationId:'wss_conversation'}});
  assert.equal(recovered.state,'completed');assert.equal(recovered.operation_id,'original-uncertain');
  assert.equal(f.sent.filter(f=>f.type==='conversation.create').length,1);assert.equal(f.sent.filter(f=>f.type==='operation.get').length,1);owner.close();
},{algorithm,lostCreateAck:true}));

for(const state of ['preparing','interrupted','unknown'])test(`recovery preserves ${state} scoped handoff without inventing completion or another mutation`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));
  const recovered=await owner.recoverOperation(reference());assert.equal(recovered.state,state);assert.equal(Object.isFrozen(recovered),true);
  const calls=f.sent.filter(f=>!f.type.startsWith('auth.'));assert.deepEqual(calls.map(f=>f.type),['operation.get']);
  assert.equal(calls[0].cid,'room');assert.equal(calls[0].sid,'session');assert.equal(calls[0].payload.operation_id,'original-intent');owner.close();
},{handler:({peer,frame})=>frame.type==='operation.get'?respond(peer,frame,'operation.updated',status({state})):false}));

for(const fault of ['kind','operation','conversation','session','revision','state'])test(`recovery refuses foreign or malformed ${fault} operation status`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await assert.rejects(()=>owner.recoverOperation(reference()),{code:'protocol'});
  assert.equal(owner.status.state,'disconnected');assert.equal(f.sent.filter(f=>f.type==='session.handoff').length,0);owner.close();
},{handler:({peer,frame})=>frame.type!=='operation.get'?false:respond(peer,frame,'operation.updated',status({...(fault==='kind'?{command_type:'session.consult'}:{}),...(fault==='operation'?{operation_id:'foreign'}:{}),...(fault==='revision'?{revision:-1}:{}),...(fault==='state'?{state:'maybe'}:{})}),{...frame,...(fault==='conversation'?{cid:'foreign'}:{}),...(fault==='session'?{sid:'foreign'}:{})})}));

test('current authorization refusal stays refused after reconnect and is never retried as a mutation',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await owner.reconnect();
  await assert.rejects(()=>owner.recoverOperation(reference()),error=>error.code==='refused'&&error.problem?.code===403);
  assert.equal(f.sent.filter(f=>f.type==='operation.get').length,1);assert.equal(owner.status.state,'connected');owner.close();
},{handler:({peer,frame})=>frame.type==='operation.get'?peer.emit('error',{code:403,category:'auth',reason:'forbidden'},frame.id,frame).then(()=>true):false}));

test('cursor recovery returns finite journal events and excludes advisory notifications',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));const result=await owner.recoverConversation({conversationId:'room',cursor:'saved-cursor',limit:2});
  assert.equal(result.kind,'events');assert.equal(result.cursor,'cursor-next');assert.equal(result.hasMore,true);
  assert.deepEqual(result.frames.map(f=>f.type),['conversation.event']);assert.equal(Object.isFrozen(result.frames),true);
  const subscribe=f.sent.find(f=>f.type==='conversation.subscribe');assert.deepEqual({...subscribe.payload},{after:'saved-cursor',live:false,limit:2});owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type!=='conversation.subscribe')return false;
  await peer.emit('participant.updated',{advisory:true},frame.id,frame);
  await peer.emit('conversation.event',{cursor:'cursor-event',sequence:1},frame.id,frame);
  await peer.emit('ack',{details:{resync_required:false,has_more:true,live:false,cursor:'cursor-next'}},frame.id,frame);return true;
}}));

for(const inline of [false,true])test(`explicit resync uses ${inline?'provided':'fresh'} current authorized snapshot`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));const result=await owner.recoverConversation({conversationId:'room',cursor:'old-cursor'});
  assert.equal(result.kind,'snapshot');assert.equal(result.cursor,'cursor-current');assert.equal(f.sent.filter(f=>f.type==='conversation.get').length,inline?0:1);owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type==='conversation.get')return respond(peer,frame,'conversation.snapshot',snapshot());
  if(frame.type!=='conversation.subscribe')return false;
  if(inline)await peer.emit('conversation.snapshot',snapshot(),frame.id,frame);
  await peer.emit('ack',{details:{resync_required:true,has_more:false,live:false}},frame.id,frame);return true;
}}));

test('expired snapshot page retries only its read once with fresh snapshot metadata',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));const result=await owner.recoverConversation({conversationId:'room',page:'expired-page',limit:3});
  assert.equal(result.kind,'snapshot');assert.equal(result.nextPage,'fresh-page');assert.equal(result.hasMore,true);
  assert.deepEqual(f.sent.filter(f=>f.type==='conversation.get').map(f=>({...f.payload})),[{page:'expired-page',limit:3},{limit:3}]);owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type!=='conversation.get')return false;
  if(frame.payload.page){await peer.emit('error',{code:409,category:'state',reason:'resync_required'},frame.id,frame);return true;}
  return respond(peer,frame,'conversation.snapshot',snapshot({has_more:true,next_page:'fresh-page'}));
}}));

for(const problem of [{code:403,category:'auth',reason:'forbidden'},{code:409,category:'state',reason:'revision_conflict'},{code:409,category:'auth',reason:'resync_required'}])test(`snapshot page does not retry ${problem.category}/${problem.reason}`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await assert.rejects(()=>owner.recoverConversation({conversationId:'room',page:'old-page'}),error=>error.code==='refused'&&error.problem?.reason===problem.reason);
  assert.equal(f.sent.filter(f=>f.type==='conversation.get').length,1);owner.close();
},{handler:({peer,frame})=>frame.type==='conversation.get'?peer.emit('error',problem,frame.id,frame).then(()=>true):false}));

test('snapshot resync read shares the original deadline rather than receiving another full budget',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));const start=performance.now();
  await assert.rejects(()=>owner.recoverConversation({conversationId:'room',page:'old-page'},{timeoutMs:70}),{code:'timeout'});
  const elapsed=performance.now()-start;assert.ok(elapsed>=55&&elapsed<180,`elapsed ${elapsed}`);assert.equal(f.sent.filter(f=>f.type==='conversation.get').length,2);owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type!=='conversation.get')return false;
  await wait(frame.payload.page?35:60);
  if(frame.payload.page)await peer.emit('error',{code:409,category:'state',reason:'resync_required'},frame.id,frame);
  else await respond(peer,frame,'conversation.snapshot',snapshot());return true;
}}));

for(const fault of ['foreign','over-limit','bad-cursor','bad-page'])test(`snapshot recovery refuses ${fault} authoritative data`,()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await assert.rejects(()=>owner.recoverConversation({conversationId:'room',limit:1}),{code:'protocol'});assert.equal(owner.status.state,'disconnected');owner.close();
},{handler:({peer,frame})=>frame.type!=='conversation.get'?false:respond(peer,frame,'conversation.snapshot',snapshot({...(fault==='over-limit'?{items:[{},{}]}:{}),...(fault==='bad-cursor'?{cursor:'bad cursor'}:{}),...(fault==='bad-page'?{has_more:false,next_page:'unexpected'}:{})}),{...frame,...(fault==='foreign'?{cid:'other'}:{})})}));

test('recovery rejects a foreign journal summary even when the event is correctly scoped',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await assert.rejects(()=>owner.recoverConversation({conversationId:'room',cursor:'saved-cursor'}),{code:'protocol'});owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type!=='conversation.subscribe')return false;
  await peer.emit('conversation.event',{cursor:'cursor-event'},frame.id,frame);
  await peer.emit('ack',{details:{resync_required:false,has_more:false,live:false,cursor:'cursor-next'}},frame.id,{cid:'foreign'});return true;
}}));

test('recovery refuses ambiguous multiple operation records instead of choosing the first',()=>fixture(async f=>{
  const owner=await BrowserTransportClient.connect(config(f));await assert.rejects(()=>owner.recoverOperation(reference()),{code:'protocol'});owner.close();
},{handler:async({peer,frame})=>{
  if(frame.type!=='operation.get')return false;
  await peer.emit('operation.updated',status(),frame.id,frame);
  return respond(peer,frame,'operation.updated',status({state:'completed'}));
}}));

test('reconnect retires a pending old status response instead of adopting it into the new epoch',()=>fixture(async f=>{
  const started=deferred(),release=deferred();let blocked=true;
  const owner=await BrowserTransportClient.connect(config(f));
  // The test peer remains real-signed but delays its old physical response.
  f.receipts.set('original-intent',status());
  const original=f.socket().emit.bind(f.socket());f.socket().emit=async(...args)=>{
    if(blocked&&args[0]==='operation.updated'){started.resolve();await release.promise;}return original(...args);
  };
  const old=owner.recoverOperation(reference({command:'create'}));const settled=old.catch(e=>e);await started.promise;
  await owner.reconnect();blocked=false;release.resolve();const result=await settled;
  assert.ok(result instanceof Error);assert.equal(owner.status.physicalEpoch,2);assert.equal(owner.status.state,'connected');owner.close();
}));
