import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserClient,browserTransportSupport} from '../typescript/dist/browser.js';
import {WebSocketControl} from '../typescript/dist/browser_control_transport.js';
import {canonicalJson,parseCanonicalJson} from '../typescript/dist/uctp_json.js';
import {signEnvelope,verifyEnvelope} from '../typescript/dist/uctp_signing.js';
import {envelope,encodeEnvelope,EnvelopeReader} from '../typescript/dist/uctp_wire.js';
const bytes = text => new TextEncoder().encode(text);
const wait = ms => new Promise(resolve=>setTimeout(resolve,ms));

import {fixture} from './fixtures/browser_control_peer.mjs';
for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} WSS authenticates signed canonical control without a WebTransport API`,()=>fixture(async({options,sent,strings})=>{
  assert.equal(browserTransportSupport().webTransport,false);assert.equal(browserTransportSupport().webSocket,true);
  const client=await BrowserClient.open(options);
  const conversation=await client.createConversation('original-create');
  assert.equal(conversation.receipt.operation_id,'original-create');assert.equal(conversation.id,'wss_conversation');
  assert.equal((await client.operationStatus(conversation.id,'original-create')).state,'completed');
  assert.equal(sent.filter(frame=>frame.type==='conversation.create').length,1);
  assert.ok(strings.every(text=>text.startsWith('{')));
  assert.throws(()=>client.datagrams,{code:'unsupported-transport'});client.close();assert.equal(client.closed,true);
},{algorithm}));
test('lost WSS mutation reply retains the original ID and fresh control only observes its receipt',()=>fixture(async({options,sent,receipts,socket})=>{
  const first=await BrowserClient.open(options);
  await assert.rejects(()=>first.createConversation('original-uncertain',{}, {timeoutMs:20}),{code:'timeout'});
  assert.equal(first.closed,true);assert.equal(socket().closeCount,1);assert.equal(receipts.get('original-uncertain').state,'completed');
  const second=await BrowserClient.open(options);
  assert.equal((await second.operationStatus('wss_conversation','original-uncertain')).operation_id,'original-uncertain');
  assert.equal(sent.filter(frame=>frame.type==='conversation.create').length,1);second.close();
},{lostCreateAck:true}));
test('WSS cancellation closes uncertain control without submitting a replacement',()=>fixture(async({options,sent})=>{
  const client=await BrowserClient.open(options);const abort=new AbortController();
  const pending=client.createConversation('cancelled-original',{}, {signal:abort.signal});abort.abort();
  await assert.rejects(()=>pending,{code:'cancelled'});assert.equal(client.closed,true);
  assert.ok(sent.filter(frame=>frame.type==='conversation.create').length<=1);
},{lostCreateAck:true}));
test('WSS rejects a forged signed server session',()=>fixture(async({options})=>{
  await assert.rejects(()=>BrowserClient.open(options),{code:'authentication'});
},{tamper:true}));
test('WSS cannot silently use insecure URLs or WebTransport-only certificate pins',()=>fixture(async({options,sent})=>{
  for(const bad of [{...options,url:'ws://127.0.0.1/connect'},{...options,serverCertificateHashes:[]}])await assert.rejects(()=>BrowserClient.open(bad),{code:'invalid-request'});
  assert.equal(sent.length,0);
}));
test('WSS server replay retires the physical client',()=>fixture(async({options,socket})=>{
  const client=await BrowserClient.open(options);const frame=await socket().emit('conversation.event',{sequence:1});socket().text(canonicalJson(frame));
  await wait(20);assert.equal(client.closed,true);
}));
test('WSS text framing preserves canonical UTF-8 and owns one control stream',()=>fixture(async({options,socket,strings})=>{
  const control=new WebSocketControl(options.url);const closed=control.closed.catch(error=>error);await control.ready;
  const stream=await control.stream();await assert.rejects(()=>control.stream(),{code:'conflict'});
  const frame=envelope('operation.get',{operation_id:'original',marker:'😀'},{cid:'conversation'}),writer=stream.writable.getWriter();
  await writer.write(encodeEnvelope(frame));assert.equal(strings[0],canonicalJson(frame));
  const reader=stream.readable.getReader();socket().text(canonicalJson(frame));const result=await reader.read();assert.equal([...new EnvelopeReader().push(result.value)][0].payload.marker,'😀');
  await reader.cancel();await closed;assert.equal(socket().closeCount,1);
},{raw:true}));
for(const kind of ['binary','empty','wide-utf8','receive-overflow'])test(`WSS refuses ${kind} with bounded text intake`,()=>fixture(async({options,socket})=>{
  const control=new WebSocketControl(options.url),closed=control.closed.catch(error=>error);await control.ready;const stream=await control.stream();
  if(kind==='binary')socket().text(new Uint8Array([1]));
  else if(kind==='empty')socket().text('');
  else if(kind==='wide-utf8')socket().text('😀'.repeat(524288));
  else for(let i=0;i<5;i++)socket().text('x'.repeat(1048576));
  assert.ok(['protocol','backpressure'].includes((await closed).code));await assert.rejects(()=>stream.readable.getReader().read());assert.equal(socket().closeCount,1);
},{raw:true}));
test('WSS bounds buffered writes before admitting another envelope',()=>fixture(async({options,socket,strings})=>{
  const control=new WebSocketControl(options.url),closed=control.closed.catch(error=>error);await control.ready;const stream=await control.stream();socket().bufferedAmount=1048576;
  await assert.rejects(()=>stream.writable.getWriter().write(encodeEnvelope(envelope('operation.get',{operation_id:'original'},{cid:'conversation'}))),{code:'backpressure'});
  assert.equal(strings.length,0);control.close();await closed;
},{raw:true}));

test('invalid WSS intake preserves the protocol refusal for an uncertain original',()=>fixture(async({options,sent,socket,receipts})=>{
  const client=await BrowserClient.open(options);const pending=client.createConversation('original-protocol-refusal');
  await wait(10);assert.equal(receipts.get('original-protocol-refusal').state,'completed');socket().text(new Uint8Array([1]));
  await assert.rejects(()=>pending,{code:'protocol'});assert.equal(client.closed,true);
  assert.equal(sent.filter(frame=>frame.type==='conversation.create').length,1);
},{lostCreateAck:true}));
test('WSS refuses invalid outbound UTF-8 before any socket write',()=>fixture(async({options,strings})=>{
  const control=new WebSocketControl(options.url),closed=control.closed.catch(error=>error);await control.ready;const stream=await control.stream();
  const malformed=new Uint8Array(5);new DataView(malformed.buffer).setUint32(0,1);malformed[4]=255;
  await assert.rejects(()=>stream.writable.getWriter().write(malformed),{code:'protocol'});assert.equal(strings.length,0);control.close();await closed;
},{raw:true}));

test('explicit WSS control never constructs an unusable WebTransport path',()=>fixture(async({options})=>{
  let attempts=0;Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:class {constructor(){attempts++;throw Error('UDP unavailable');}}});
  const client=await BrowserClient.open(options);assert.equal((await client.createConversation('wss-after-unusable-wt')).receipt.state,'completed');assert.equal(attempts,0);client.close();
}));

test('actual WSS transport remains immutable when caller edits its construction options',()=>fixture(async({options})=>{
  const client=await BrowserClient.open(options);options.transport='webtransport';
  assert.equal(client.controlTransport,'websocket');
  assert.throws(()=>client.datagrams,{code:'unsupported-transport'});
  client.close();assert.equal(client.controlTransport,'websocket');
}));

for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} pending open snapshots actor context before caller prepares the next peer`,()=>fixture(async({options,sent})=>{
  const identity=options.identityId,device=options.device.id,platform=options.device.platform;
  const pending=BrowserClient.open(options);
  options.identityId='next_peer_identity';options.device.id='next_peer_device';options.device.platform='next_platform';
  const client=await pending;
  assert.equal(client.identityId,identity);
  const hello=sent.find(frame=>frame.type==='auth.hello');assert.equal(hello.payload.device.id,device);assert.equal(hello.payload.device.platform,platform);
  const result=await client.createConversation('original-enrolled-actor');assert.equal(result.receipt.operation_id,'original-enrolled-actor');
  assert.equal(sent.filter(frame=>frame.type==='conversation.create').length,1);client.close();
},{algorithm}));
for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} physical peer retains enrolled server trust while caller prepares new trust`,()=>fixture(async({options,sent})=>{
  const client=await BrowserClient.open(options);
  // A separately configured next peer may use new trust. It cannot retroactively
  // expire the already authenticated peer's captured server-key policy.
  options.server.expiresAt=Date.now()/1000-1;options.server.keyId='next-server-key';
  const result=await client.createConversation('original-before-reconnect');
  assert.equal(result.receipt.operation_id,'original-before-reconnect');assert.equal(client.closed,false);
  assert.equal(sent.filter(frame=>frame.type==='conversation.create').length,1);client.close();
},{algorithm}));
test('getter-backed enrollment options retain the same snapshot contract as plain objects',()=>fixture(async({options,sent})=>{
  const inherited=Object.create(options);
  inherited.device=Object.create(options.device);inherited.signing=Object.create(options.signing);inherited.server=Object.create(options.server);
  const originalIdentity=options.identityId,originalDevice=options.device.id;
  const pending=BrowserClient.open(inherited);options.identityId='next_identity';options.device.id='next_device';options.server.keyId='next_server';
  const client=await pending;
  assert.equal(client.identityId,originalIdentity);assert.equal(sent.find(frame=>frame.type==='auth.hello').payload.device.id,originalDevice);
  assert.equal((await client.createConversation('original-inherited-enrollment')).receipt.state,'completed');client.close();
}));
