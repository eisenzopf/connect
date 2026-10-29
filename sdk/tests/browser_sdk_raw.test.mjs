import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,browserSdkFeatures} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300},request=(type='operation.get',extra={})=>({v:2,id:'caller-metadata',ts:'2026-01-01T00:00:00Z',type,cid:'room',payload:{operation_id:'decision-original',vendor:{retained:[true,null,1]}},...extra});
const cancellation=()=>request('operation.cancel',{operation_id:'decision-original',payload:{operation_id:'original',vendor:{retained:[true,null,1]}}});
function keyError(kind,certainty){return error=>{assert.equal(error.kind,kind);assert.equal(error.certainty,certainty);assert.deepEqual(error.operation,{operationId:'decision-original',command:'operation.cancel',scope:{conversationId:'room'}});return true;};}
function scenario(run,{path='wss-webrtc',algorithm='EdDSA',fault}={}){
 let mode=fault;const record={operation_id:'decision-original',record_id:'host-decision-original',command_type:'operation.cancel',state:'completed',revision:1,result:{target_operation_id:'original',state_at_decision:'completed',disposition:'already_terminal',evidence_id:null}};
 return fixture(async f=>{
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:path,resolveEnrollment:async()=>f.options,timeoutMs:500});
  try{await run({client,f,setMode:v=>mode=v});}finally{await client.close({timeoutMs:500});}
 },{algorithm,wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
  if(!['operation.cancel','operation.get','conversation.subscribe'].includes(frame.type))return false;
  const scope={cid:frame.cid,sid:frame.sid,connid:frame.connid,operation_id:frame.operation_id},emit=(type,payload,headers=scope,extensions)=>peer.emit(type,payload,frame.id,headers,extensions);
  if(mode==='revoked')return emit('error',{code:403,category:'auth',reason:'forbidden'}).then(()=>true);
  if(mode==='lost-ack'||mode==='stall')return true;
  if(mode==='unavailable')return emit('error',{code:503,category:'server',reason:'temporarily_unavailable'}).then(()=>true);
  if(mode==='bad-error')return emit('error',{code:403,category:'auth',details:'private-canary'}).then(()=>true);
  const value=structuredClone(record);if(mode==='bad-schema')delete value.revision;if(mode==='hint')value.journal_hint=true;
  const first=await emit(mode==='unknown-kind'?'vendor.unsupported':'operation.updated',value,mode==='scope'?{...scope,cid:'foreign'}:scope,{vendor:mode==='echo'?frame.payload.vendor:{kept:'signed-extension'}});
  if(mode==='multi')await emit('operation.updated',{...value,revision:2});
  if(mode==='replay')await emit('operation.updated',value,scope,{id:first.id});
  await emit('ack',{details:{vendor:{last:true}}});return true;
 }});
}
for(const path of ['webtransport','wss-webrtc'])for(const algorithm of ['EdDSA','ES256'])test(`raw ${path}/${algorithm} captures intent, fresh physical metadata and all signed immutable response fields`,()=>scenario(async({client,f})=>{
 const before=f.sent.length,mutable=cancellation();mutable.signature={keyid:'caller-only',alg:'PS256',sig:'locally-not-verified'};const validated=client.raw.validate(mutable);mutable.payload.operation_id='changed';mutable.payload.vendor.retained.push('changed');mutable.operation_id='changed';assert.equal(f.sent.length,before);assert(Object.isFrozen(validated)&&Object.isFrozen(validated.value.payload.vendor.retained));assert.equal(validated.value.signature.alg,'PS256');
 await client.capabilities.require(browserSdkFeatures.rawCanonical,opts);assert.equal(f.sent.length,before);const result=await client.raw.request(validated,opts),sent=f.sent.find(v=>v.type==='operation.cancel');assert.equal(sent.operation_id,'decision-original');assert.equal(sent.payload.operation_id,'original');assert.deepEqual(JSON.parse(JSON.stringify(sent.payload.vendor)),{retained:[true,null,1]});assert.notEqual(sent.id,'caller-metadata');assert.notEqual(sent.ts,'2026-01-01T00:00:00Z');assert.equal(sent.signature.keyid,f.options.signing.keyId);assert.equal(sent.signature.alg,algorithm);assert.equal(result.length,2);assert.deepEqual(result.map(v=>v.value.type),['operation.updated','ack']);assert.equal(result[0].value.in_reply_to,sent.id);assert.equal(result[1].value.in_reply_to,sent.id);assert.equal(result[0].value.vendor.kept,'signed-extension');assert.equal(result[0].value.signature.alg,algorithm);assert(Object.isFrozen(result)&&Object.isFrozen(result[0].value)&&Object.isFrozen(result[0].value.signature));assert.equal(f.sent.filter(v=>v.type==='operation.get').length,0);
 const original=await client.raw.request(client.raw.validate(request()),opts);assert.equal(original[0].value.payload.operation_id,'decision-original');assert.equal(f.sent.filter(v=>v.type==='operation.cancel').length,1);
},{path,algorithm}));
test('raw retains every valid correlated observation through the final ACK',()=>scenario(async({client})=>{
 const frames=await client.raw.request(client.raw.validate(request()),opts);assert.deepEqual(frames.map(v=>v.value.type),['operation.updated','operation.updated','ack']);assert.equal(frames[1].value.payload.revision,2);
},{fault:'multi'}));
for(const fault of ['bad-schema','hint','unknown-kind','scope','replay','bad-error'])test('raw '+fault+' signed response is a protocol failure with original uncertain mutation intent',()=>scenario(async({client})=>{
 await assert.rejects(()=>client.raw.request(client.raw.validate(cancellation()),opts),error=>{keyError('protocol','uncertain')(error);assert.equal(error.message.includes('private-canary'),false);return true;});assert.equal(client.transport.state,'disconnected');
},{fault}));
for(const fault of ['lost-ack','unavailable'])test('raw '+fault+' retains one logical ID and recovers status after explicit reconnect without replay',()=>scenario(async({client,f,setMode})=>{
 await assert.rejects(()=>client.raw.request(client.raw.validate(cancellation()),{timeoutMs:25}),keyError(fault==='lost-ack'?'deadline':'application','uncertain'));setMode(undefined);await client.reconnect({timeoutMs:500});const read=await client.raw.request(client.raw.validate(request()),opts);assert.equal(read[0].value.payload.result.target_operation_id,'original');assert.equal(f.sent.filter(v=>v.type==='operation.cancel').length,1);
},{fault}));
test('raw current refusal cannot return previously observed data or cached permission',()=>scenario(async({client,f,setMode})=>{
 const valid=client.raw.validate(request());await client.raw.request(valid,opts);setMode('revoked');await assert.rejects(()=>client.raw.request(valid,opts),{kind:'authentication',certainty:'refused'});assert.equal(f.sent.filter(v=>v.type==='operation.get').length,2);
}));
test('raw local validation and unsupported request classification perform no IO',()=>scenario(async({client,f})=>{
 const before=f.sent.length;assert.throws(()=>client.raw.validate(request('operation.get',{cid:'resource\n'})),{kind:'validation',certainty:'not-submitted'});await assert.rejects(()=>client.raw.request({value:request()},opts),{kind:'validation'});
 const extension=client.raw.validate({...cancellation(),vendor:{retained:true}});assert.equal(extension.value.vendor.retained,true);await assert.rejects(()=>client.raw.request(extension,opts),{kind:'unsupported',certainty:'not-submitted'});
 for(const input of [request('ack',{payload:{}}),request('auth.keepalive',{payload:{session_token:'local'}}),request('conversation.subscribe',{payload:{live:true}}),request('operation.get',{in_reply_to:'original-attempt'})])await assert.rejects(()=>client.raw.request(client.raw.validate(input),opts),{kind:'unsupported'});
 const hierarchy=client.raw.validate(request('operation.get',{connid:'connection'}));await assert.rejects(()=>client.raw.request(hierarchy,opts),{kind:'validation'});assert.equal(f.sent.length,before);
}));
test('raw pure validation remains available after close while requests cannot borrow another owner',()=>scenario(async({client,f})=>{
 const before=f.sent.length;await client.close({timeoutMs:500});const local=client.raw.validate(cancellation());await assert.rejects(()=>client.raw.request(local,opts),{certainty:'not-submitted'});assert.equal(f.sent.length,before);
}));
test('raw pre-abort, invalid budget and options keep a mutation known not submitted',()=>scenario(async({client,f})=>{
 const before=f.sent.length,value=client.raw.validate(cancellation()),stop=new AbortController();stop.abort();await assert.rejects(()=>client.raw.request(value,{...opts,signal:stop.signal}),keyError('cancelled','not-submitted'));for(const timeoutMs of [0,Infinity,60001])await assert.rejects(()=>client.raw.request(value,{timeoutMs}),keyError('validation','not-submitted'));await assert.rejects(()=>client.raw.request(value,{...opts,terminalType:'auth.session'}),{kind:'validation',certainty:'not-submitted'});assert.equal(f.sent.length,before);
}));
test('raw submitted abort preserves uncertainty without an automatic cancellation',()=>scenario(async({client,f})=>{
 const stop=new AbortController(),pending=client.raw.request(client.raw.validate(cancellation()),{...opts,signal:stop.signal});setTimeout(()=>stop.abort(),5);await assert.rejects(()=>pending,keyError('cancelled','uncertain'));assert.equal(f.sent.filter(v=>v.type==='operation.cancel').length,1);
},{fault:'stall'}));

for(const path of ['webtransport','wss-webrtc'])for(const graph of ['deep','wide'])test('raw '+path+' preserves a '+graph+' fixed-wire extension beyond application JSON defaults',()=>scenario(async({client,f})=>{
 let vendor=graph==='wide'?Array(120000).fill(0):null;if(graph==='deep')for(let i=0;i<125;i++)vendor={node:vendor};const input=request();input.payload.vendor=vendor;const result=await client.raw.request(client.raw.validate(input),{timeoutMs:2000});if(graph==='wide')assert.equal(result[0].value.vendor.length,120000);else{let value=result[0].value.vendor;for(let i=0;i<125;i++)value=value.node;assert.equal(value,null);}assert(Object.isFrozen(result[0].value.vendor));assert.equal(f.sent.filter(v=>v.type==='operation.get').length,1);
},{path,fault:'echo'}));
