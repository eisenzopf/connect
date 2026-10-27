import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { canonicalJson, parseCanonicalJson } from '../typescript/dist/uctp_json.js';
import { signEnvelope, verifyEnvelope, base64url, unbase64url } from '../typescript/dist/uctp_signing.js';
import { envelope, encodeEnvelope, EnvelopeReader } from '../typescript/dist/uctp_wire.js';
import { BrowserClient } from '../typescript/dist/browser.js';

const bytes = s => new TextEncoder().encode(s);
const vector = JSON.parse(readFileSync(new URL('../../.deps/rvoip/crates/identity/auth-core/tests/fixtures/sig9421-cross-language-v1.json', import.meta.url)));
async function keys(alg='EdDSA') {
  const pair = await crypto.subtle.generateKey(alg==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'}, false, ['sign','verify']);
  return {signing:{key:pair.privateKey,keyId:'enrolled-test-key',algorithm:alg},server:{key:pair.publicKey,keyId:'enrolled-test-key',algorithm:alg,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300}};
}
test('JCS matches independent cross-language fixture and Ed25519 signature', async()=>{
  assert.equal(canonicalJson(vector.unsigned_envelope),vector.canonical_utf8);
  const key = await crypto.subtle.importKey('jwk',{kty:'OKP',crv:'Ed25519',d:base64url(Uint8Array.from(Buffer.from(vector.private_seed_hex,'hex'))),x:base64url(Uint8Array.from(Buffer.from(vector.public_key_hex,'hex')))}, {name:'Ed25519'},false,['sign']);
  const frame=await signEnvelope(vector.unsigned_envelope,{key,keyId:'public-rfc8032-fixture',algorithm:'EdDSA'});
  assert.equal(frame.signature.sig,vector.signature_base64url_no_pad);
});
test('strict JSON refuses erased duplicate keys, invalid Unicode, nonfinite values and trailing tokens',()=>{
  for(const raw of ['{"a":1,"a":2}','{"a":1,"\\u0061":2}','{"signature":{"sig":1,"sig":2}}','"\\ud800"','[1e400]','true false','[1,]','{"a":1,}','\ufeff{}','01','[+1]','{"__proto__":1,"__proto__":2}'])
    assert.throws(()=>parseCanonicalJson(bytes(raw)),{code:'protocol'},raw);
  assert.throws(()=>parseCanonicalJson(new Uint8Array([0xff])),{code:'protocol'});
  for(const value of [NaN,Infinity,undefined,new Date(),{a:undefined},'\udfff',new Array(2)]) assert.throws(()=>canonicalJson(value),{code:'protocol'});
  const value=parseCanonicalJson(bytes('{"__proto__":{"safe":true},"emoji":"😀","tiny":1e-8}'));
  assert.equal(Object.getPrototypeOf(value),null);
  assert.equal(canonicalJson(value),''+ '{"__proto__":{"safe":true},"emoji":"😀","tiny":1e-8}');
  assert.throws(()=>parseCanonicalJson(bytes('['.repeat(34)+'0'+']'.repeat(34))),{code:'protocol'});
});
test('framing accepts every split, coalesced frames and bounded UTF-8',()=>{
  const frame=envelope('operation.get',{operation_id:'durable_1'},{cid:'c1'}),wire=encodeEnvelope(frame);
  for(let split=0;split<=wire.length;split++){
    const decoder=new EnvelopeReader();const out=[...decoder.push(wire.subarray(0,split)),...decoder.push(wire.subarray(split))];
    assert.equal(out.length,1);assert.equal(out[0].id,frame.id);decoder.end();
  }
  const joined=new Uint8Array(wire.length*2);joined.set(wire);joined.set(wire,wire.length);
  assert.equal([...new EnvelopeReader().push(joined)].length,2);
  for(const n of [0,1048577,0xffffffff]){const header=new Uint8Array(4);new DataView(header.buffer).setUint32(0,n);assert.throws(()=>[...new EnvelopeReader().push(header)],{code:'protocol'});}
  const partial=new EnvelopeReader();[...partial.push(wire.subarray(0,8))];assert.throws(()=>partial.end(),{code:'protocol'});
});
for(const alg of ['EdDSA','ES256'])test(`${alg} validates enrolled server identity, full signed content, freshness and key lifetime`,async()=>{
  const {signing,server}=await keys(alg);const frame=await signEnvelope(envelope('ack',{status:'ok'}),signing);
  await verifyEnvelope(frame,server);
  for(const bad of [{...frame,payload:{status:'tampered'}},{...frame,signature:{...frame.signature,keyid:'other'}},{...frame,signature:{...frame.signature,alg:alg==='ES256'?'EdDSA':'ES256'}},{...frame,signature:{...frame.signature,sig:frame.signature.sig+'='}}])
    await assert.rejects(()=>verifyEnvelope(bad,server));
  await assert.rejects(()=>verifyEnvelope(frame,{...server,expiresAt:Date.now()/1000-1}),{code:'authentication'});
  await assert.rejects(()=>verifyEnvelope(frame,{...server,notBefore:Date.now()/1000+1}),{code:'authentication'});
  const stale=await signEnvelope({...frame,ts:new Date(Date.now()-61000).toISOString()},signing);
  await assert.rejects(()=>verifyEnvelope(stale,server),{code:'authentication'});
  assert.throws(()=>unbase64url('AB'),{code:'protocol'});
});

async function mockClient(run, {stall=false, tamper=false}={}) {
  const trust=await keys();const sent=[];let instance;
  const originals=['WebTransport','isSecureContext'].map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]);
  class Transport {
    ready=Promise.resolve();
    closed=new Promise(resolve=>{this.finish=resolve;});
    datagrams={readable:new ReadableStream(),writable:new WritableStream()};
    constructor(){instance=this;this.output=new ReadableStream({start:c=>this.controller=c});}
    async emit(type,payload,in_reply_to,extra={}){
      let frame=await signEnvelope({...envelope(type,payload,{in_reply_to}),...extra},trust.signing);
      if(tamper&&type==='auth.session')frame.payload.identity_id='forged';
      this.controller.enqueue(encodeEnvelope(frame));return frame;
    }
    async createBidirectionalStream(){
      const decoder=new EnvelopeReader();
      return {readable:this.output,writable:new WritableStream({write:async chunk=>{
        for(const frame of decoder.push(chunk)){
          sent.push(frame);
          if(frame.type!=='auth.hello')await verifyEnvelope(frame,trust.server);
          if(frame.type==='auth.hello')await this.emit('auth.challenge',{accepted_methods:['bearer'],server_capabilities:{application_profiles:['connect/2']}},frame.id);
          else if(frame.type==='auth.response')await this.emit('auth.session',{identity_id:'identity_test',participant_id:'participant_test',session_token:'test-session',expires_at:new Date(Date.now()+60000).toISOString(),application_profile:'connect/2'},frame.id);
          else if(!stall){
            if(frame.type==='conversation.create')await this.emit('ack',{details:{reply_complete:true,conversation:{id:'conversation_original',receipt:{operation_id:frame.operation_id,conversation_id:'conversation_original',state:'completed'}}}},frame.id);
            else if(frame.type==='operation.get')await this.emit('operation.updated',{operation_id:frame.payload.operation_id,state:'completed'},frame.id);
            if(frame.type!=='conversation.create')await this.emit('ack',{},frame.id);
          }
        }
      }})};
    }
    close(){this.didClose=true;this.finish();}
  }
  Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:Transport});
  Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:true});
  const options={url:'https://127.0.0.1:4433/uctp',credential:'fixture-only',identityId:'identity_test',device:{id:'device_test',kind:'browser',platform:'test'},signing:trust.signing,server:trust.server};
  try{await run({options,sent,transport:()=>instance,trust});}
  finally{instance?.close();for(const[k,d]of originals)if(d)Object.defineProperty(globalThis,k,d);else delete globalThis[k];}
}
test('canonical client authenticates and preserves logical create ID across fresh physical peers',()=>mockClient(async({options,sent})=>{
  const first=await BrowserClient.open(options);const a=await first.createConversation('stable-operation');first.close();
  const second=await BrowserClient.open(options);const b=await second.createConversation('stable-operation');
  assert.equal(a.id,b.id);assert.deepEqual(a.receipt,b.receipt);
  assert.equal((await second.operationStatus(a.id,'stable-operation')).state,'completed');
  const creates=sent.filter(f=>f.type==='conversation.create');assert.equal(creates.length,2);assert.equal(creates[0].operation_id,creates[1].operation_id);assert.notEqual(creates[0].id,creates[1].id);
  second.close();assert.equal(second.closed,true);
}));
test('request deadline and cancellation retire uncertain transport without retry',()=>mockClient(async({options,sent,transport})=>{
  const first=await BrowserClient.open(options);
  await assert.rejects(()=>first.createConversation('keep-this-id',{}, {timeoutMs:10}),{code:'timeout'});
  assert.equal(transport().didClose,true);assert.equal(sent.filter(f=>f.type==='conversation.create').length,1);
  const second=await BrowserClient.open(options);const controller=new AbortController();
  const result=second.operationStatus('c1','original-id',{signal:controller.signal});controller.abort();
  await assert.rejects(()=>result,{code:'cancelled'});assert.equal(second.closed,true);
},{stall:true}));
test('forged server session cannot authenticate',()=>mockClient(async({options})=>{
  await assert.rejects(()=>BrowserClient.open(options),{code:'authentication'});
},{tamper:true}));
test('server replay and unconsumed event overflow retire physical peer',()=>mockClient(async({options,transport,trust})=>{
  let client=await BrowserClient.open(options);
  const frame=await transport().emit('conversation.event',{sequence:1},undefined);
  transport().controller.enqueue(encodeEnvelope(frame));
  await new Promise(r=>setTimeout(r,20));assert.equal(client.closed,true);
  client=await BrowserClient.open(options);
  for(let i=0;i<257;i++)await transport().emit('conversation.event',{sequence:i},undefined);
  await new Promise(r=>setTimeout(r,20));assert.equal(client.closed,true);
}));

test('default actual WebTransport remains immutable when caller edits its construction options',()=>mockClient(async({options})=>{
  const client=await BrowserClient.open(options);options.transport='websocket';
  assert.equal(client.controlTransport,'webtransport');client.close();
  assert.equal(client.controlTransport,'webtransport');
}));
