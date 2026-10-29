import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,browserOperationKinds,sdkId,sdkRevision} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300},id=sdkId,rev=sdkRevision;
async function scenario(run,{path='wss-webrtc',algorithm='EdDSA',fault}={}){
 let mode=fault,revision=1;const records=new Map();let member={participant_id:'target',conversation_id:'room',identity_id:'recipient',kind:'human',role:'participant',membership_epoch:1,active:true,removal_reason:null};
 return fixture(async f=>{
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:path,resolveEnrollment:async()=>f.options,timeoutMs:500});const room=client.conversations.byId(id('room'));
  try{return await run({client,room,f,records,setMode:v=>mode=v});}finally{await client.close({timeoutMs:500});}
 },{algorithm,wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
  const scope={cid:frame.cid,operation_id:frame.operation_id};
  const reply=async(type,payload)=>{await peer.emit(type,payload,frame.id,scope);await peer.emit('ack',{},frame.id,scope);return true;};
  if(frame.type==='conversation.get')return reply('conversation.snapshot',{revision,cursor:'cursor-current',has_more:false,items:[{kind:'conversation',data:{conversation_id:'room',revision,closed:false,policy:'persistent'}},{kind:'participant',data:member}]});
  if(frame.type==='operation.get'){
   if(mode==='revoked')return peer.emit('error',{code:403,category:'auth',reason:'forbidden'},frame.id,scope).then(()=>true);
   const record=structuredClone(records.get(frame.payload.operation_id));if(mode==='record-command')record.command_type='conversation.participant.remove';if(mode==='record-epoch')record.result.data.participant.membership_epoch=0;return reply('operation.updated',record);
  }
  if(!frame.type.startsWith('conversation.participant.'))return false;
  if(mode==='revoked'||frame.expected_revision!==revision)return peer.emit('error',{code:mode==='revoked'?403:409,category:mode==='revoked'?'auth':'state',reason:mode==='revoked'?'forbidden':'revision_conflict'},frame.id,scope).then(()=>true);
  const action=frame.type.split('.').at(-1);revision++;
  member={...member,...(action==='add'?{identity_id:frame.payload.identity_id,kind:frame.payload.kind,role:frame.payload.role,membership_epoch:member.membership_epoch+1,active:true,removal_reason:null}:action==='update'?{role:frame.payload.role}:{active:false,removal_reason:null})};
  const result={kind:'membership',data:{kind:'participant',participant:structuredClone(member),revision,receipt:{operation_id:frame.operation_id,conversation_id:'room',state:'completed',completion_boundary:'local-mutation',status_reference:{action:'get',operation_id:frame.operation_id}}}};
  records.set(frame.operation_id,{operation_id:frame.operation_id,record_id:'record-'+frame.operation_id,command_type:frame.type,state:'completed',revision:1,result:structuredClone(result)});
  if(mode==='cid')result.data.participant.conversation_id='foreign';if(mode==='target')result.data.participant.participant_id='foreign';if(mode==='identity')result.data.participant.identity_id='foreign';if(mode==='receipt')result.data.receipt.operation_id='foreign';if(mode==='epoch')result.data.participant.membership_epoch=0;if(mode==='boundary')result.data.receipt.completion_boundary='provider-delivered';
  await peer.emit('operation.updated',{operation_id:frame.operation_id,state:'completed',revision:1,result},frame.id,scope);
  if(mode!=='notice-missing')await peer.emit('conversation.participant.changed',{kind:mode==='notice-kind'?'removed':action==='add'?'added':action==='update'?'updated':'removed',participant_id:member.participant_id,membership_epoch:mode==='notice-epoch'?99:member.membership_epoch},frame.id,mode==='notice-scope'?{...scope,sid:'foreign'}:scope);
  await peer.emit('ack',{},frame.id,scope);return true;
 }});
}
for(const path of ['webtransport','wss-webrtc'])for(const algorithm of ['EdDSA','ES256'])test(`Participant controls ${path}/${algorithm} capture intent and recover original scoped receipt views`,()=>scenario(async({client,room,f})=>{
 const input={identityId:id('recipient'),kind:'human',role:'reviewer',expectedRevision:rev(1)},pending=room.participants.add(input,{...opts,operationId:id('add')});input.identityId='changed';input.role='changed';const operation=await pending;assert.equal(f.sent.some(v=>v.type==='operation.get'),false);const added=(await client.operations.restore(operation.referenceJson(),browserOperationKinds.participantAdd).wait(opts)).value;assert.equal(added.id,'target');assert.equal(added.receiptSnapshot.identityId,'recipient');assert.equal(added.receiptSnapshot.role,'reviewer');assert.equal(added.receiptConversationRevision,2);assert(Object.isFrozen(added.receiptSnapshot));
 const updated=await added.update({role:'observer',expectedConversationRevision:rev(2)},{...opts,operationId:id('update')});assert.equal((await updated.wait(opts)).value.receiptSnapshot.role,'observer');assert.equal((await added.refresh(opts)).role,'observer');assert.equal(added.receiptSnapshot.role,'reviewer');
 const removed=await added.remove({expectedConversationRevision:rev(3)},{...opts,operationId:id('remove')});const absent=(await client.operations.recover(removed.reference).wait(opts)).value;assert.equal(absent.id,added.id);assert.equal(absent.receiptSnapshot.active,false);assert.equal(absent.receiptConversationRevision,4);assert.deepEqual(f.sent.filter(v=>v.type.startsWith('conversation.participant.')).map(v=>[v.type,v.expected_revision]),[['conversation.participant.add',1],['conversation.participant.update',2],['conversation.participant.remove',3]]);assert.equal(f.sent.find(v=>v.type==='conversation.participant.add').payload.identity_id,'recipient');
},{path,algorithm}));
for(const fault of ['cid','target','receipt','epoch','boundary','notice-missing','notice-kind','notice-epoch','notice-scope'])test('Participant mutation refuses '+fault+' acknowledgement with its original uncertain key',()=>scenario(async({room})=>{
 await assert.rejects(()=>room.participants.byId(id('target')).update({role:'observer',expectedConversationRevision:rev(1)},{...opts,operationId:id('original')}),{kind:'protocol',certainty:'uncertain',operation:{operationId:'original',command:'conversation.participant.update',scope:{conversationId:'room'}}});
},{fault}));
test('Participant add refuses a different acknowledged Identity',()=>scenario(async({room})=>{
 await assert.rejects(()=>room.participants.add({identityId:id('recipient'),kind:'human',role:'observer',expectedRevision:rev(1)},{...opts,operationId:id('original')}),{kind:'protocol',certainty:'uncertain'});
},{fault:'identity'}));
for(const fault of ['record-command','record-epoch','revoked'])test('original Participant recovery refuses '+fault+' without another mutation',()=>scenario(async({room,f,setMode})=>{
 const original=await room.participants.byId(id('target')).update({role:'observer',expectedConversationRevision:rev(1)},{...opts,operationId:id('original')});setMode(fault);await assert.rejects(()=>original.wait(opts),{kind:fault==='revoked'?'authentication':'protocol'});assert.equal(f.sent.filter(v=>v.type.startsWith('conversation.participant.')).length,1);
}));
test('invalid Participant shape, role, kind and revision refuse before IO',()=>scenario(async({room,f})=>{
 const n=f.sent.length;for(const input of [{identityId:'recipient',kind:'robot',role:'observer',expectedRevision:1},{identityId:'recipient',kind:'human',role:'x'.repeat(65),expectedRevision:1},{identityId:'recipient',kind:'human',role:'\ud800',expectedRevision:1},{identityId:'recipient',kind:'human',role:'a\nb',expectedRevision:1},{identityId:'recipient',kind:'human',role:'observer',expectedRevision:Number.MAX_SAFE_INTEGER},{identityId:'recipient',kind:'human',role:'observer',expectedRevision:1,tenant_id:'spoof'}])assert.throws(()=>room.participants.add(input,{...opts,operationId:id('original')}),{kind:'validation'});for(const input of [{role:'observer',expectedConversationRevision:-1},{role:'',expectedConversationRevision:1},{role:'observer',expectedConversationRevision:1,identityId:'other'}])assert.throws(()=>room.participants.byId(id('target')).update(input,{...opts,operationId:id('original')}),{kind:'validation'});assert.throws(()=>room.participants.byId(id('target')).remove({expectedConversationRevision:1,reason:'uncontracted'},{...opts,operationId:id('original')}),{kind:'validation'});assert.equal(f.sent.length,n);
}));
test('stale Conversation revision refuses with no fixture effect or automatic revision retry',()=>scenario(async({room,f,records})=>{
 await assert.rejects(()=>room.participants.byId(id('target')).remove({expectedConversationRevision:rev(0)},{...opts,operationId:id('stale')}));assert.equal(records.size,0);assert.equal(f.sent.filter(v=>v.type==='conversation.participant.remove').length,1);
}));
