import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,browserOperationKinds,browserCursor,sdkId} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300},id=sdkId,one=first=>({first,rest:[]});
const accepted=(op,mid,cid='room')=>({kind:'accepted',data:{request_id:op,message_id:mid,conversation_id:cid,receipt:{operation_id:op,conversation_id:cid,state:'completed',completion_boundary:'local-mutation',status_reference:{action:'get',operation_id:op}}}});
const fact=(n=1,extra={})=>({event_id:'fact-'+n,cursor:'cursor-'+n,conversation_id:'room',session_id:null,connection_id:null,operation_id:'send-'+n,participant_id:'source',kind:'message',revision:n,source:{kind:'message',correlation_id:'message-'+n},observed_at:100+n,audience:[{participant_id:'source',membership_epoch:1},{participant_id:'recipient',membership_epoch:1}],data:{kind:'message',data:{id:'message-'+n,conversation_id:'room',body:'synthetic-'+n}},...extra});
async function scenario(run,{path='wss-webrtc',fault,algorithm='EdDSA'}={}){
 let mode=fault,gate;const records=new Map(),journal=[];
 return fixture(async f=>{
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:path,resolveEnrollment:async()=>f.options,timeoutMs:500});const room=client.conversations.byId(id('room'));
  try{return await run({client,room,f,records,journal,setMode:v=>mode=v,setGate:v=>gate=v});}finally{await client.close({timeoutMs:500});}
 },{algorithm,wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
  const reply=async(type,payload,scope=frame)=>{await peer.emit(type,payload,frame.id,scope);await peer.emit('ack',{},frame.id,frame);return true;};
  if(frame.type==='message.send'){
   const value=accepted(frame.operation_id,frame.payload.msg_id);records.set(frame.operation_id,{operation_id:frame.operation_id,record_id:'record-'+frame.operation_id,command_type:'send',state:'completed',revision:1,result:structuredClone(value)});
   if(mode==='ack-cid')value.data.conversation_id='foreign';if(mode==='ack-mid')value.data.message_id='foreign';if(mode==='boundary')value.data.receipt.completion_boundary='provider-delivered';if(mode==='status-reference')value.data.receipt.status_reference.operation_id='foreign';
   return reply('ack',{details:value},mode==='ack-scope'?{...frame,sid:'foreign'}:frame);
  }
  if(frame.type==='operation.get')return reply('operation.updated',{...records.get(frame.payload.operation_id),...(mode==='record-command'?{command_type:'message.send'}:{})});
  if(frame.type==='conversation.get')return reply('conversation.snapshot',{revision:0,cursor:'cursor-0',items:[{kind:'conversation',data:{conversation_id:'room',revision:0,closed:false,policy:'persistent'}}],has_more:false});
  if(frame.type==='conversation.subscribe'){
   if(mode==='revoked')return peer.emit('error',{code:403,category:'auth',reason:'forbidden'},frame.id,frame).then(()=>true);
   const position=Number(frame.payload.after.split('-').at(-1));const selected=journal.find(v=>Number(v.cursor.split('-').at(-1))>position);const captured=selected?structuredClone(selected):undefined;if(gate)await gate(frame);
   if(mode==='resync'){await peer.emit('conversation.snapshot',{revision:1,cursor:'resync-cursor',items:[],has_more:false},frame.id,frame);await peer.emit('ack',{details:{cursor:'resync-cursor',has_more:false,resync_required:true,live:false}},frame.id,frame);return true;}
   if(captured){if(mode==='event-cid')captured.conversation_id='foreign';if(mode==='event-source')captured.source.correlation_id='';if(mode==='event-audience')captured.audience.push(captured.audience[0]);if(mode==='event-nested')captured.data.data.conversation_id='foreign';await peer.emit('conversation.event',{event_id:captured.event_id,cursor:captured.cursor,kind:captured.kind,revision:captured.revision,data:captured},frame.id,{...frame,...(captured.session_id?{sid:captured.session_id}:{}),...(captured.connection_id?{connid:captured.connection_id}:{}),...(captured.operation_id?{operation_id:captured.operation_id}:{})});}
   await peer.emit('ack',{details:{cursor:captured?'summary-'+captured.cursor:frame.payload.after,has_more:false,resync_required:false,live:false}},frame.id,frame);return true;
  }
  return false;
 }});
}
for(const path of ['webtransport','wss-webrtc'])for(const algorithm of ['EdDSA','ES256'])test(`typed private message ${path}/${algorithm} captures original IDs and restores acceptance without resubmission`,()=>scenario(async({client,room,f})=>{
 const input={messageId:id('message'),to:one(id('recipient')),text:'hello 👋'},pending=room.send(input,{...opts,operationId:id('original')});input.text='changed';input.to.first=id('other');const operation=await pending;assert.equal(f.sent.filter(v=>v.type==='operation.get').length,0);assert.deepEqual(operation.reference.scope,{conversationId:'room'});assert.equal(operation.reference.command,'message.send');const saved=operation.referenceJson(),value=(await client.operations.restore(saved,browserOperationKinds.messageSend).wait(opts)).value;assert.equal(value.messageId,'message');assert.equal(value.boundary,'local-mutation');assert.equal(value.operation.operationId,'original');assert(Object.isFrozen(value));assert.equal(f.sent.filter(v=>v.type==='message.send').length,1);assert.deepEqual(JSON.parse(JSON.stringify(f.sent.find(v=>v.type==='message.send').payload)),{msg_id:'message',to:['recipient'],body:'hello 👋',body_encoding:'utf8',content_type:'text/plain',label:'connect-messages',attachments:[],in_reply_to_msg:null});
},{path,algorithm}));
for(const fault of ['ack-cid','ack-mid','boundary','status-reference','ack-scope'])test('typed message refuses '+fault+' local mutation acknowledgement and retains its original uncertain key',()=>scenario(async({room,f})=>{
 await assert.rejects(()=>room.send({messageId:id('message'),to:one(id('recipient')),text:'synthetic'},{...opts,operationId:id('original')}),{kind:'protocol',certainty:'uncertain',operation:{operationId:'original',command:'message.send',scope:{conversationId:'room'}}});assert.equal(f.sent.filter(v=>v.type==='message.send').length,1);
},{fault}));
test('typed message record uses the exact current Store command rather than permitting a rebound ledger type',()=>scenario(async({room,setMode})=>{
 const op=await room.send({messageId:id('message'),to:one(id('recipient')),text:'synthetic'},{...opts,operationId:id('original')});setMode('record-command');await assert.rejects(()=>op.wait(opts),{kind:'protocol'});
}));
test('private message validates frozen shape, duplicate recipients, UTF8 and full text bound before IO',()=>scenario(async({room,f})=>{
 const n=f.sent.length;for(const input of [{messageId:'m',to:[] ,text:'x'},{messageId:'m',to:{first:'r',rest:['r']},text:'x'},{messageId:'m',to:one('r'),text:'x',from:'spoof'},{messageId:'m',to:one('r'),text:'x'.repeat(512*1024+1)},{messageId:'m',to:one('r'),text:'\ud800'}])assert.throws(()=>room.send(input,{...opts,operationId:id('original')}),{kind:'validation'});assert.equal(f.sent.length,n);
 const op=await room.send({messageId:id('large'),to:one(id('recipient')),text:'x'.repeat(512*1024)},{...opts,operationId:id('large-original')});assert.equal((await op.wait(opts)).value.messageId,'large');
}));
for(const path of ['webtransport','wss-webrtc'])test(`typed journal ${path} starts at an authorized baseline and resumes after the caller's delivered cursor`,()=>scenario(async({room,journal,f})=>{
 const reader=room.events({},opts),initial=await reader.ready();assert.equal(initial,'cursor-0');journal.push(fact(1),fact(2));const first=(await reader.next()).value;assert.equal(first.eventId,'fact-1');assert.equal(first.conversationId,'room');assert.equal(first.operationId,'send-1');assert.equal(first.source.correlationId,'message-1');assert(Object.isFrozen(first.data.data));await reader.close();const resumed=room.events({after:first.cursor},opts);assert.equal((await resumed.next()).value.eventId,'fact-2');assert(f.sent.filter(v=>v.type==='conversation.subscribe').every(v=>v.payload.live===false&&v.payload.limit===1));await resumed.close();
},{path}));
for(const fault of ['event-cid','event-source','event-audience','event-nested'])test('typed journal refuses '+fault+' signed fact scope/shape',()=>scenario(async({room,journal,setMode})=>{
 const reader=room.events({},opts);await reader.ready();journal.push(fact(1));setMode(fault);await assert.rejects(()=>reader.next(),{kind:'protocol'});await reader.close();
}));
test('journal resync stays explicit and never adopts a replacement snapshot cursor',()=>scenario(async({room,setMode})=>{
 const reader=room.events({after:browserCursor('original-cursor')},opts);setMode('resync');await assert.rejects(()=>reader.next(),{kind:'resync-required'});await reader.close();
}));
for(const path of ['webtransport','wss-webrtc'])test(`journal local timeout ${path} retains its position and rechecks current authority after an abandoned read`,()=>scenario(async({room,journal,setGate,setMode,f})=>{
 const reader=room.events({}, {timeoutMs:25});await reader.ready();journal.push(fact(1));let release;const held=new Promise(r=>release=r);setGate(()=>held);await assert.rejects(()=>reader.next(),{kind:'deadline'});setGate(undefined);setMode('revoked');release();await assert.rejects(()=>reader.next(),{kind:'authentication'});setMode(undefined);assert.equal((await reader.next()).value.eventId,'fact-1');assert(f.sent.filter(v=>v.type==='conversation.subscribe').every(v=>v.payload.after==='cursor-0'));await reader.close();
},{path}));
test('event readers bound retained requests, reject overlapping waits and close without consuming a fact',()=>scenario(async({room,journal,setGate,f})=>{
 const readers=Array.from({length:8},()=>room.events({},opts));assert.throws(()=>room.events({},opts),{kind:'conflict'});for(const r of readers)await r.close();const reader=room.events({},opts);await reader.ready();journal.push(fact(1));let release,seen=false;const held=new Promise(r=>release=r);setGate(()=>{seen=true;return held;});const pending=reader.next();while(!seen)await new Promise(r=>setTimeout(r,1));await assert.rejects(()=>reader.next(),{kind:'conflict'});await reader.close();assert.deepEqual(await pending,{done:true,value:undefined});setGate(undefined);release();const fresh=room.events({after:browserCursor('cursor-0')},opts);assert.equal((await fresh.next()).value.eventId,'fact-1');await fresh.close();assert.equal(f.sent.filter(v=>v.type==='message.send').length,0);
}));
test('event readers from a retired physical cannot follow a replacement implicitly',()=>scenario(async({client,room})=>{
 const reader=room.events({},opts);await reader.ready();await client.reconnect({preferredTransport:'wss-webrtc',timeoutMs:500});await assert.rejects(()=>reader.next(),{kind:'cancelled'});await reader.close();
}));

test('journal retains a full512KiB accepted message within the canonical1MiB frame bound',()=>scenario(async({room,journal})=>{
 const reader=room.events({},opts);await reader.ready();const body='x'.repeat(512*1024);journal.push(fact(1,{data:{kind:'message',data:{id:'message-1',conversation_id:'room',body}}}));assert.equal((await reader.next()).value.data.data.body,body);await reader.close();
}));
