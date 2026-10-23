import assert from 'node:assert/strict';
import {test} from 'node:test';
import fs from 'node:fs';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import {IntentJournal,configuredVoiceBinding,historyPage,readArtifactBytes} from '../../examples/conversation-client/workflow.js';

function contactFixture(client) {
 const elements=new Map();
 const byId=id=>{
  if(!elements.has(id))elements.set(id,{value:'',textContent:'',children:[],listeners:new Map(),
   append(row){this.children.push(row);},replaceChildren(){this.children=[];},
   addEventListener(name,callback){this.listeners.set(name,callback);}});
  return elements.get(id);
 };
 const sandbox={document:{getElementById:byId,createElement:()=>({textContent:''})},Date,
  act:async(_label,action)=>action(),needClient:()=>client,value:id=>byId(id).value.trim()};
 const source=fs.readFileSync(new URL('../../examples/conversation-client/contacts.js',import.meta.url),'utf8')
  .replace(/^import[^\n]+\n/,'');
 vm.createContext(sandbox);vm.runInContext(source,sandbox);
 return {byId,edit(id,value){byId(id).value=value;byId(id).listeners.get('input')?.();},
  inspect:()=>byId('inspect-contact').onclick()};
}
function contactClient(calls=[]) {
 return {resolve:async address=>{calls.push(['resolve',address]);return {id:'identity-'+address};},
  capabilities:async(address,conversation)=>{calls.push(['capabilities',address,conversation]);return {
   availability:{state:'available',expires_at:Math.floor(Date.now()/1000)+300},
   routes:[{transport:'native',media:['text'],readiness:'ready'}]};},
  contactState:async identity=>{calls.push(['contactState',identity]);return {ownership:'unknown'};}};
}
function deferred() {let resolve;const promise=new Promise(done=>resolve=done);return {promise,resolve};}

test('reference contact lookup never mixes a delayed identity with edited address or conversation',async()=>{
 for(const field of ['contact-address','conversation']) {
  const calls=[],client=contactClient(calls),pending=deferred();
  client.resolve=async address=>{calls.push(['resolve',address]);return pending.promise;};
  const ui=contactFixture(client);ui.edit('contact-address','alice');ui.edit('conversation','first');
  const lookup=ui.inspect();ui.edit(field,'changed');pending.resolve({id:'identity-alice'});
  await assert.rejects(lookup,/selection changed/i);
  assert.deepEqual(calls,[['resolve','alice']]);
  assert.equal(ui.byId('participant').value,'');assert.equal(ui.byId('contact-state').textContent,'');
  assert.deepEqual(ui.byId('contact-routes').children,[]);
 }
});

test('reference contact input edits invalidate pending and completed selection even after edit-back',async()=>{
 for(const field of ['contact-address','conversation','participant']) {
  const client=contactClient(),pending=deferred(),started=deferred();
  client.contactState=async()=>{started.resolve();return pending.promise;};
  const ui=contactFixture(client);ui.edit('contact-address','alice');ui.edit('conversation','first');
  const lookup=ui.inspect();await started.promise;
  const original=ui.byId(field).value;ui.edit(field,'changed');ui.edit(field,original);
  pending.resolve({ownership:'verified_native'});
  await assert.rejects(lookup,/selection changed/i);
  assert.equal(ui.byId('participant').value,'');assert.equal(ui.byId('contact-state').textContent,'');
  assert.deepEqual(ui.byId('contact-routes').children,[]);
 }
 const ui=contactFixture(contactClient());ui.edit('contact-address','alice');await ui.inspect();
 assert.equal(ui.byId('participant').value,'identity-alice');
 ui.edit('conversation','another');
 assert.equal(ui.byId('participant').value,'');assert.equal(ui.byId('contact-state').textContent,'');
 assert.deepEqual(ui.byId('contact-routes').children,[]);
});

test('reference contact replacement failure clears owned selection without clearing manual participant',async()=>{
 for(const stage of ['resolve','capabilities','contactState']) {
  const calls=[],client=contactClient(calls),ui=contactFixture(client);
  ui.edit('contact-address','alice');ui.edit('conversation','first');await ui.inspect();
  assert.deepEqual(calls,[['resolve','alice'],['capabilities','alice','first'],['contactState','identity-alice']]);
  assert.equal(ui.byId('participant').value,'identity-alice');
  assert.match(ui.byId('contact-state').textContent,/Native ownership: unknown.*available.*not.*SMS/);
  assert.equal(ui.byId('contact-routes').children[0].textContent,'native · text · ready');
  client[stage]=async()=>{throw Error('selected lookup failed');};
  // A programmatic form update can omit the input event; starting lookup must clear old output too.
  ui.byId('contact-address').value='bob';
  await assert.rejects(ui.inspect(),/selected lookup failed/);
  assert.equal(ui.byId('participant').value,'');assert.equal(ui.byId('contact-state').textContent,'');
  assert.deepEqual(ui.byId('contact-routes').children,[]);
 }
 const ui=contactFixture(contactClient());ui.edit('contact-address','alice');await ui.inspect();
 ui.edit('participant','manually-selected');
 assert.equal(ui.byId('participant').value,'manually-selected');
 assert.equal(ui.byId('contact-state').textContent,'');assert.deepEqual(ui.byId('contact-routes').children,[]);
});
const storage = () => {const values=new Map();return {getItem:k=>values.get(k)??null,setItem:(k,v)=>{if(v.length>128000)throw Error('quota');values.set(k,v);}};};
test('reference client reuses saved mutation identity after lost acknowledgement and reload',async()=>{
 const saved=storage(), submitted=[];
 const client={request:async op=>{submitted.push(op);if(submitted.length===1)throw Error('lost ack');return {kind:'message',data:{id:'one'}};}};
 const input={op:'send',conversation_id:'c',message:{to:'all',content_type:'text/plain',body:'one',body_encoding:'utf8',label:'connect-messages',attachments:[],in_reply_to_msg:null,msg_id:null}};
 await assert.rejects(new IntentJournal(saved,'identity').run(client,'send:c',input),/lost ack/);
 const reloaded=new IntentJournal(saved,'identity');
 await assert.rejects(reloaded.run(client,'send:c',{...input,message:{...input.message,body:'different'}}),/unresolved/);
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
   return page++===0?{messages:Array.from({length:31},(_,i)=>({id:`m${i}`,sender:'human',body:`message ${i}`})),next:'cursor-100'}:{messages:page===2?[{id:'m100',sender:'human',body:'Newest message'}]:[],next:'cursor-101'};
 },contextView:async()=>({watermark:101,omitted:1,deliveries_omitted:2,items:[{kind:"artifact_ready",body:JSON.stringify({id:"existing",name:"existing.bin",size:10,state:"ready"})}]}),contextDeliveries:async()=>({deliveries:[{state:'failed',provider:'vapi',session_id:'session',watermark:101,id:'receipt'}],next:null}),request:async op=>{
   if(op.op==='artifact_begin') return {kind:'artifact',data:{id:'artifact'}};
   if(op.op==='artifact_append'){appends.push(op);if(lost){lost=false;throw Error('lost chunk ack');}}
   return {kind:'artifact',data:{id:'artifact'}};
 }};
 class BrowserHttpClient{constructor(){return client;}}
 const source=fs.readFileSync(new URL('../../examples/conversation-client/app.js',import.meta.url),'utf8').replace(/^import[\s\S]*?from .*?;\n/gm,'').replace(/^export \{[^}]+\};/gm,'');
 const sandbox={window:{addEventListener(){}},document:{getElementById:byId,createElement:el,querySelectorAll:()=>[]},sessionStorage:storage(),BrowserHttpClient,ConnectError:class extends Error{},BrowserVoiceSession:{},IntentJournal,configuredVoiceBinding,historyPage,crypto:webcrypto,Uint8Array,btoa:globalThis.btoa,console};
 vm.createContext(sandbox);vm.runInContext(source,sandbox);
 const filesSource=fs.readFileSync(new URL('../../examples/conversation-client/files.js',import.meta.url),'utf8').replace(/^import[\s\S]*?from .*?;\n/gm,'').replace(/^export function/gm,'function');
 vm.runInContext(filesSource,sandbox);
 byId('identity').value='human';byId('http').value='http://127.0.0.1:1234';byId('conversation').value='c';
 await byId('connect').onclick();await byId('refresh').onclick();
 assert.equal(byId('more-history').disabled,false, 'byte-capped short page retains unread cursor');
 await byId('refresh').onclick();
 assert.equal(byId('messages').children.length,32);
 assert.match(JSON.stringify(byId('messages')),/Newest message/);
 assert.match(JSON.stringify(byId('deliveries')),/failed/);
 assert.match(byId('context-state').textContent,/2 older delivery receipts/);
 assert.equal(byId('artifact-list').children.length,1);
 assert.match(byId('artifact-list').children[0].children[0].textContent,/existing.bin/);
 const bytes=new Uint8Array(70000);byId('file').files=[{name:'fixture.bin',size:bytes.length,type:'application/octet-stream',arrayBuffer:async()=>bytes.buffer}];
 await byId('upload').onclick();assert.equal(byId('state').textContent,'Action failed');
 await byId('upload').onclick();assert.equal(byId('state').textContent,'File uploaded',byId('detail').textContent);
 assert.equal(appends.length,4);assert.equal(appends[0].request_id,appends[1].request_id);
 assert.equal(new Set(appends.map(x=>x.request_id)).size,3);
});
test('definite initial rejection permits corrected input; denial after uncertainty preserves intent',async()=>{
 const saved=storage(),journal=new IntentJournal(saved,'identity');
 const input={op:'send',conversation_id:'c',message:{to:'all',content_type:'text/plain',body:'too large',body_encoding:'utf8',label:'connect-messages',attachments:[],in_reply_to_msg:null,msg_id:null}};
 const denied=async()=>{throw Object.assign(Error('invalid'),{code:'invalid-request'});};
 await assert.rejects(journal.run({request:denied},'send:c',input));
 assert.deepEqual(journal.read(),{});
 await assert.rejects(journal.run({request:async()=>{throw Error('lost ack');}},'send:c',input));
 const pending=journal.read()['send:c'].operation.request_id;
 await assert.rejects(journal.retry({request:denied}));
 assert.equal(journal.read()['send:c'].operation.request_id,pending);
});

test('pairing uncertainty preserves its ID without persisting challenge inputs or results',async()=>{
 const saved=storage(),submitted=[],journal=new IntentJournal(saved,'identity');
 const input={op:'contact_pairing_complete',pairing_id:'pair',challenge:'private-one-use-challenge'};
 const client={request:async op=>{submitted.push(op);if(submitted.length===1)throw Error('lost ack');return {kind:'contact_pairing',data:{state:'verified_native',challenge:'private-result'}};}};
 await assert.rejects(journal.runPairing(client,'pair',input),/lost ack/);
 const record=JSON.stringify(journal.read());assert.ok(!record.includes(input.challenge));
 await assert.rejects(journal.runPairing(client,'pair',{...input,challenge:'changed'}),/original pairing/);
 const reloaded=new IntentJournal(saved,'identity');await reloaded.runPairing(client,'pair',input);
 assert.equal(submitted[0].request_id,submitted[1].request_id);
 assert.ok(!JSON.stringify(reloaded.read()).includes('private-result'));
 assert.equal(await reloaded.retry(client),null);
});


test('reference file verification rejects corrupted, cross-conversation and truncated bytes', async () => {
 const bytes=new Uint8Array(70000).map((_,i)=>i%251);
 const hash=Buffer.from(await webcrypto.subtle.digest('SHA-256',bytes)).toString('hex');
 const metadata={id:'file',conversation_id:'conversation',state:'ready',size:bytes.length,sha256:hash};
 const client={artifactGet:async(cid,id,offset,limit)=>({artifact:metadata,offset,
  data:Buffer.from(bytes.slice(offset,offset+limit)).toString('base64'),eof:offset+limit>=bytes.length})};
 const actual=await readArtifactBytes(client,'conversation','file');
 assert.deepEqual(actual.bytes,bytes);
 await assert.rejects(readArtifactBytes({...client,artifactGet:async(...args)=>({...await client.artifactGet(...args),artifact:{...metadata,conversation_id:'foreign'}})},'conversation','file'),/Invalid file response/);
 await assert.rejects(readArtifactBytes({...client,artifactGet:async(...args)=>({...await client.artifactGet(...args),data:'AA==',eof:true})},'conversation','file'),/Invalid file chunk/);
 await assert.rejects(readArtifactBytes({...client,artifactGet:async(...args)=>({...await client.artifactGet(...args),artifact:{...metadata,sha256:'0'.repeat(64)}})},'conversation','file'),/integrity/);
});

test('reference file download respects current denial without returning partial bytes', async () => {
 let attempts=0;
 const bytes=new Uint8Array(40000),metadata={id:'file',conversation_id:'conversation',state:'ready',size:bytes.length,sha256:'0'.repeat(64)};
 const client={artifactGet:async()=>{if(attempts++)throw Object.assign(Error('revoked'),{code:'forbidden'});
  return {artifact:metadata,offset:0,data:Buffer.from(bytes.slice(0,32768)).toString('base64'),eof:false};}};
 await assert.rejects(readArtifactBytes(client,'conversation','file'),/revoked/);
 assert.equal(attempts,2);
});


test('selected browser binding preserves the exact accepted Session and denies a retargeted response', async()=>{
    const calls=[];
    const client={browserBinding:async(...ids)=>{calls.push(ids);return {conversation_id:'c',session_id:'s',participant_id:'peer',signaling_url:'ws://127.0.0.1:1'};}};
    assert.equal((await configuredVoiceBinding(client,'c','peer','s')).session_id,'s');
    assert.deepEqual(calls,[['c','s']]);
    await assert.rejects(configuredVoiceBinding(client,'c','peer','different'),/configured for conversation/);
});


test('actual video UI cannot reactivate an ended session after delayed playback', async () => {
    const elements=new Map();
    const byId=id=>{
        if(!elements.has(id))elements.set(id,{value:'',disabled:false,textContent:'',srcObject:null,pause(){},append(){}});
        return elements.get(id);
    };
    let ended,attached,attachStarted,closed=0;
    const endedPromise=new Promise(resolve=>ended=resolve);
    const attachPromise=new Promise(resolve=>attached=resolve);
    const started=new Promise(resolve=>attachStarted=resolve);
    const current={ended:endedPromise,localPreview:{},attach:async()=>{attachStarted();await attachPromise;},close:async()=>{closed++;}};
    const client={sessionGet:async()=>({state:'accepted',media:['audio','video']})};
    const sandbox={document:{getElementById:byId},window:{addEventListener(){}},
        BrowserVideoSession:{open:async()=>current},act:async(_label,action)=>action(),
        configuredVoiceBinding:async()=>({conversation_id:'c',session_id:'s',signaling_url:'ws://127.0.0.1:1'}),
        needClient:()=>client,needConversation:()=>'c',value:id=>id==='session'?'s':'fixture',refresh:async()=>{},mutate:async()=>{}};
    const source=fs.readFileSync(new URL('../../examples/conversation-client/video.js',import.meta.url),'utf8')
        .replace(/^import[\s\S]*?from .*?;\n/gm,'');
    vm.createContext(sandbox);vm.runInContext(source,sandbox);
    const joining=byId('join-video').onclick();await started;
    ended();await Promise.resolve();
    attached();await assert.rejects(joining,/Video ended while playback/);
    assert.equal(closed,1);
    assert.equal(vm.runInContext('session',sandbox),undefined);
    assert.equal(byId('join-video').disabled,false);
    assert.equal(byId('end-video').disabled,true);
    assert.equal(byId('mute-camera').disabled,true);
    assert.equal(byId('mute-microphone').disabled,true);
    assert.equal(byId('local-video').srcObject,null);
    assert.equal(byId('remote-video').srcObject,null);
});
