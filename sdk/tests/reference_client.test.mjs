import assert from 'node:assert/strict';
import {test} from 'node:test';
import fs from 'node:fs';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import {IntentJournal,configuredVoiceBinding,historyPage} from '../../examples/conversation-client/workflow.js';
const storage = () => {const values=new Map();return {getItem:k=>values.get(k)??null,setItem:(k,v)=>{if(v.length>128000)throw Error('quota');values.set(k,v);}};};
test('reference client reuses saved mutation identity after lost acknowledgement and reload',async()=>{
 const saved=storage(), submitted=[];
 const client={request:async op=>{submitted.push(op);if(submitted.length===1)throw Error('lost ack');return {kind:'message',data:{id:'one'}};}};
 const input={op:'send',conversation_id:'c',body:'one'};
 await assert.rejects(new IntentJournal(saved,'identity').run(client,'send:c',input),/lost ack/);
 const reloaded=new IntentJournal(saved,'identity');
 await assert.rejects(reloaded.run(client,'send:c',{...input,body:'different'}),/unresolved/);
 await reloaded.retry(client);
 assert.deepEqual(submitted[0],submitted[1]);
 await reloaded.run(client,'send:c',input); assert.equal(submitted.length,2);
});
test('storage failure prevents submission and voice mismatch fails before opening media',async()=>{
 let calls=0;
 const journal=new IntentJournal({getItem:()=>null,setItem:()=>{throw Error('quota');}},'identity');
 await assert.rejects(journal.run({request:async()=>{calls++;}},'send',{op:'send'}),/quota/);
 assert.equal(calls,0);
 const client={browserBinding:async()=>({conversation_id:'actual',participant_id:'assistant',signaling_url:'ws://127.0.0.1:1234'})};
 await assert.rejects(configuredVoiceBinding(client,'selected','assistant'),/configured for conversation actual/);
 assert.equal((await configuredVoiceBinding(client,'actual','assistant')).conversation_id,'actual');
});
test('actual reference UI continues history, exposes failed context receipts and resumes upload',async()=>{
 const el=()=>({value:'',textContent:'',children:[],style:{},files:[],append(...v){this.children.push(...v);},replaceChildren(){this.children=[];},addEventListener(){}});
 const elements=new Map();const byId=id=>{if(!elements.has(id))elements.set(id,el());return elements.get(id);};
 let page=0, lost=true; const appends=[];
 const client={health:async()=>({schema:10,profile:'fixture'}),history:async(cid,cursor)=>{
   assert.equal(cursor,page===0?null:page===1?'cursor-100':'cursor-101');
   return page++===0?{messages:Array.from({length:100},(_,i)=>({id:`m${i}`,sender:'human',body:`message ${i}`})),next:'cursor-100'}:{messages:page===2?[{id:'m100',sender:'human',body:'Newest message'}]:[],next:'cursor-101'};
 },contextView:async()=>({watermark:101,omitted:1,deliveries_omitted:2,items:[]}),contextDeliveries:async()=>({deliveries:[{state:'failed',provider:'vapi',session_id:'session',watermark:101,id:'receipt'}],next:null}),request:async op=>{
   if(op.op==='artifact_begin') return {kind:'artifact',data:{id:'artifact'}};
   if(op.op==='artifact_append'){appends.push(op);if(lost){lost=false;throw Error('lost chunk ack');}}
   return {kind:'artifact',data:{id:'artifact'}};
 }};
 class BrowserHttpClient{constructor(){return client;}}
 const source=fs.readFileSync(new URL('../../examples/conversation-client/app.js',import.meta.url),'utf8').replace(/^import[\s\S]*?from .*?;\n/gm,'');
 const sandbox={window:{addEventListener(){}},document:{getElementById:byId,createElement:el,querySelectorAll:()=>[]},sessionStorage:storage(),BrowserHttpClient,ConnectError:class extends Error{},BrowserVoiceSession:{},IntentJournal,configuredVoiceBinding,historyPage,crypto:webcrypto,Uint8Array,btoa:globalThis.btoa,console};
 vm.createContext(sandbox);vm.runInContext(source,sandbox);
 byId('identity').value='human';byId('http').value='http://127.0.0.1:1234';byId('conversation').value='c';
 await byId('connect').onclick();await byId('refresh').onclick();await byId('refresh').onclick();
 assert.equal(byId('messages').children.length,101);
 assert.match(JSON.stringify(byId('messages')),/Newest message/);
 assert.match(JSON.stringify(byId('deliveries')),/failed/);
 assert.match(byId('context-state').textContent,/2 older delivery receipts/);
 const bytes=new Uint8Array(70000);byId('file').files=[{name:'fixture.bin',size:bytes.length,type:'application/octet-stream',arrayBuffer:async()=>bytes.buffer}];
 await byId('upload').onclick();assert.equal(byId('state').textContent,'Action failed');
 await byId('upload').onclick();assert.equal(byId('state').textContent,'File uploaded',byId('detail').textContent);
 assert.equal(appends.length,4);assert.equal(appends[0].request_id,appends[1].request_id);
 assert.equal(new Set(appends.map(x=>x.request_id)).size,3);
});
test('definite initial rejection permits corrected input; denial after uncertainty preserves intent',async()=>{
 const saved=storage(),journal=new IntentJournal(saved,'identity');
 const input={op:'send',conversation_id:'c',body:'too large'};
 const denied=async()=>{throw Object.assign(Error('invalid'),{code:'invalid-request'});};
 await assert.rejects(journal.run({request:denied},'send:c',input));
 assert.deepEqual(journal.read(),{});
 await assert.rejects(journal.run({request:async()=>{throw Error('lost ack');}},'send:c',input));
 const pending=journal.read()['send:c'].operation.request_id;
 await assert.rejects(journal.retry({request:denied}));
 assert.equal(journal.read()['send:c'].operation.request_id,pending);
});
