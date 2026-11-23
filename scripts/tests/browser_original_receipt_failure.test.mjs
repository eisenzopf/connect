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


// OriginalV8-only passive diagnostics: execute extracted helpers, never a browser.
const settlementSources=[source,await readFile(new URL('../video/browser-public-video-abort.mjs',import.meta.url),'utf8')];
function settlementHelperFor(sourceText){
 const start=sourceText.indexOf('  function originalReceiptFailureDiagnostic(');
 const end=sourceText.indexOf('  // End original receipt failure diagnostics.',start);
 assert.ok(start>=0&&end>start);return sourceText.slice(start,end);
}
function settlementFixture(report=()=>{},sourceText=source){
 const helper=settlementHelperFor(sourceText);
 let now=10;const context={originalReceiptFailure:report,performance:{now:()=>now}};vm.createContext(context);
 const f=vm.runInContext(helper+'\n({observeOriginalReplacement,observeOriginalSettlement,originalSettlementScope,disposeOriginalSettlementObservers});',context);
 return {...f,clock:value=>{now=value;},plain:value=>JSON.parse(JSON.stringify(value))};
}
function settlementOwner(){
 const callbacks=new Set();let closes=0,disposals=0;
 return {owner:{close(){closes++;},onClose(fn){callbacks.add(fn);return()=>{disposals++;callbacks.delete(fn);};}},
  notify:reason=>{for(const fn of callbacks)fn(reason);},count:()=>callbacks.size,disposals:()=>disposals,closes:()=>closes};
}
const settlementInput={operationId:'move-1-0',conversationId:'room',sessionId:'session',connectionId:'candidate'};
const settlementRoot={...settlementInput,recordId:'opr_original',targetConnectionId:'candidate'};
test('passive lower target observer preserves original object promises values and rejection identities',async()=>{
 for(const sourceText of settlementSources){
 const rows=[],f=settlementFixture(value=>rows.push(value),sourceText),owner=settlementOwner();let no;
 const reason=Object.freeze({code:'media-decoder-stopped',name:'ConnectError',message:'PRIVATE-CANARY'}),value=Object.freeze({ready:true});
 const prepared=Promise.resolve(value),completed=new Promise((yes,reject)=>{no=reject;});
 const begun=Object.freeze({...owner,prepared,completed});
 assert.equal(f.observeOriginalReplacement(begun,settlementInput,settlementRoot),begun);
 assert.equal(begun.prepared,prepared);assert.equal(begun.completed,completed);assert.equal(await prepared,value);
 const failed=assert.rejects(completed,error=>error===reason);owner.notify(reason);no(reason);await failed;await Promise.resolve();
 assert.deepEqual(rows.map(row=>row.stage),['target-original-prepared','target-owner-close','target-original-completed']);
 assert.equal(rows[1].code,'media-decoder-stopped');assert.equal(rows[1].name,'ConnectError');
 assert.equal(rows[1].scope.record_id,'opr_original');assert.ok(!JSON.stringify(rows).includes('PRIVATE-CANARY'));
 f.disposeOriginalSettlementObservers();assert.equal(owner.count(),0);assert.equal(owner.disposals(),1);assert.equal(owner.closes(),0);
 }
});
test('passive receipt chronology follows actual settlements while allSettled retains original input ordering',async()=>{
 for(const sourceText of settlementSources){
 const rows=[],f=settlementFixture(value=>rows.push(value),sourceText);let sourceNo,targetNo;
 const sourceWork=new Promise((yes,no)=>{sourceNo=no;}),targetWork=new Promise((yes,no)=>{targetNo=no;});
 const scope=f.originalSettlementScope(settlementInput,settlementRoot),sourceReason=Object.freeze({code:'refused'}),targetReason=Object.freeze({code:'media-stopped'});
 assert.equal(f.observeOriginalSettlement(sourceWork,'source-operation-wait',scope),sourceWork);
 assert.equal(f.observeOriginalSettlement(targetWork,'target-prepared-completion',scope),targetWork);
 const receipts=Promise.allSettled([sourceWork,targetWork]);f.clock(20);targetNo(targetReason);await Promise.resolve();
 f.clock(30);sourceNo(sourceReason);const actual=await receipts;
 assert.equal(actual[0].reason,sourceReason);assert.equal(actual[1].reason,targetReason);
 assert.deepEqual(rows.map(row=>[row.stage,row.sequence,row.time_ms]),[['target-prepared-completion',1,20],['source-operation-wait',2,30]]);
 }
});
test('passive target observer refuses foreign root and sink failures cannot change original work',async()=>{
 for(const sourceText of settlementSources){
 for(const report of [()=>{throw Error('sink');},()=>Promise.reject(Error('sink'))]){
  const f=settlementFixture(report,sourceText),owner=settlementOwner(),value=Object.freeze({done:true}),work=Promise.resolve(value),begun={...owner,prepared:work,completed:work};
  assert.equal(f.observeOriginalReplacement(begun,settlementInput,{...settlementRoot,targetConnectionId:'foreign'}),begun);
  assert.equal(owner.count(),0);assert.equal(await work,value);
  assert.equal(f.observeOriginalReplacement(begun,settlementInput,settlementRoot),begun);assert.equal(await work,value);
  const reason=Object.freeze({code:'refused'}),early=Promise.reject(reason);
  assert.equal(f.observeOriginalSettlement(early,'target-prepared-completion',f.originalSettlementScope(settlementInput,settlementRoot)),early);
  await assert.rejects(early,error=>error===reason);
  f.disposeOriginalSettlementObservers();assert.equal(owner.count(),0);await Promise.resolve();
 }
 }
});
test('passive lower errors are bounded and redact hostile getters and freeform fields',async()=>{
 for(const sourceText of settlementSources){
 const rows=[],f=settlementFixture(value=>rows.push(value),sourceText),owner=settlementOwner(),reason={name:'bad name',code:'raw private text',stack:'SECRET',problem:{code:503,category:'storage',reason:'storage_unavailable'}};
 Object.defineProperty(reason,'kind',{get(){throw Error('SECRET');}});
 const begun={...owner,prepared:Promise.resolve(1),completed:Promise.resolve(2)};
 f.observeOriginalReplacement(begun,settlementInput,settlementRoot);owner.notify(reason);await Promise.resolve();
 const row=rows.find(row=>row.stage==='target-owner-close');assert.equal(row.name,null);assert.equal(row.code,null);assert.equal(row.kind,null);
 assert.deepEqual(f.plain(row.problem),{code:503,category:'storage',reason:'storage_unavailable'});
 assert.ok(!JSON.stringify(rows).includes('SECRET'));assert.ok(rows.every(row=>JSON.stringify(row).length<=4096));
 f.disposeOriginalSettlementObservers();
 assert.match(sourceText,/try\{const begun=original\.apply\(this,args\);return name==='beginReplacement'\?observeOriginalReplacement\(begun,args\[1\],args\[4\]\):begun;\}finally\{arming=null;\}/);
 }
});

test('actual constructor wrapper delegates once with same receiver arguments and synchronous failure',async()=>{
 for(const sourceText of settlementSources){
 const rows=[],owner=settlementOwner(),prepared=Promise.resolve(1),completed=Promise.resolve(2),begun={...owner,prepared,completed};
 const invocations=[];let thrown;
 function original(...args){invocations.push({self:this,args});if(thrown)throw thrown;return begun;}
 const BrowserRtcConnection={},context={BrowserRtcConnection,begin:original,beginReplacement:original,arming:null,
  check:(ok,message)=>assert.ok(ok,message),originalReceiptFailure:value=>rows.push(value),performance:{now:()=>10}};
 vm.createContext(context);vm.runInContext(settlementHelperFor(sourceText),context);
 const start=sourceText.indexOf("  for(const [name,original]of [['begin',begin],['beginReplacement',beginReplacement]])");
 const end=sourceText.indexOf('  HTMLVideoElement.prototype.requestVideoFrameCallback=',start);assert.ok(start>=0&&end>start);
 vm.runInContext(sourceText.slice(start,end),context);
 const receiver={},args=[{},settlementInput,'participant',{},settlementRoot,{timeoutMs:80}];
 assert.equal(BrowserRtcConnection.beginReplacement.apply(receiver,args),begun);assert.equal(invocations.length,1);
 assert.equal(invocations[0].self,receiver);args.forEach((arg,index)=>assert.equal(invocations[0].args[index],arg));
 assert.equal(begun.prepared,prepared);assert.equal(begun.completed,completed);assert.equal(context.arming,null);
 thrown=Object.freeze({code:'media-stopped'});assert.throws(()=>BrowserRtcConnection.beginReplacement.apply(receiver,args),error=>error===thrown);
 assert.equal(invocations.length,2);assert.equal(context.arming,null);assert.equal(owner.count(),1);
 vm.runInContext('disposeOriginalSettlementObservers()',context);assert.equal(owner.count(),0);await Promise.all([prepared,completed]);
 }
});
