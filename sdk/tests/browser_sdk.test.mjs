import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,BrowserSdkError,browserOperationKinds,sdkId,sdkRevision} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300},id=sdkId,one=first=>({first,rest:[]});
const reference=(command='session.invite',extra={})=>({schema:'connect-operation-reference/1',operation_id:'original',command,scope:{conversation_id:'room',session_id:'session'},...extra});
function scenario(run,{path='wss-webrtc',fault}={}) {
 let revision=0,state='invited',recordOverride,revisionOverride,mode=fault;const records=new Map();
 const view=(sid='session')=>({id:sid,conversation_id:'room',state,media:['data'],intent:'synthetic',capabilities_offer:{data:['reliable-ordered']},codec_preferences:[],is_main:true,revision,topology_revision:revision,created_at:1,updated_at:2,ended_reason:null});
 const items=()=>[{kind:'conversation',data:{conversation_id:'room',revision,closed:false,policy:'persistent'}},...['caller','callee'].map((pid,i)=>({kind:'participant',data:{participant_id:pid,conversation_id:'room',identity_id:i?'wss_identity':'other',kind:'human',role:'participant',membership_epoch:1,active:mode!=='removed'||i===0,removal_reason:null}})),{kind:'connection',data:{connection_id:mode==='foreign-connection'?'other':'connection',session_id:mode==='foreign-parent'?'other':'session',participant_id:'callee',membership_epoch:1,endpoint_id:'endpoint',device_id:'device',generation:1,revision:3,state:'ready',replaces:null}},
 {kind:'stream',data:{stream_id:'source',session_id:'session',participant_id:'callee',connection_id:'connection',medium:'video',state:'active',revision:7}},
 ...(mode==='scoped-streams'?[{kind:'connection',data:{connection_id:'subscriber',session_id:'session',participant_id:'caller',membership_epoch:1,endpoint_id:'endpoint',device_id:'device',generation:1,revision:4,state:'ready',replaces:null}}]:[]),
 ...(mode==='scoped-streams'||mode==='duplicate-stream'?[{kind:'stream',data:{stream_id:'source',session_id:'session',participant_id:'caller',connection_id:mode==='duplicate-stream'?'connection':'subscriber',medium:'video',state:'paused',revision:9}}]:[]),
 {kind:'session',data:view()},{kind:'session_participant',data:{session_id:'session',participant_id:'callee',membership_epoch:1,role:'participant',state:state==='invited'?'invited':'present'}}];
 return fixture(async f=>{
  const client=await BrowserSdkClient.connect({identityId:f.options.identityId,deviceId:f.options.device.id,tenantId:'tnt_default',endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:path,timeoutMs:500,attemptTimeoutMs:100,resolveEnrollment:async()=>f.options});
  const peer=()=>path==='webtransport'?f.transports.at(-1):f.socket();
  try{return await run({f,client,records,peer,view,setMode:v=>mode=v,setRecord:v=>recordOverride=v,setRevision:v=>revisionOverride=v});}finally{client.close();}
 },{wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
  const reply=async(type,payload,scope=frame)=>{await peer.emit(type,payload,frame.id,scope);await peer.emit('ack',{},frame.id,frame);return true;};
  if(frame.type==='conversation.get'){
   if(mode==='revoked')return peer.emit('error',{code:401,category:'auth',reason:'authentication_failed'},frame.id,frame).then(()=>true);
   return reply('conversation.snapshot',{revision:mode==='changed-page'&&frame.payload.page?revision+1:revision,cursor:'cursor-current',items:items(),has_more:mode==='changed-page'&&!frame.payload.page,...(mode==='changed-page'&&!frame.payload.page?{next_page:'next-page'}:{})});
  }
  if(frame.type==='operation.get'){
   const r=records.get(frame.payload.operation_id)??{operation_id:frame.payload.operation_id,command_type:'session.invite',state:mode==='pending'?'unknown':'completed',revision:1,result:{kind:'session_control',data:{operation_id:frame.payload.operation_id,action:'session.invite',session:view()}}};
   return reply('operation.updated',{...r,record_id:recordOverride??'record-'+frame.payload.operation_id,revision:revisionOverride??r.revision});
  }
  if(frame.type==='connection.end'){
   const r={operation_id:frame.operation_id,command_type:frame.type,state:'completed',revision:1,result:{ended:true,connection_id:mode==='foreign-end'?'other':frame.connid}};records.set(frame.operation_id,r);return reply('operation.updated',r);
  }
  if(frame.type.startsWith('session.')){
   let r=records.get(frame.operation_id);if(!r){revision++;state=frame.type==='session.reject'?'rejected':frame.type==='session.accept'?'accepted':'invited';
    const result=frame.type==='session.reject'?{kind:'session_decision',data:{operation_id:frame.operation_id,conversation_id:'room',session_id:frame.sid,state:'rejected',revision,reason_code:486,reason:'Unavailable'}}:{kind:'session_control',data:{operation_id:frame.operation_id,action:frame.type==='session.update'?frame.payload.kind:frame.type,session:{...view(frame.sid),...(mode==='foreign-result'?{conversation_id:'foreign'}:{})}}};
    r={operation_id:frame.operation_id,command_type:frame.type,state:'completed',revision:1,result};records.set(frame.operation_id,r);
   }return reply('operation.updated',{operation_id:mode==='foreign-ack'?'foreign':r.operation_id,state:r.state,revision:r.revision,result:r.result});
  }
  if(frame.type==='conversation.create')return reply('ack',{details:{reply_complete:true,conversation:{id:'room',receipt:{operation_id:frame.operation_id}}}});
  return false;
 }});
}
for(const path of ['webtransport','wss-webrtc'])test(`typed browser ${path} mutations return observable operations and restore status without resubmitting`,()=>scenario(async({f,client})=>{
 const room=client.conversations.byId(id('room')),operation=await room.sessions.invite({sessionId:id('session'),to:[id('callee')],medium:'text-chat',intent:'synthetic',capabilities:{data:['reliable-ordered']}},{...opts,operationId:id('original')});
 assert.equal(operation.reference.operationId,'original');assert.equal(f.sent.filter(x=>x.type==='operation.get').length,0);
 const restored=client.operations.restore(operation.referenceJson(),browserOperationKinds.sessionInvite);const completion=await restored.wait(opts);assert.equal(completion.value.id,'session');assert.equal(completion.value.conversationId,'room');assert.equal(completion.snapshot.state,'completed');assert.equal(completion.value.receiptSnapshot.state,'invited');
 assert.equal((await completion.value.refresh(opts)).id,'session');assert.equal(f.sent.filter(x=>x.type==='session.invite').length,1);assert.throws(()=>restored.reference.scope.sessionId='foreign');
},{path}));
for(const action of ['accept','reject'])test(`typed incoming invitation supplies scoped Session for ${action} after current-authorized refresh`,()=>scenario(async({f,client,peer})=>{
 const incoming=client.incomingInvitations(opts);await peer().emit('session.invite',{from:'caller',to:['callee'],medium:'text-chat',intent:'synthetic',capabilities_offer:{data:['reliable-ordered']}},undefined,{cid:'room',sid:'session',operation_id:'original-invite'});
 const hint=(await incoming.next()).value;assert.equal(hint.session.id,'session');assert.equal(hint.session.conversationId,'room');assert.equal(hint.to,'callee');assert.equal(hint.source.correlationId,'original-invite');assert.throws(()=>hint.capabilities.data.push('foreign'));
 assert.equal(f.sent.filter(v=>v.type==='conversation.get').length,1);const current=await hint.session.refresh(opts);
 const operation=action==='accept'?await hint.session.accept({capabilities:{data:['reliable-ordered']},expectedTopologyRevision:current.topologyRevision},{...opts,operationId:id('decision')}):await hint.session.reject({reasonCode:486,reason:'Unavailable',expectedTopologyRevision:current.topologyRevision},{...opts,operationId:id('decision')});
 const result=await operation.wait(opts);assert.equal(action==='accept'?result.value.id:result.value.session.id,'session');assert.equal(f.sent.filter(v=>v.type==='session.'+action).length,1);await incoming.close();assert.deepEqual(await incoming.next(),{done:true,value:undefined});
}));
test('operation references refuse foreign kinds, scopes and unsupported continuation without IO',()=>scenario(async({f,client})=>{
 const n=f.sent.length;
 for(const value of [reference('session.accept'),reference('session.invite',{extra:'x'}),reference('session.invite',{scope:{conversation_id:'room'}}),reference('session.invite',{scope:{conversation_id:'room',session_id:'session',connection_id:'foreign'}})])assert.throws(()=>client.operations.restore(value,browserOperationKinds.sessionInvite),e=>e instanceof BrowserSdkError&&e.kind==='validation');
 assert.throws(()=>client.operations.recover({operationId:'original',command:'session.invite',scope:{conversationId:'room',sessionId:'session',connectionId:'foreign'}}),{kind:'validation'});assert.equal(f.sent.length,n);
}));
for(const fault of ['record','revision'])test(`operation ${fault} rollback/rebinding refuses and retires its physical`,()=>scenario(async({client,setRecord,setRevision})=>{
 const op=client.operations.restore(reference(),browserOperationKinds.sessionInvite);await op.status(opts);if(fault==='record')setRecord('foreign-record');else setRevision(0);await assert.rejects(()=>op.status(opts),{kind:'protocol'});assert.equal(client.transport.state,'disconnected');
}));
test('cancelled operation wait preserves original reference and never dispatches cancellation/mutation',()=>scenario(async({client,f})=>{
 const op=client.operations.restore(reference(),browserOperationKinds.sessionInvite),abort=new AbortController();const pending=op.wait({timeoutMs:200,signal:abort.signal});setTimeout(()=>abort.abort(),10);await assert.rejects(()=>pending,{kind:'cancelled'});assert.equal(op.reference.operationId,'original');assert.equal(f.sent.filter(v=>!v.type.startsWith('auth.')&&v.type!=='operation.get').length,0);
},{fault:'pending'}));
test('snapshot pagination cannot combine different authorized revisions',()=>scenario(async({client,f})=>{
 await assert.rejects(()=>client.conversations.byId(id('room')).refresh(opts),{kind:'resync-required'});assert.equal(f.sent.filter(v=>v.type==='conversation.get').length,2);
},{fault:'changed-page'}));
test('completed foreign result is not converted into a Session handle',()=>scenario(async({client})=>{
 const op=await client.conversations.byId(id('room')).sessions.invite({sessionId:id('session'),to:[],medium:'text-chat',intent:'synthetic',capabilities:{}},{...opts,operationId:id('original')});await assert.rejects(()=>op.wait(opts),{kind:'protocol',certainty:'recorded',operation:{operationId:'original',command:'session.invite',scope:{conversationId:'room',sessionId:'session'}}});
},{fault:'foreign-result'}));
for(const mode of ['removed','revoked'])test(`queued invitation does not bypass ${mode} current authorization`,()=>scenario(async({client,peer,f,setMode})=>{
 const incoming=client.incomingInvitations({timeoutMs:50});await peer().emit('session.invite',{from:'caller',to:['callee'],medium:'text-chat',intent:'synthetic',capabilities_offer:{}},undefined,{cid:'room',sid:'session',operation_id:'original-invite'});setMode(mode);await assert.rejects(()=>incoming.next(),{kind:mode==='removed'?'deadline':'authentication'});assert.equal(f.sent.some(v=>v.type==='session.accept'||v.type==='session.reject'),false);await incoming.close();
}));
test('closing/overlapping invitation waits stay bounded and issue no decisions',()=>scenario(async({client,f})=>{
 const incoming=client.incomingInvitations(opts),pending=incoming.next();await assert.rejects(()=>incoming.next(),{kind:'conflict'});await incoming.close();assert.deepEqual(await pending,{done:true,value:undefined});assert.equal(f.sent.filter(v=>v.type.startsWith('session.')).length,0);
}));

test('uncertain malformed mutation response retains the submitted original operation key',()=>scenario(async({client,f})=>{
 await assert.rejects(()=>client.conversations.byId(id('room')).sessions.invite({sessionId:id('session'),to:[id('callee')],medium:'text-chat',intent:'synthetic',capabilities:{}},{...opts,operationId:id('original')}),{kind:'protocol',certainty:'uncertain',operation:{operationId:'original',command:'session.invite',scope:{conversationId:'room',sessionId:'session'}}});
 assert.equal(f.sent.filter(f=>f.type==='session.invite').length,1);
},{fault:'foreign-ack'}));
test('recover keeps the typed result and reads only the original operation',()=>scenario(async({client,f})=>{
 const restored=client.operations.restore(reference(),browserOperationKinds.sessionInvite),operation=client.operations.recover(restored.reference);
 assert.equal((await operation.wait(opts)).value.id,'session');assert.equal(f.sent.filter(f=>f.type==='session.invite').length,0);
}));
for(const forbidden of [{connid:'foreign'},{in_reply_to:'unknown'},{expected_revision:0}])test('incoming invitation refuses foreign scope '+Object.keys(forbidden)[0],()=>scenario(async({client,peer})=>{
 const incoming=client.incomingInvitations(opts);await peer().emit('session.invite',{from:'caller',to:['callee'],medium:'text-chat',intent:'synthetic',capabilities_offer:{}},forbidden.in_reply_to,{cid:'room',sid:'session',operation_id:'original-invite',...forbidden});
 await assert.rejects(()=>incoming.next(),{kind:'protocol'});await incoming.close();
}));

test('typed Connection and Stream refreshes remain current authorized observations with separate revision families',()=>scenario(async({client,f})=>{
 const connection=client.conversations.byId(id('room')).sessions.byId(id('session')).connections.byId(id('connection'));
 const snapshot=await connection.refresh(opts);assert.equal(snapshot.resourceRevision,3);assert.equal(snapshot.streamRevision,undefined);
 const streams=await connection.streams.list(opts);assert.equal(streams.length,1);const observed=await streams[0].refresh(opts);assert.equal(observed.connectionId,'connection');assert.equal(observed.resourceRevision,7);assert.equal(observed.ownership,'observation');
 await assert.rejects(()=>connection.capture({video:'camera'},opts),{kind:'receipt-unavailable'});assert.equal(f.sent.some(v=>v.type==='connection.offer'),false);
 const op=await connection.end({reasonCode:200,reason:'Finished',expectedStreamRevision:sdkRevision(1)},{...opts,operationId:id('end-original')});
 assert.deepEqual(op.reference.scope,{conversationId:'room',sessionId:'session',connectionId:'connection'});assert.equal((await client.operations.recover(op.reference).wait(opts)).value.id,'connection');assert.equal(f.sent.filter(v=>v.type==='connection.end').length,1);
}));
for(const fault of ['foreign-connection','foreign-parent'])test('Connection current parent mismatch refuses '+fault,()=>scenario(async({client})=>{
 await assert.rejects(()=>client.conversations.byId(id('room')).sessions.byId(id('session')).connections.byId(id('connection')).refresh(opts),{kind:'receipt-unavailable'});
},{fault}));
test('foreign completed Connection end cannot retire another local owner',()=>scenario(async({client})=>{
 const connection=client.conversations.byId(id('room')).sessions.byId(id('session')).connections.byId(id('connection'));
 const op=await connection.end({reasonCode:200,reason:'Finished'},{...opts,operationId:id('end-original')});await assert.rejects(()=>op.wait(opts),{kind:'protocol',certainty:'recorded'});
},{fault:'foreign-end'}));
test('Stream identity never escapes its current Connection/Session parent',()=>scenario(async({client,f})=>{
 const connection=client.conversations.byId(id('room')).sessions.byId(id('session')).connections.byId(id('foreign'));
 await assert.rejects(()=>connection.streams.byId(id('source')).refresh(opts),{kind:'receipt-unavailable'});assert.equal(f.sent.some(v=>v.type.startsWith('connection.')),false);
}));

test('typed Connection offer rejects invalid or unsupported intent before IO without downgrading codecs or directions',()=>scenario(async({client,f})=>{
 const session=client.conversations.byId(id('room')).sessions.byId(id('session')),n=f.sent.length;
 for(const streams of [[],[null],[{id:'source',kind:'audio',direction:'wrong',codecs:['opus']}],[{id:'source',kind:'audio',direction:'sendrecv',codecs:[]}]])await assert.rejects(()=>session.connections.offer({connectionId:id('connection'),streams},{...opts,operationId:id('original')}),{kind:'validation',certainty:'not-submitted'});
 for(const stream of [{id:'source',kind:'audio',direction:'sendrecv',codecs:['pcmu']},{id:'source',kind:'video',direction:'sendonly',codecs:['vp8']}])await assert.rejects(()=>session.connections.offer({connectionId:id('connection'),streams:one({...stream,codecs:one(stream.codecs[0])})},{...opts,operationId:id('original')}),{kind:'unsupported',certainty:'not-submitted'});
 await assert.rejects(()=>session.connections.offer({connectionId:id('connection'),streams:one({id:'source',kind:'data',direction:'sendrecv',codecs:one('binary')})},{...opts,operationId:id('original')}),{kind:'unsupported',certainty:'not-submitted',operation:{operationId:'original',command:'connection.offer',scope:{conversationId:'room',sessionId:'session',connectionId:'connection'}}});
 assert.equal(f.sent.length,n);
}));
test('historical Connection cannot render or prompt for capture and foreign Stream handles refuse locally',()=>scenario(async({client,f})=>{
 const session=client.conversations.byId(id('room')).sessions.byId(id('session')),connection=session.connections.byId(id('connection')),n=f.sent.length;
 await assert.rejects(()=>connection.render({audio:false},opts),{kind:'receipt-unavailable'});
 await assert.rejects(()=>connection.render({audio:false,video:[{stream:session.connections.byId(id('foreign')).streams.byId(id('source')),canvas:{}}]},opts),{kind:'validation'});
 await assert.rejects(()=>connection.capture({audio:true},opts),{kind:'receipt-unavailable'});assert.equal(f.sent.length,n);
}));

test('same logical Stream is observed independently on publisher and subscriber Connections',()=>scenario(async({client})=>{
 const connections=client.conversations.byId(id('room')).sessions.byId(id('session')).connections;
 const publisher=connections.byId(id('connection')),subscriber=connections.byId(id('subscriber'));
 assert.equal((await publisher.streams.list(opts)).length,1);assert.equal((await subscriber.streams.list(opts)).length,1);
 assert.equal((await publisher.streams.byId(id('source')).refresh(opts)).resourceRevision,7);
 const selected=await subscriber.streams.byId(id('source')).refresh(opts);assert.equal(selected.connectionId,'subscriber');assert.equal(selected.state,'paused');assert.equal(selected.resourceRevision,9);
},{fault:'scoped-streams'}));
test('duplicate logical Stream within one Connection remains a protocol failure',()=>scenario(async({client})=>{
 await assert.rejects(()=>client.conversations.byId(id('room')).sessions.byId(id('session')).connections.byId(id('connection')).streams.list(opts),{kind:'protocol'});
},{fault:'duplicate-stream'}));

for(const path of ['webtransport','wss-webrtc']) {
 for(const input of [{action:'participant-leave',participantId:id('callee')},{action:'role-change',participantId:id('callee'),role:'observer'},{action:'medium-upgrade',media:{first:'audio',rest:['screen']}},{action:'codec-renegotiate',codecPreferences:{first:'opus',rest:['vp8']}}])test(`typed browser ${path} Session ${input.action} captures original intent and restores the original scoped outcome`,()=>scenario(async({client,f})=>{
  const session=client.conversations.byId(id('room')).sessions.byId(id('session'));
  const body=structuredClone(input),operation=await session.update(body,sdkRevision(0),{...opts,operationId:id('session-update')});
  if(body.codecPreferences)body.codecPreferences.first='changed';if(body.media)body.media.first='video';
  assert.equal(f.sent.filter(v=>v.type==='operation.get').length,0);assert.equal(operation.reference.command,'session.update');
  const restored=client.operations.restore(operation.referenceJson(),browserOperationKinds.sessionUpdate),completed=await restored.wait(opts);
  assert.equal(completed.value.id,'session');assert.equal(completed.value.receiptSnapshot.topologyRevision,1);assert.equal(f.sent.filter(v=>v.type==='session.update').length,1);
  const wire=f.sent.find(v=>v.type==='session.update');assert.equal(wire.expected_revision,0);
  const expected=input.action==='participant-leave'?{action:input.action,participant_id:'callee'}:input.action==='role-change'?{action:input.action,participant_id:'callee',role:'observer'}:input.action==='medium-upgrade'?{action:input.action,media:['audio','screen']}:{action:input.action,codec_preferences:['opus','vp8']};assert.deepEqual(JSON.parse(JSON.stringify(wire.payload)),{kind:expected.action,details:Object.fromEntries(Object.entries(expected).filter(([key])=>key!=='action'))});
 },{path}));
 test(`typed browser ${path} existing Session invitation retains scope, NonEmpty targets and explicit topology revision`,()=>scenario(async({client,f})=>{
  const session=client.conversations.byId(id('room')).sessions.byId(id('session')),input={to:one(id('callee')),medium:'mixed',intent:'retry',capabilities:{audio:['opus']},expectedTopologyRevision:sdkRevision(0)};
  const pending=session.inviteParticipants(input,{...opts,operationId:id('reinvite')});input.to.first=id('changed');input.capabilities.audio[0]='changed';
  const operation=await pending,result=(await operation.wait(opts)).value;assert.equal(result.id,session.id);assert.equal(operation.reference.command,'session.invite');
  const wire=f.sent.find(v=>v.type==='session.invite');assert.equal(wire.sid,session.id);assert.equal(wire.expected_revision,0);assert.deepEqual(JSON.parse(JSON.stringify(wire.payload)),{to:['callee'],medium:'mixed',intent:'retry',capabilities_offer:{audio:['opus']}});
 },{path}));
}
test('Session updates and existing invitations reject ambiguous/empty/duplicate/oversize input before any IO',()=>scenario(async({client,f})=>{
 const session=client.conversations.byId(id('room')).sessions.byId(id('session')),n=f.sent.length,mutation={...opts,operationId:id('original')};
 for(const input of [{action:'participant-leave',participantId:'callee',role:'ignored'},{action:'role-change',participantId:'callee',role:'x'.repeat(65)},{action:'medium-upgrade',media:{first:'audio',rest:['audio']}},{action:'medium-upgrade',media:{first:'bogus',rest:[]}},{action:'codec-renegotiate',codecPreferences:{first:'opus',rest:['opus']}},{action:'codec-renegotiate',codecPreferences:{first:'opus',rest:Array.from({length:16},(_,i)=>'c'+i)}}])assert.throws(()=>session.update(input,sdkRevision(0),mutation),{kind:'validation'});
 for(const input of [{to:[],medium:'voice',intent:'test',capabilities:{}},{to:one('callee'),medium:'voice',intent:'x'.repeat(129),capabilities:{}},{to:one('callee'),medium:'voice',intent:'test',capabilities:{audio:[]}},{to:{first:'callee',rest:['callee']},medium:'voice',intent:'test',capabilities:{}}])assert.throws(()=>session.inviteParticipants({...input,expectedTopologyRevision:sdkRevision(0)},mutation),{kind:'validation'});
 assert.equal(f.sent.length,n);
}));

for(const path of ['webtransport','wss-webrtc'])test(`typed browser ${path} Conversation creation uses the frozen Identity collection`,()=>scenario(async({client,f})=>{
 const input={policy:'persistent',participants:[id('recipient')]},pending=client.conversations.create(input,{...opts,operationId:id('create')});input.participants[0]=id('changed');const operation=await pending;assert.equal(operation.reference.command,'create');assert.deepEqual(JSON.parse(JSON.stringify(f.sent.find(v=>v.type==='conversation.create').payload)).initial_participants,[{identity_id:'recipient',role:'participant'}]);
 const n=f.sent.length;assert.throws(()=>client.conversations.create({policy:'persistent',participants:[{identityId:'recipient',role:'participant'}]},{...opts,operationId:id('invalid')}),{kind:'validation'});assert.equal(f.sent.length,n);
},{path}));
