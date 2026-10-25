import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserClient,browserTransportSupport} from '../typescript/dist/browser.js';
import {WebSocketControl} from '../typescript/dist/browser_control_transport.js';
import {canonicalJson,parseCanonicalJson} from '../typescript/dist/uctp_json.js';
import {signEnvelope,verifyEnvelope} from '../typescript/dist/uctp_signing.js';
import {envelope,encodeEnvelope,EnvelopeReader} from '../typescript/dist/uctp_wire.js';
const bytes = text => new TextEncoder().encode(text);
const wait = ms => new Promise(resolve=>setTimeout(resolve,ms));

// Canonical control boundary fixture, not a TLS, network or WebRTC qualification.
async function fixture(run,{algorithm='EdDSA',lostCreateAck=false,tamper=false,raw=false}={}) {
  const keyOptions=algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
  const clientPair=await crypto.subtle.generateKey(keyOptions,false,['sign','verify']);
  const serverPair=await crypto.subtle.generateKey(keyOptions,false,['sign','verify']);
  const signing={key:clientPair.privateKey,keyId:'wss-client-key',algorithm};
  const clientTrust={key:clientPair.publicKey,keyId:signing.keyId,algorithm,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300};
  const serverSigning={key:serverPair.privateKey,keyId:'wss-server-key',algorithm};
  const server={key:serverPair.publicKey,keyId:serverSigning.keyId,algorithm,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300};
  const sent=[],strings=[],tasks=[],receipts=new Map();let socket;
  const originals=['WebSocket','WebTransport','isSecureContext'].map(key=>[key,Object.getOwnPropertyDescriptor(globalThis,key)]);
  class Socket extends EventTarget {
    static CONNECTING=0;static OPEN=1;static CLOSED=3;
    readyState=0;bufferedAmount=0;closeCount=0;
    constructor(){super();socket=this;queueMicrotask(()=>{if(this.readyState!==0)return;this.readyState=1;this.dispatchEvent(new Event('open'));});}
    text(value){this.dispatchEvent(new MessageEvent('message',{data:value}));}
    async emit(type,payload,in_reply_to){
      const frame=await signEnvelope(envelope(type,payload,{in_reply_to}),serverSigning);
      if(tamper&&type==='auth.session')frame.payload.identity_id='forged';
      this.text(canonicalJson(frame));return frame;
    }
    send(text){
      assert.equal(typeof text,'string');strings.push(text);if(raw)return;
      const frame=parseCanonicalJson(bytes(text));sent.push(frame);
      tasks.push((async()=>{
        if(frame.type!=='auth.hello')await verifyEnvelope(frame,clientTrust);
        if(frame.type==='auth.hello')await this.emit('auth.challenge',{accepted_methods:['bearer'],server_capabilities:{application_profiles:['connect/2']}},frame.id);
        else if(frame.type==='auth.response')await this.emit('auth.session',{identity_id:'wss_identity',participant_id:'wss_participant',session_token:'fixture-session',expires_at:new Date(Date.now()+60000).toISOString(),application_profile:'connect/2'},frame.id);
        else if(frame.type==='conversation.create'){
          const receipt={operation_id:frame.operation_id,conversation_id:'wss_conversation',state:'completed'};receipts.set(frame.operation_id,receipt);
          if(!lostCreateAck)await this.emit('ack',{details:{reply_complete:true,conversation:{id:'wss_conversation',receipt}}},frame.id);
        }else{
          if(frame.type==='operation.get')await this.emit('operation.updated',receipts.get(frame.payload.operation_id)??{operation_id:frame.payload.operation_id,state:'unknown'},frame.id);
          await this.emit('ack',{},frame.id);
        }
      })());
    }
    close(){if(this.readyState===3)return;this.readyState=3;this.closeCount++;this.dispatchEvent(new Event('close'));}
  }
  Object.defineProperty(globalThis,'WebSocket',{configurable:true,value:Socket});
  Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:undefined});
  Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:true});
  const options={transport:'websocket',url:'wss://fixture.invalid/connect',credential:'fixture-only',identityId:'wss_identity',device:{id:'wss_device',kind:'browser',platform:'fixture'},signing,server,timeoutMs:500};
  try{await run({options,sent,strings,socket:()=>socket,receipts});await Promise.all(tasks);}
  finally{socket?.close();for(const[key,descriptor]of originals)if(descriptor)Object.defineProperty(globalThis,key,descriptor);else delete globalThis[key];}
}
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
