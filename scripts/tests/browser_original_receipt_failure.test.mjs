import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const source=await readFile(new URL('../video/browser-public-video-roundtrip.mjs',import.meta.url),'utf8');
const start=source.indexOf('  function originalReceiptFailureDiagnostic(');
const end=source.indexOf('  // End original receipt failure diagnostics.',start);
assert.ok(start>=0&&end>start);
const helper=source.slice(start,end);
const reference={schema:'connect-operation-reference/1',operation_id:'original-move-0-1',command:'connection.replace',
 scope:{conversation_id:'room',session_id:'session',connection_id:'original-source'}};
function fixture(report=()=>{}){
 const context={originalReceiptFailure:report};vm.createContext(context);
 const functions=vm.runInContext(helper+'\n({originalReceiptFailureDiagnostic,reportOriginalReceiptFailures,originalContinuationFailureDiagnostic,observeOriginalContinuation});',context);
 return {...functions,plain:value=>JSON.parse(JSON.stringify(value))};
}
test('actual receipt diagnostic preserves distinguishable signed public failures and original reference',()=>{
 const f=fixture();
 for(const problem of [{code:503,category:'authorization',reason:'authority_unavailable'},
  {code:503,category:'storage',reason:'storage_unavailable'},{code:409,category:'conflict',reason:'operation_conflict'},
  {code:403,category:'authorization',reason:'permission_denied'}]){
  const reason={kind:problem.code===403?'authentication':'application',certainty:'read-only',problem};
  const value=f.plain(f.originalReceiptFailureDiagnostic('source-operation-wait',reason,reference,{record_id:'opr_original'}));
  assert.deepEqual(value.problem,problem);assert.equal(value.kind,reason.kind);assert.equal(value.certainty,'read-only');
  assert.deepEqual(value.operation,{operation_id:reference.operation_id,command:reference.command,scope:reference.scope});
  assert.equal(value.last_observed_record_id,'opr_original');assert.ok(JSON.stringify(value).length<=4096);
 }
});
test('actual receipt diagnostic excludes messages stacks credentials payloads and continuation bodies',()=>{
 const f=fixture(),secret='PRIVATE-CREDENTIAL-CANARY';
 const value=f.plain(f.originalReceiptFailureDiagnostic('target-prepared-completion',{
  kind:'application',certainty:'read-only',message:secret,stack:secret,credential:secret,payload:{secret},
  problem:{code:503,category:'storage',reason:'storage_unavailable',message:secret,detail:secret}},
  {...reference,credential:secret,continuation:{secret},scope:{...reference.scope,credential:secret}},
  {record_id:'opr_original',raw:secret}));
 assert.ok(!JSON.stringify(value).includes(secret));
 assert.deepEqual(Object.keys(value),['profile','receipt_owner','kind','certainty','problem','operation','last_observed_record_id']);
 assert.deepEqual(Object.keys(value.problem),['code','category','reason']);
});
test('actual receipt diagnostic bounds malformed fields and survives throwing getters',()=>{
 const f=fixture(),reason={kind:'x'.repeat(4096),certainty:'read-only',problem:{code:Infinity,category:'raw text with spaces',reason:'x'.repeat(97)}};
 Object.defineProperty(reason,'kind',{get(){throw Error('private');}});
 const value=f.plain(f.originalReceiptFailureDiagnostic('source-operation-wait',reason,{operation_id:'x'.repeat(257),command:'connection.replace',scope:null},null));
 assert.equal(value.kind,null);assert.deepEqual(value.problem,{code:null,category:null,reason:null});
 assert.equal(value.operation.operation_id,null);assert.equal(value.last_observed_record_id,null);
 assert.ok(JSON.stringify(value).length<=4096);
});
test('actual failure reporter distinguishes both original owners without replacing their rejection objects',async()=>{
 const reported=[],f=fixture(value=>{reported.push(value);throw Error('diagnostic sink unavailable');});
 const sourceReason=Object.freeze({kind:'application',certainty:'read-only',problem:{code:503,category:'authorization',reason:'authority_unavailable'}});
 const targetReason=Object.freeze({kind:'application',certainty:'read-only',problem:{code:503,category:'storage',reason:'storage_unavailable'}});
 const receipts=[{status:'rejected',reason:sourceReason},{status:'rejected',reason:targetReason}];
 await f.reportOriginalReceiptFailures(receipts,reference,{record_id:'opr_original'});
 assert.deepEqual(reported.map(value=>value.receipt_owner),['source-operation-wait','target-prepared-completion']);
 assert.equal(receipts[0].reason,sourceReason);assert.equal(receipts[1].reason,targetReason);
 assert.match(source,/for\(const receipt of receipts\)if\(receipt\.status==='rejected'\)throw receipt\.reason;/);
});
test('actual reporter has no healthy receipt observation side effect',async()=>{
 let called=0;const f=fixture(()=>{called++;});
 await f.reportOriginalReceiptFailures([{status:'fulfilled',value:1},{status:'fulfilled',value:2}],reference,null);
 assert.equal(called,0);
 assert.match(source,/if\(receipts\.some\(receipt=>receipt\.status==='rejected'\)\)await reportOriginalReceiptFailures\(receipts,reference,phases\.get\(operationId\)\);/);
});

// These continuation controls run only the actual request wrapper and diagnostic
// functions. They never connect a browser or manufacture a successful media root.
function continuationFixture(report=()=>{}){
 let originalWork,originalArguments;const observed=[];
 class BrowserClient{}
 const context={BrowserClient,originalReceiptFailure:report,observeReply:value=>observed.push(value),
  continuation(...args){originalArguments={self:this,args};return originalWork;}};
 vm.createContext(context);vm.runInContext(helper,context);
 const begin=source.indexOf('  BrowserClient.prototype.requestReplacementContinuation=function(');
 const end=source.indexOf('  const invalid=',begin);assert.ok(begin>=0&&end>begin);
 vm.runInContext(source.slice(begin,end),context);
 return {client:new BrowserClient(),observed,set:work=>{originalWork=work;},arguments:()=>originalArguments};
}
test('actual continuation diagnostic retains raw public ConnectError code and exact original request scope only',()=>{
 const f=fixture(),secret='PRIVATE-PAYLOAD-CANARY';
 const value=f.plain(f.originalContinuationFailureDiagnostic('operation.get','original',
  {cid:'room',sid:'session',connid:'source',operation_id:'ignored-offer-id',credential:secret,payload:{secret}},
  {code:'refused',message:secret,stack:secret,problem:{code:503,category:'authorization',reason:'authority_unavailable',message:secret}}));
 assert.deepEqual(value,{profile:'connect/original-continuation-failure/1',request_type:'operation.get',code:'refused',
  problem:{code:503,category:'authorization',reason:'authority_unavailable'},scope:{operation_id:'original',cid:'room',sid:'session',connid:'source'}});
 assert.ok(!JSON.stringify(value).includes(secret));assert.ok(JSON.stringify(value).length<=4096);
});
test('actual operation.get rejection before target preparation reports same request without replacing original promise or reason',async()=>{
 const reported=[],f=continuationFixture(value=>{reported.push(value);return Promise.resolve();});
 let reject;const work=new Promise((resolve,no)=>{reject=no;});f.set(work);
 const scope={cid:'room',sid:'session',connid:'source'},payload={operation_id:'original'};
 const result=f.client.requestReplacementContinuation('operation.get',payload,scope,{timeoutMs:80});
 assert.equal(result,work);assert.equal(f.arguments().self,f.client);assert.equal(f.arguments().args[1],payload);assert.equal(f.arguments().args[2],scope);
 const reason=Object.freeze({code:'refused',problem:{code:503,category:'storage',reason:'storage_unavailable'}});
 const rejected=assert.rejects(result,error=>error===reason);reject(reason);await rejected;
 assert.equal(reported.length,1);assert.equal(reported[0].code,'refused');assert.equal(reported[0].problem.reason,'storage_unavailable');
 assert.equal(reported[0].scope.operation_id,'original');assert.equal(f.observed.length,0);
});
test('actual successful continuation preserves exact promise response and original phase observer without a failure report',async()=>{
 let reported=0;const f=continuationFixture(()=>{reported++;}),response=Object.freeze([{type:'operation.updated'}]);
 const work=Promise.resolve(response);f.set(work);
 const result=f.client.requestReplacementContinuation('operation.get',{operation_id:'original'},{cid:'room',sid:'session',connid:'source'},{timeoutMs:80});
 assert.equal(result,work);assert.equal(await result,response);assert.equal(f.observed[0],response);assert.equal(reported,0);
});
test('throwing or rejected diagnostic callback never replaces original continuation failure',async()=>{
 for(const report of [()=>{throw Error('sink');},()=>Promise.reject(Error('sink'))]){
  const f=continuationFixture(report),reason=Object.freeze({code:'refused',problem:{code:503,category:'authorization',reason:'authority_unavailable'}});
  let reject;const work=new Promise((resolve,no)=>{reject=no;});f.set(work);
  const result=f.client.requestReplacementContinuation('operation.get',{operation_id:'original'},{cid:'room',sid:'session',connid:'source'},{timeoutMs:80});
  assert.equal(result,work);const rejected=assert.rejects(result,error=>error===reason);reject(reason);await rejected;
  await Promise.resolve();
 }
});
test('actual helper covers both original low-level dispatch sites with bounded hostile-field diagnostics',()=>{
 const f=fixture(),reason={code:'x'.repeat(97),problem:{code:9999,category:'x'.repeat(97),reason:'untrusted text'}};
 Object.defineProperty(reason,'problem',{get(){throw Error('private');}});
 const value=f.plain(f.originalContinuationFailureDiagnostic('operation.get','x'.repeat(257),null,reason));
 assert.equal(value.code,null);assert.deepEqual(value.problem,{code:null,category:null,reason:null});assert.equal(value.scope.operation_id,null);
 assert.equal((source.match(/return observeOriginalContinuation\(work,type,/g)||[]).length,2,'ordinary original dispatch and checkpoint-preserving public-ready dispatch both observed');
 assert.ok(source.includes('void work.then(observeReply,reason=>'));assert.ok(source.includes('   return work;'));
});
