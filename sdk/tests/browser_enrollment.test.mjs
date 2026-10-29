import assert from 'node:assert/strict';
import { test } from 'node:test';
import { BrowserEnrollmentKey, BrowserEnrollmentSession } from '../typescript/dist/browser_enrollment.js';
import { BrowserEnrollmentStore } from '../typescript/dist/browser_enrollment_store.js';
import { enrollmentProofMessage, normalizeOffer } from '../typescript/dist/browser_enrollment_contract.js';
const ui='https://app.fixture.invalid';
function request(public_key='ERERERERERERERERERERERERERERERERERERERERERE',algorithm='EdDSA') {
 return {operation_id:'approve-one',completion_operation_id:'claim-one',identity_id:'identity-one',device:{id:'device-one',kind:'browser',platform:'synthetic'},key_id:'key-one',algorithm,public_key,transport:'wss-webrtc',endpoint_address:'https://endpoint.fixture.invalid/device',capabilities:['video','audio'],browser_origin:ui,operations:['send','operation.get'],recipients:['peer-b','peer-a'],credential_expires_at:2000003600,challenge_expires_at:2000000300};
}
function offer(r=request()){return {profile:'connect-enrollment/1',enrollment_id:'enrollment-one',endpoint_id:'endpoint-one',installation_id:'installation-one',issuer_identity_id:'issuer-one',request:r,challenge:'a'.repeat(64)};}
async function browser(work) {
 const saved=new Map(['isSecureContext','location','fetch'].map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]));
 Object.defineProperty(globalThis,'isSecureContext',{value:true,configurable:true});Object.defineProperty(globalThis,'location',{value:{origin:ui},configurable:true});
 try{return await work();}finally{for(const [k,d]of saved)d?Object.defineProperty(globalThis,k,d):delete globalThis[k];}
}
async function session(algorithm='EdDSA') {
 const key=await BrowserEnrollmentKey.generate(algorithm),r=request(key.publicKey,algorithm),now=Math.floor(Date.now()/1000);r.credential_expires_at=now+300;r.challenge_expires_at=now+120;
 const state={version:1,origin:'https://service.fixture.invalid',request:structuredClone(r),key:key.snapshot(),offer:offer(r)};
 return {state,key,session:await BrowserEnrollmentSession.restore(state)};
}
function credential(s) {const o=s.offer,r=o.request;return {enrollment_id:o.enrollment_id,endpoint_id:o.endpoint_id,device_id:r.device.id,key_id:r.key_id,transport:r.transport,browser_origin:r.browser_origin,credential:{operation_id:r.completion_operation_id,credential_id:'11111111-1111-4111-8111-111111111111',identity_id:r.identity_id,issuer_identity_id:o.issuer_identity_id,issuer_credential_id:'22222222-2222-4222-8222-222222222222',installation_id:o.installation_id,operations:r.operations,recipients:r.recipients,issued_at:Math.floor(Date.now()/1000),expires_at:r.credential_expires_at,credential:'bc'.repeat(32)}};}
const response=(value,status=200)=>new Response(JSON.stringify({profile:'connect-enrollment/1',result:{Ok:value}}),{status,headers:{'Content-Type':'application/json'}});
test('browser enrollment JCS context matches independent Rust/Python literal and stable normalization',async()=>{
 const expected='636f6e6e6563742d656e726f6c6c6d656e742d70726f6f662f31000b160e9aac009069e854244c3852c5b97b39db1050ace720eb7d54ecb79ee702';
 assert.equal(Buffer.from(await enrollmentProofMessage(offer())).toString('hex'),expected);
 const changed=offer();changed.request.capabilities.push('audio');changed.request.operations.reverse();changed.request.recipients.push('peer-a');changed.request.endpoint_address='https://ENDPOINT.fixture.invalid/device';
 assert.equal(Buffer.from(await enrollmentProofMessage(changed)).toString('hex'),expected);
 for(const field of ['challenge','issuer_identity_id','endpoint_id','installation_id']) {const other=offer();other[field]=field==='challenge'?'b'.repeat(64):'other';assert.notEqual(Buffer.from(await enrollmentProofMessage(other)).toString('hex'),expected);}
 for(const operation of ['admin','credential.delegate','*','send *']) {const o=offer();o.request.operations=[operation];assert.throws(()=>normalizeOffer(o));}
});
for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} nonextractable browser key roundtrips structured clone and proves the original offer`,()=>browser(async()=>{
 const {key,state}=await session(algorithm);assert.equal(key.snapshot().privateKey.extractable,false);await assert.rejects(()=>crypto.subtle.exportKey('pkcs8',key.snapshot().privateKey));
 const restored=await BrowserEnrollmentKey.restore(structuredClone(key.snapshot()));assert.equal(restored.publicKey,key.publicKey);assert.equal(JSON.stringify(key),'{}');
 const proof=await restored.claim(state.offer);assert.equal(proof.operation_id,state.request.completion_operation_id);
 const params=algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',hash:'SHA-256'};
 assert.equal(await crypto.subtle.verify(params,key.snapshot().publicKey,Buffer.from(proof.signature,'base64url'),await enrollmentProofMessage(state.offer)),true);
 const other=await BrowserEnrollmentKey.generate(algorithm);await assert.rejects(()=>BrowserEnrollmentKey.restore({...key.snapshot(),publicKey:other.snapshot().publicKey}));
}));
test('browser restoration refuses broadened intent, other Origin/key/profile and JSON-erased handles',()=>browser(async()=>{
 const {state}=await session();
 for(const change of [v=>v.offer.request.operations.push('extra'),v=>v.request.browser_origin='https://other.fixture.invalid',v=>v.offer.request.transport='quic',v=>v.key=JSON.parse(JSON.stringify(v.key))]) {const copy=structuredClone(state);change(copy);await assert.rejects(()=>BrowserEnrollmentSession.restore(copy));}
 Object.defineProperty(globalThis,'isSecureContext',{value:false,configurable:true});await assert.rejects(()=>BrowserEnrollmentKey.generate('EdDSA'),{code:'unsupported-transport'});
 assert.throws(()=>new BrowserEnrollmentStore(),{code:'unsupported-transport'});
}));
test('browser claim sends no bearer/cookies/explicit Origin and validates every protected response binding',()=>browser(async()=>{
 const {session:s}=await session();const good=credential(s);let calls=0;
 globalThis.fetch=async(url,init)=>{calls++;assert.equal(url,'https://service.fixture.invalid/v2/enrollments/enrollment-one/claim');assert.equal(init.credentials,'omit');assert.equal(init.redirect,'error');assert.equal(init.cache,'no-store');assert.equal(init.referrerPolicy,'no-referrer');assert.deepEqual(Object.keys(init.headers),['Content-Type']);const claim=JSON.parse(init.body);assert.equal(claim.operation_id,'claim-one');assert.equal(claim.browser_origin,ui);return response(good);};
 assert.deepEqual(await s.claim(),good);assert.equal(calls,1);assert.equal(JSON.stringify(s),'{}');
 for(const mutate of [v=>v.enrollment_id='other',v=>v.endpoint_id='other',v=>v.device_id='other',v=>v.key_id='other',v=>v.transport='webtransport',v=>v.browser_origin='https://other.fixture.invalid',v=>v.credential.operation_id='other',v=>v.credential.identity_id='other',v=>v.credential.issuer_identity_id='other',v=>v.credential.installation_id='other',v=>v.credential.operations.push('extra'),v=>v.credential.recipients.push('other'),v=>v.credential.expires_at++,v=>v.credential.issued_at=Date.now(),v=>v.credential.credential='wrong']) {
  const value=structuredClone(good);mutate(value);globalThis.fetch=async()=>response(value);await assert.rejects(()=>s.claim(),{code:'protocol'});
 }
}));
test('browser claim bounds streaming, rejects malformed/duplicate/redirect responses and sanitizes refusals',()=>browser(async()=>{
 const {session:s}=await session();
 const failures=[()=>new Response('x',{headers:{'Content-Length':'65537'}}),()=>new Response('x'.repeat(65537)),()=>new Response('{"profile":"connect-enrollment/1","profile":"other","result":{}}'),()=>new Response('{"profile":"connect-enrollment/1","result":{},"secret":"remote-secret"}'),()=>new Response('',{status:307})];
 for(const make of failures){let calls=0;globalThis.fetch=async()=>{calls++;return make();};await assert.rejects(()=>s.claim(),{code:'protocol'});assert.equal(calls,1);}
 globalThis.fetch=async()=>new Response(JSON.stringify({profile:'connect-enrollment/1',result:{Err:{code:'forbidden',message:'remote-secret'}}}),{status:403});
 await assert.rejects(()=>s.claim(),e=>e.code==='forbidden'&&!e.message.includes('remote-secret')&&e.problem===undefined);
}));
test('browser claim cancellation/deadline covers stalled fetch and partial body without replacement intent',()=>browser(async()=>{
 const {session:s}=await session();let calls=0;const cancelled=new AbortController();cancelled.abort();globalThis.fetch=async()=>{calls++;throw Error('must not run');};await assert.rejects(()=>s.claim({signal:cancelled.signal}),{code:'cancelled'});assert.equal(calls,0);
 for(const body of [false,true])for(const cancel of [false,true]){
  const abort=new AbortController();let started;const ready=new Promise(r=>started=r);globalThis.fetch=async()=>{calls++;started();if(!body)return new Promise(()=>{});return new Response(new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('{'));}}));};
  const pending=s.claim({timeoutMs:cancel?1000:25,signal:abort.signal});await ready;if(cancel)abort.abort();await assert.rejects(()=>pending,{code:cancel?'cancelled':'timeout'});
 }
 assert.equal(s.offer.request.completion_operation_id,'claim-one');
}));
test('browser installed options pin the approved physical transport and current Origin',()=>browser(async()=>{
 const {session:s}=await session();const value=credential(s),serverPair=await crypto.subtle.generateKey({name:'Ed25519'},false,['sign','verify']);const server={key:serverPair.publicKey,keyId:'server-key',algorithm:'EdDSA',notBefore:0,expiresAt:Date.now()/1000+1000};
 const options=s.clientOptions(value,{url:'wss://service.fixture.invalid/connect',server});assert.equal(options.transport,'websocket');assert.equal(options.credential,value.credential.credential);assert.equal(options.signing.key.extractable,false);
 assert.throws(()=>s.clientOptions(value,{url:'https://service.fixture.invalid/connect',server}));
 assert.throws(()=>s.clientOptions(value,{url:'wss://service.fixture.invalid/connect',server,serverCertificateHashes:[{algorithm:'sha-256',value:new Uint8Array(32)}]}));
 const wtState=s.snapshot();wtState.request.transport='webtransport';wtState.offer.request.transport='webtransport';const wt=await BrowserEnrollmentSession.restore(wtState),wtValue=credential(wt),pin=new Uint8Array(32).fill(7);
 for(const pins of [Array.from({length:9},()=>({algorithm:'sha-256',value:pin})),[{algorithm:'sha-256',value:new Uint8Array(31)}],[{algorithm:'sha-512',value:pin}]])assert.throws(()=>wt.clientOptions(wtValue,{url:'https://service.fixture.invalid/connect',server,serverCertificateHashes:pins}));
 const copied=wt.clientOptions(wtValue,{url:'https://service.fixture.invalid/connect',server,serverCertificateHashes:[{algorithm:'sha-256',value:pin}]});pin.fill(8);assert.deepEqual(new Uint8Array(copied.serverCertificateHashes[0].value),new Uint8Array(32).fill(7));
 Object.defineProperty(globalThis,'location',{value:{origin:'https://other.fixture.invalid'},configurable:true});assert.throws(()=>s.clientOptions(value,{url:'wss://service.fixture.invalid/connect',server}));
}));

test('browser vault refuses denied IndexedDB before approval and sanitizes its native exception',()=>browser(async()=>{
 const {state,key}=await session(),saved=Object.getOwnPropertyDescriptor(globalThis,'indexedDB');
 Object.defineProperty(globalThis,'indexedDB',{value:{open(){throw new DOMException('private-storage-canary','SecurityError');}},configurable:true});
 try{const vault=new BrowserEnrollmentStore();await assert.rejects(()=>vault.prepare('original-name',state.origin,state.request,key),e=>e.code==='storage-unavailable'&&!e.message.includes('private-storage-canary'));}
 finally{saved?Object.defineProperty(globalThis,'indexedDB',saved):delete globalThis.indexedDB;}
}));
