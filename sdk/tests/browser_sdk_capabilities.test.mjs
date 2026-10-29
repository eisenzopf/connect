import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,browserSdkFeatures,sdkId} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300};
function scenario(run,{path='wss-webrtc',algorithm='EdDSA',application=false,fallback=false,handler}={}) {
 return fixture(async f=>{
  let resolutions=0,applicationReads=0;
  const client=await BrowserSdkClient.connect({tenantId:sdkId('tnt_default'),identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:fallback?'webtransport':path,resolveEnrollment:async()=>{resolutions++;return f.options;},...(application?{resolveApplicationRead:async()=>{applicationReads++;throw Error('must not run');}}:{}),timeoutMs:500});
  try{await run({client,f,counts:()=>({resolutions,applicationReads})});}finally{await client.close({timeoutMs:500});}
 },{algorithm,wtMode:path==='webtransport'?'ok':undefined,handler});
}
for(const path of ['webtransport','wss-webrtc'])for(const algorithm of ['EdDSA','ES256'])for(const application of [false,true])test(`browser capability support ${path}/${algorithm}/application=${application} is local and immutable`,()=>scenario(async({client,f,counts})=>{
 const before=f.sent.length,count=counts(),snapshot=await client.capabilities.refresh(opts);
 assert.equal(snapshot.profile,'connect/2');assert.equal(snapshot.runtimeRole,'browser');assert.equal(snapshot.selectedTransport,path);assert.equal(snapshot.fallbackReason,undefined);
 const expected=['durable-events','incoming-invitations','original-id-operations','raw-canonical',...(application?['message-receipts']:[])].sort();assert.deepEqual([...snapshot.features],expected);assert.equal(snapshot.features.size,expected.length);assert(Object.isFrozen(snapshot)&&Object.isFrozen(snapshot.features)&&Object.isFrozen(client.capabilities)&&Object.isFrozen(browserSdkFeatures));
 assert.equal(snapshot.features.add,undefined);assert.throws(()=>Set.prototype.add.call(snapshot.features,'forged'),TypeError);assert.throws(()=>snapshot.selectedTransport='quic',TypeError);
 assert.deepEqual([...snapshot.features.entries()],expected.map(v=>[v,v]));assert.deepEqual([...snapshot.features.keys()],expected);assert.deepEqual([...snapshot.features.values()],expected);
 const receiver={},seen=[];snapshot.features.forEach(function(value,key,owner){assert.equal(this,receiver);assert.equal(owner,snapshot.features);assert.equal(value,key);seen.push(value);},receiver);assert.deepEqual(seen,expected);
 await client.capabilities.require(browserSdkFeatures.originalIdOperations,opts);
 for(const name of ['native-programmatic-media','desktop-capture','provider-delivered','decoded-media-ready','unknown-feature',...(application?[]:['message-receipts'])])await assert.rejects(()=>client.capabilities.require(name,opts),{kind:'unsupported',certainty:'read-only'});
 assert.equal(f.sent.length,before);assert.deepEqual(counts(),count);
},{path,algorithm,application}));
test('browser capabilities follow explicit physical selection and retain immutable earlier snapshots',()=>scenario(async({client,f,counts})=>{
 const before=f.sent.length,first=await client.capabilities.refresh(opts);assert.equal(first.selectedTransport,'wss-webrtc');assert.equal(first.fallbackReason,'preferred-unsupported');assert.equal(f.sent.length,before);
 f.installWT('ok');await client.reconnect({preferredTransport:'webtransport',timeoutMs:500});const after=f.sent.length,second=await client.capabilities.refresh(opts);assert.equal(second.selectedTransport,'webtransport');assert.equal(second.fallbackReason,undefined);assert.equal(first.selectedTransport,'wss-webrtc');assert.equal(first.fallbackReason,'preferred-unsupported');assert.equal(f.sent.length,after);assert.equal(counts().resolutions,2);
},{fallback:true}));
test('invalid, expired budget and caller abort capability checks send no control or resolver work',()=>scenario(async({client,f,counts})=>{
 const before=f.sent.length,count=counts();for(const feature of ['',null,'RAW','private feature','a'.repeat(129)])await assert.rejects(()=>client.capabilities.require(feature,opts),{kind:'validation'});
 for(const timeoutMs of [0,NaN,Infinity,60001])await assert.rejects(()=>client.capabilities.refresh({timeoutMs}),{kind:'validation'});
 const stop=new AbortController();stop.abort();await assert.rejects(()=>client.capabilities.refresh({...opts,signal:stop.signal}),{kind:'cancelled',certainty:'read-only'});
 const late=new AbortController(),pending=client.capabilities.require(browserSdkFeatures.originalIdOperations,{...opts,signal:late.signal});late.abort();await assert.rejects(()=>pending,{kind:'cancelled',certainty:'read-only'});
 assert.equal(f.sent.length,before);assert.deepEqual(counts(),count);assert.equal(client.transport.state,'connected');
}));
test('expired signed authentication cannot return a live local capability profile',()=>scenario(async({client,f})=>{
 const before=f.sent.length,now=Date.now;Date.now=()=>now()+120000;
 try{await assert.rejects(()=>client.capabilities.refresh(opts),{kind:'authentication',certainty:'read-only'});await assert.rejects(()=>client.capabilities.require(browserSdkFeatures.originalIdOperations,opts),{kind:'authentication'});}finally{Date.now=now;}
 assert.equal(f.sent.length,before);assert.equal(client.transport.state,'connected');assert.equal((await client.capabilities.refresh(opts)).runtimeRole,'browser');
}));
test('closed or disconnected browser owners cannot fabricate live capability snapshots',()=>scenario(async({client,f})=>{
 f.socket().close();await new Promise(setImmediate);assert.equal(client.transport.state,'disconnected');await assert.rejects(()=>client.capabilities.refresh(opts),{kind:'cancelled',certainty:'read-only'});await client.reconnect({timeoutMs:500});await client.capabilities.require(browserSdkFeatures.originalIdOperations,opts);await client.close({timeoutMs:500});await assert.rejects(()=>client.capabilities.refresh(opts),{kind:'cancelled',certainty:'read-only'});
}));
test('local capability support never supplies cached server permission after revocation',()=>scenario(async({client,f})=>{
 const snapshot=await client.capabilities.refresh(opts),before=f.sent.length;f.setCredential('revoked');await client.capabilities.require(browserSdkFeatures.originalIdOperations,opts);assert.equal(f.sent.length,before);
 await assert.rejects(()=>client.conversations.create({policy:'persistent',participants:[]},{...opts,operationId:sdkId('refused-create')}),{kind:'authentication',certainty:'refused'});assert.equal(snapshot.features.has(browserSdkFeatures.originalIdOperations),true);assert.equal(f.sent.filter(v=>v.type==='conversation.create').length,1);
}));

test('local capability checks do not consume the sixteen occupied canonical control slots',()=>{
 const ready=Promise.withResolvers();let seen=0;
 return scenario(async({client,f})=>{
  const operation=client.operations.restore({schema:'connect-operation-reference/1',operation_id:'saved',command:'conversation.create',scope:{}},client.operationKinds.conversationCreate),pending=Array.from({length:16},()=>operation.status({timeoutMs:5000})),settled=Promise.allSettled(pending);let timer;
  try{await Promise.race([ready.promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('sixteen requests not observed')),300);})]);}finally{clearTimeout(timer);}
  assert.equal(f.sent.filter(v=>v.type==='operation.get').length,16);const before=f.sent.length;await client.capabilities.require(browserSdkFeatures.originalIdOperations,opts);assert.equal(f.sent.length,before);await client.close({timeoutMs:500});assert((await settled).every(v=>v.status==='rejected'));
 },{handler:async({frame})=>{if(frame.type!=='operation.get')return false;if(++seen===16)ready.resolve();return true;}});
});
test('capability require cannot cross a physical retirement during its local continuation',()=>scenario(async({client})=>{
 const pending=client.capabilities.require(browserSdkFeatures.originalIdOperations,opts);await client.close({timeoutMs:500});await assert.rejects(()=>pending,{kind:'cancelled',certainty:'read-only'});
}));
