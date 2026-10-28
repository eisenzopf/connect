#!/usr/bin/env node
// Synthetic SDK helper framing/process fixture. No native verification claim.
import {readFileSync,writeFileSync,appendFileSync} from 'node:fs';
import {dirname,join} from 'node:path';
import {createInterface} from 'node:readline';
// Trusted command/reference parser simulation only; Rust qualification is separate.
if (process.argv[2] === 'sdk-validate-provisioning') {
 const path=process.env.CONNECT_SDK_FIXTURE_CONFIG,config=JSON.parse(readFileSync(path,'utf8')),dir=dirname(path),value=JSON.parse(readFileSync(0,'utf8')),ref=process.argv.includes('--reference');
 appendFileSync(join(dir,'admin-validation.jsonl'),JSON.stringify({argv:process.argv.slice(2),pid:process.pid,value})+'\n',{mode:0o600});
 const names={identity:'identity.provision',participant:'participant.provision',endpoint:'endpoint.provision',endpoint_verify:'endpoint.verify',endpoint_revoke:'endpoint.revoke',identity_revoke:'identity.revoke',record_read_role:'record-read-role.configure',signing_key:'signing-key.enroll',signing_key_revoke:'signing-key.revoke',device_revoke:'device.revoke',credential_revoke:'credential.revoke'};
 const allowed=[...Object.values(names),'operation.cancel','operation.reconcile'];
 let valid=typeof value?.operation_id==='string'&&/^[A-Za-z0-9._~-]{1,128}$/.test(value.operation_id);
 if(ref)valid&&=value.schema==='connect-operation-reference/1'&&allowed.includes(value.command)&&value.scope&&Object.keys(value.scope).length===0&&Object.keys(value).every(k=>['schema','command','scope','operation_id'].includes(k));
 else valid&&=Object.keys(names).includes(value.type);
 const normalized={...value};if(!ref&&typeof normalized.alias==='string')normalized.alias=normalized.alias.trim();
 const saved=ref?value:{schema:'connect-operation-reference/1',operation_id:value?.operation_id,command:names[value?.type],scope:{}};
 let result=valid?{profile:'connect-sdk-provisioning-validation/1',valid:true,value:normalized,reference:saved,kind:saved.command==='operation.cancel'?'operation_cancel':saved.command==='operation.reconcile'?'operation_reconcile':'administrative'}:{profile:'connect-sdk-provisioning-validation/1',valid:false};
 if(config.adminValidator==='profile')result.profile='other';
 if(config.adminValidator==='scope')result.reference.scope={conversation_id:'foreign'};
 if(config.adminValidator==='kind-array')result.kind=['administrative'];
 if(config.adminValidator==='foreign-id')result.reference.operation_id='foreign';
 if(config.adminValidator==='hang'){setInterval(()=>{},1000);await new Promise(()=>{});}
 process.stdout.write(JSON.stringify(result)+'\n');process.exit(0);
}
// Local validator simulation only; actual frozen-parser evidence is separate.
if (process.argv[2] === 'sdk-validate-envelope') {
 const path=process.env.CONNECT_SDK_FIXTURE_CONFIG, config=JSON.parse(readFileSync(path,'utf8')),dir=dirname(path);
 const value=JSON.parse(readFileSync(0,'utf8'));
 appendFileSync(join(dir,'validation.jsonl'),JSON.stringify({argv:process.argv.slice(2),pid:process.pid,value})+'\n',{mode:0o600});
 if(config.validator==='hang'){setInterval(()=>{},1000);await new Promise(()=>{});}
 const key=v=>v.operation_id==null?null:{operationId:v.operation_id,command:v.type,scope:{conversationId:v.cid??null,sessionId:v.sid??null,connectionId:v.connid??null}};
 const error=v=>({kind:'unsupported',certainty:v.operation_id==null?'read-only':'not-submitted',operation:key(v),problem:null,observation:null});
 const unsupported=value.type==='ack'||value.type==='connection.offer'||value.type==='conversation.subscribe'&&value.payload.live!==false;
 let result={profile:'connect-sdk-validation/1',valid:true,request:unsupported?{error:error(value)}:{key:key(value)}};
 if(value.type==='operation.get'&&typeof value.payload.operation_id!=='string')result={...result,valid:false,request:null};
 if(config.validator==='profile')result.profile='other';
 if(config.validator==='boolean')result.valid='true';
 if(config.validator==='foreign-key')result.request={key:{...key(value),operationId:'foreign'}};
 if(config.validator==='error-array')result.request={error:{...error(value),kind:['unsupported']}};
 if(config.validator==='duplicate')process.stdout.write('{"profile":"connect-sdk-validation/1","profile":"connect-sdk-validation/1","valid":false,"request":null}');
 else if(config.validator==='oversize')process.stdout.write('x'.repeat(20000));
 else process.stdout.write(JSON.stringify(result)+'\n');
 process.exit(0);
}
const path=process.argv[process.argv.indexOf('--config')+1], config=JSON.parse(readFileSync(path,'utf8')),dir=dirname(path);
writeFileSync(join(dir,'pid'),String(process.pid),{mode:0o600});
writeFileSync(join(dir,'argv.json'),JSON.stringify(process.argv.slice(2)),{mode:0o600});
process.on('exit',code=>writeFileSync(join(dir,'exit.json'),JSON.stringify({code}),{mode:0o600}));
if(config.signalDrainMs!==undefined||config.ignoreTermination){setInterval(()=>{},1000);process.on('SIGTERM',()=>{
 appendFileSync(join(dir,'signals.jsonl'),JSON.stringify('SIGTERM')+'\n',{mode:0o600});
 if(!config.ignoreTermination)setTimeout(()=>{writeFileSync(join(dir,'drained.json'),JSON.stringify({drained:true}),{mode:0o600});process.exit(0);},config.signalDrainMs);
});}
if(config.mode==='hang-close')setInterval(()=>{},1000);
const key=reference=>({operationId:reference.operation_id,command:reference.command,scope:{conversationId:reference.scope.conversation_id??null,sessionId:reference.scope.session_id??null,connectionId:reference.scope.connection_id??null}});
const reference=(id,command,scope={},continuation)=>({schema:'connect-operation-reference/1',operation_id:id,command,scope,...(continuation?{continuation}:{})});
let count=0;const intents=new Map();let incomingReader=null,incomingPending=true,incomingPolls=0;
const conn=(a,rev=null)=>({id:a.connection_id??a.reference.scope.connection_id,conversationId:a.conversation_id??a.reference.scope.conversation_id,sessionId:a.session_id??a.reference.scope.session_id,receiptStreamRevision:rev});
const binding=()=>({parameters:{id:'audio',medium:'audio',direction:'sendrecv',codec:{name:'pcmu'},streamLocalId:1},streamGeneration:1,connectionGeneration:1});
createInterface({input:process.stdin}).on('line',line=>{
 const req=JSON.parse(line),r=req.request,n=++count;let a=r.action;
 if(a.type==='administrative_operation')a={...a,kind:a.reference.command==='operation.cancel'?'operation_cancel':a.reference.command==='operation.reconcile'?'operation_reconcile':'administrative'};
 appendFileSync(join(dir,'requests.jsonl'),line+'\n',{mode:0o600});
 if(config.mode==='exit')return process.exit(0);
 if(config.mode==='partial')return process.stdout.write('{');
 if(config.mode==='utf8')return process.stdout.write(Buffer.from([255,10]));
 if(config.mode==='oversize')return process.stdout.write('x'.repeat(2097154)+'\n');
 let result;
 if(a.type==='administrative_provision'){
  const names={identity:'identity.provision',participant:'participant.provision',endpoint:'endpoint.provision',endpoint_verify:'endpoint.verify',endpoint_revoke:'endpoint.revoke',identity_revoke:'identity.revoke',record_read_role:'record-read-role.configure',signing_key:'signing-key.enroll',signing_key_revoke:'signing-key.revoke',device_revoke:'device.revoke',credential_revoke:'credential.revoke'};
  result={Ok:reference(a.mutation.operation_id,names[a.command.type])};
 }
 else if(a.type==='incoming_poll') {
  incomingPolls++;
  if(config.mode==='incoming-revoked-after-first'&&incomingPolls>1)result={Err:{kind:'authentication',certainty:'read-only',operation:null,problem:null,observation:null}};
  else if(incomingReader!==null&&incomingReader!==a.reader_id)result={Err:{kind:'conflict',certainty:'read-only',operation:null,problem:null,observation:null}};
  else {
   incomingReader=a.reader_id;
   if(a.acknowledged==='delivery-original')incomingPending=false;
   const hint={conversation_id:'room',session_id:'session',from_participant:'caller',to_participant:'callee',operation_id:'original-invite',source:{kind:'session.invite',correlation_id:'original-invite'},medium:'voice',intent:'synchronous-engagement',capabilities:{audio:['g.711-mu'],data:['reliable-ordered']}};
   if(config.mode==='incoming-source')hint.source.correlation_id='foreign';
   if(config.mode==='incoming-medium')hint.medium=['voice'];
   if(config.mode==='incoming-participants')hint.from_participant=hint.to_participant;
   if(config.mode==='incoming-intent')hint.intent='private\ncontrol';
   if(config.mode==='incoming-extra')hint.native_owner=true;
   const pending=!incomingPending||config.mode==='incoming-empty'||incomingPolls<=(config.incomingPendingPolls??0);
   result={Ok:{reader_id:config.mode==='incoming-reader'?'foreign':a.reader_id,delivery_id:pending?null:'delivery-original',invitation:pending?null:hint}};
   if(config.mode==='incoming-null')result.Ok.delivery_id=null;
  }
 } else if(a.type==='incoming_close') {
  if(incomingReader!==null&&incomingReader!==a.reader_id)result={Err:{kind:'conflict',certainty:'read-only',operation:null,problem:null,observation:null}};
  else {if(a.acknowledged==='delivery-original')incomingPending=false;incomingReader=null;result={Ok:{reader_id:a.reader_id,closed:true}};}
 }
 else if(a.type==='capabilities_refresh')result={Ok:{profile:'connect/2',runtime_role:config.mode==='caps-foreign'?'node':'rust',selected_transport:config.mode==='caps-transport'?'http':'quic',fallback_reason:null,features:config.mode==='caps-duplicate'?['original-id-operations','original-id-operations']:['original-id-operations','native-programmatic-media','durable-events','incoming-invitations','raw-canonical','message-receipts']}};
 else if(a.type==='raw_request') {
  const v=a.envelope,scope=Object.fromEntries(['cid','sid','connid','operation_id'].filter(k=>k in v).map(k=>[k,v[k]]));
  const error={kind:'unsupported',certainty:v.operation_id==null?'read-only':'not-submitted',operation:v.operation_id==null?null:{operationId:v.operation_id,command:'ack',scope:{conversationId:v.cid??null,sessionId:v.sid??null,connectionId:v.connid??null}},problem:null,observation:null};
  const frame={v:2,id:'reply',ts:'2026-10-28T00:00:00Z',type:'ack',...scope,in_reply_to:'fresh-wire-request',payload:{details:{original:'preserved',vendor:'x'.repeat(config.rawSize??1)}},signature:{keyid:'server',alg:'EdDSA',sig:Buffer.alloc(64).toString('base64url')}};
  if(config.rawNodes)frame.payload.details.vendor=Array(config.rawNodes).fill(false);
  for(let i=0;i<(config.rawDepth??0);i++)frame.payload.details.vendor={nested:frame.payload.details.vendor};
  result={Ok:{requestId:'fresh-wire-request',envelopes:[{value:frame,request:{error}}]}};
  if(config.mode==='raw-correlation')frame.in_reply_to='foreign';
  if(config.mode==='raw-scope')frame.cid='foreign';
  if(config.mode==='raw-logical')frame.operation_id='foreign';
  if(config.mode==='raw-alg-array')frame.signature.alg=['EdDSA'];
  if(config.mode==='raw-bad-signature')frame.signature.sig='xx';
  if(config.mode==='raw-error-frame')frame.type='error';
  if(config.mode==='raw-no-ack')frame.type='operation.snapshot';
  if(config.mode==='raw-duplicate'||config.mode==='raw-after-ack')result.Ok.envelopes.push({value:{...frame,id:config.mode==='raw-duplicate'?frame.id:'next'},request:{error}});
  if(config.mode==='raw-empty')result.Ok.envelopes=[];
  if(config.mode==='raw-wrong-descriptor')result.Ok.envelopes[0].request={key:{operationId:'foreign',command:'ack',scope:{conversationId:v.cid??null,sessionId:null,connectionId:null}}};
 }
 else if(a.type==='session_handoff'||a.type==='connection_move') {
  const i=a.input,handoff=a.type==='session_handoff',scope={conversation_id:a.conversation_id,session_id:a.session_id,connection_id:handoff?i.via:a.connection_id};
  const continuation=handoff?{kind:'handoff',retained_connection_id:i.retain,source_participant:i.from_participant,target_participant:i.to,candidate:i.candidate,role:i.role,outgoing:i.outgoing}:{kind:'replacement',retained_connection_id:i.retain,target:i.target};
  result={Ok:reference(i.mutation.operation_id,handoff?'session.handoff':'connection.replace',scope,continuation)};
 }
 else if(a.type==='consultation_return')result={Ok:reference(a.mutation.operation_id,'session.consult.return',a.reference.scope,{kind:'consultation-return',retained_connection_id:a.reference.continuation.retained_connection_id,consultation_record_id:a.reference.continuation.record_id??'record-original'})};
 else if(a.type==='conversation_create')result={Ok:reference(a.mutation.operation_id,'conversation.create')};
 else if(a.type==='conversation_send')result={Ok:reference(a.mutation.operation_id,'message.send',{conversation_id:a.conversation_id})};
 else if(a.type.startsWith('participant_')&&a.type!=='participant_refresh')result={Ok:reference(a.mutation.operation_id,'conversation.participant.'+a.type.slice(12),{conversation_id:a.conversation_id})};
 else if(a.type==='participant_refresh')result={Ok:{id:a.participant_id,conversationId:a.conversation_id,identityId:'identity',kind:'human',role:'participant',membershipEpoch:1,active:true,removalReason:null}};
 else if(a.type==='connection') {
  const act=a.action;
  if(act.type==='refresh')result={Ok:{id:a.connection_id,conversationId:a.conversation_id,sessionId:a.session_id,generation:1,streamRevision:null,state:'active'}};
  else if(act.type==='stream_refresh')result={Ok:{id:act.stream_id,conversationId:a.conversation_id,sessionId:a.session_id,connectionId:a.connection_id,medium:'audio',state:'active',resourceRevision:7,direction:null,format:null,clock:config.clock?{sourceId:'source:cname',generation:1,unitsPerSecond:'18446744073709551615',synchronized:false}:null,ownership:'observation'}};
  else {intents.set(act.mutation.operation_id,act);const command=(act.type==='subscribe'||act.type==='unsubscribe'?'stream.':'connection.')+act.type;result={Ok:reference(act.mutation.operation_id,command,{conversation_id:a.conversation_id,session_id:a.session_id,connection_id:a.connection_id})};}
 }
 else if(a.type==='session') {
  const act=a.action;
  if(act.type==='consult')result={Ok:reference(act.mutation.operation_id,'session.consult',{conversation_id:a.conversation_id,session_id:a.session_id,connection_id:act.via},{kind:'consultation',retained_connection_id:act.retain,record_id:'record-original'})};
  else if(act.type==='refresh')result={Ok:{id:a.session_id,conversationId:a.conversation_id,state:'accepted',media:['audio'],intent:'synthetic',capabilitiesOffer:{audio:['g.711-mu']},codecPreferences:[],isMain:true,revision:1,topologyRevision:2,createdAt:1,updatedAt:2,endedReason:null}};
  else result={Ok:reference(act.mutation.operation_id,'session.'+act.type,{conversation_id:a.conversation_id,session_id:a.session_id})};
 }
 else if(a.type==='message_receipts'){
  const observation={participantId:'participant',membershipEpoch:1,channel:'native',deliveryId:'delivery',state:'delivered',provenance:'client_reported',observedAt:1};
  result={Ok:{conversationId:a.conversation_id,messageId:a.message_id,observations:config.receipts??[observation]}};
  if(config.mode==='receipt-cid')result.Ok.conversationId='foreign';
  if(config.mode==='receipt-mid')result.Ok.messageId='foreign';
  if(config.mode==='receipt-duplicate')result.Ok.observations.push({...observation});
  if(config.mode==='receipt-state-array')result.Ok.observations[0].state=['delivered'];
  if(config.mode==='receipt-provenance')result.Ok.observations[0].provenance='human-read';
  if(config.mode==='receipt-epoch')result.Ok.observations[0].membershipEpoch=0;
  if(config.mode==='receipt-time')result.Ok.observations[0].observedAt=-1;
  if(config.mode==='receipt-channel')result.Ok.observations[0].channel='bad channel';
  if(config.mode==='receipt-extra')result.Ok.observations[0].extra=true;
  if(config.mode==='receipt-overflow')result.Ok.observations=Array.from({length:4097},(_,i)=>({...observation,deliveryId:'d'+i}));
 }
 else if(a.type==='conversation_refresh')result={Ok:{id:a.conversation_id,closed:false,revision:1,participants:[]}};
 else {
   const snap={key:key(a.reference),state:config.state??'completed',revision:1,recordId:'record-original',cancelRequested:null,expiresAt:null,problem:null};
   if(a.action.type==='wait') {
     const value=a.kind==='administrative'?{kind:'identity',identity_id:'created',created:true}:a.kind==='message_send'?{messageId:'message',operation:key(a.reference),boundary:'local-mutation'}:
       a.kind==='conversation_create'?{id:'room'}:a.kind.startsWith('conversation_participant_')?{id:'participant',conversationId:a.reference.scope.conversation_id}:
       a.kind==='session_consult'?{session:{id:a.reference.scope.session_id,conversationId:a.reference.scope.conversation_id},retained:[{...conn(a),id:a.reference.continuation.retained_connection_id}],admitted:[{...conn(a),id:'private'}],retired:[],topologyRevision:3,conversationRevision:null}:a.kind==='session_reject'||a.kind==='session_cancel'?{session:{id:a.reference.scope.session_id,conversationId:a.reference.scope.conversation_id},state:a.kind==='session_reject'?'rejected':'cancelled',topologyRevision:2}:
       a.kind.startsWith('session_')?{id:a.reference.scope.session_id,conversationId:a.reference.scope.conversation_id}:{decision:key(a.reference),target:'original',observedState:'completed',disposition:'already-terminal',evidenceId:a.kind==='operation_reconcile'?'evidence':null};
     let actual=value;
     if(a.kind==='session_handoff'||a.kind==='connection_replace'){
      const c=a.reference.continuation,handoff=a.kind==='session_handoff',id=handoff?c.candidate??'admitted':c.target.kind==='candidate'?c.target.id:'admitted';
      const handle=id=>({...conn(a),id});
      actual={retained:[handle(c.retained_connection_id)],admitted:[handle(id)],retired:handoff?(c.outgoing==='leave'?[a.reference.scope.connection_id]:[]):[a.reference.scope.connection_id],topologyRevision:3,conversationRevision:handoff&&c.role.scope==='conversation'?c.role.expected_conversation_revision+1:null};
      if(handoff)actual.roleChange=config.mode==='handoff-no-native'?null:{participantRevision:4,roleScope:c.role.scope,outgoing:c.outgoing,sourceParticipant:c.source_participant,targetParticipant:c.target_participant,cleanup:c.outgoing==='leave'?'pending':'not_requested'};
      else {actual.connection=handle(id);actual.binding={generation:3,participantId:'from',membershipEpoch:2,endpointId:c.target.kind==='endpoint'?c.target.id:'endpoint',deviceId:'device'};}
     } else if(a.kind==='connection_offer'||a.kind==='connection_end')actual=conn(a,a.kind==='connection_offer'?1:null);
     else if(a.kind==='connection_update'){
      const u=intents.get(a.reference.operation_id)?.update??{action:'quality',interval_ms:250};let outcome;
      if(u.action==='quality')outcome={kind:'quality',intervalMs:u.interval_ms};
      else if(u.action==='codec-renegotiate')outcome={kind:'codecs',resourcesChanged:true,streams:[binding()]};
      else outcome={kind:'gates',action:u.action,streams:[{...binding(),state:u.action==='hold'?'paused':'active',sourceMuted:u.action==='mute',heldDirection:null}]};
      actual={connection:conn(a),connectionGeneration:1,streamRevision:2,outcome};
     }else if(a.kind==='stream_subscribe'||a.kind==='stream_unsubscribe')actual={connection:conn(a),connectionGeneration:1,streamRevision:2,resourcesChanged:true,bindings:config.capture==='unavailable'?null:config.capture==='empty'?[]:[binding()]};
     result={Ok:{snapshot:snap,value:actual}};
   } else if(['cancel','reconcile'].includes(a.action.type))result={Ok:reference(a.action.mutation.operation_id,'operation.'+a.action.type,a.reference.scope)};
   else result={Ok:snap};
 }
 if(config.mode==='admin-conflict'&&a.type==='administrative_provision'&&a.command.alias==='changed')result={Err:{kind:'conflict',certainty:'refused',operation:null,problem:null,observation:null}};
 if(config.mode==='refuse'&&n===1)result={Err:{kind:config.kind??'receipt-unavailable',certainty:config.certainty??'recorded',operation:null,problem:{code:410,category:'receipt-unavailable',reason:'receipt-expired'},observation:config.observation??null}};
 if(config.mode==='state-array') {const snap=result.Ok?.snapshot??result.Ok;if(snap?.state)snap.state=[snap.state];}
 if(config.mode==='decision-state-array'&&result.Ok?.value?.observedState)result.Ok.value.observedState=[result.Ok.value.observedState];
 if(config.mode==='participant-kind-array'&&result.Ok?.kind)result.Ok.kind=[result.Ok.kind];
 if(config.mode==='conversation-kind-array'&&result.Ok?.participants)result.Ok.participants=[{id:'participant',conversationId:a.conversation_id,identityId:'identity',kind:['human'],role:'participant',membershipEpoch:1,active:true,removalReason:null}];
 if(config.mode==='movement-context'&&result.Ok?.continuation)result.Ok.continuation.retained_connection_id='foreign';
 if(config.mode==='movement-scope'&&result.Ok?.scope)result.Ok.scope.session_id='foreign';
 if(config.mode==='movement-duplicate'&&result.Ok?.value?.admitted)result.Ok.value.admitted[0].id=result.Ok.value.retained[0].id;
 if(config.mode==='movement-local-receipt'&&result.Ok?.value?.retained)result.Ok.value.retained[0].receiptStreamRevision=7;
 if(config.mode==='handoff-candidate'&&result.Ok?.value?.admitted)result.Ok.value.admitted[0].id='foreign';
 if(config.mode==='handoff-role'&&result.Ok?.value?.roleChange)result.Ok.value.roleChange.targetParticipant='foreign';
 if(config.mode==='handoff-revision'&&result.Ok?.value)result.Ok.value.conversationRevision=99;
 if(config.mode==='handoff-outgoing'&&result.Ok?.value)result.Ok.value.retired=[];
 if(config.mode==='handoff-cleanup'&&result.Ok?.value?.roleChange)result.Ok.value.roleChange.cleanup='not_requested';
 if(config.mode==='move-endpoint'&&result.Ok?.value?.binding)result.Ok.value.binding.endpointId='foreign';
 if(config.mode==='move-generation'&&result.Ok?.value?.binding)result.Ok.value.binding.generation=0;
 if(config.mode==='move-foreign-connection'&&result.Ok?.value?.connection)result.Ok.value.connection.sessionId='foreign';
 if(config.mode==='move-unretired'&&result.Ok?.value)result.Ok.value.retired=[];
 if(config.mode==='move-candidate'&&result.Ok?.value?.admitted){result.Ok.value.admitted[0].id='foreign';result.Ok.value.connection.id='foreign';}
 if(config.mode==='consult-foreign-context'&&result.Ok?.continuation)result.Ok.continuation.retained_connection_id='foreign';
 if(config.mode==='consult-foreign-record'&&result.Ok?.continuation) {if(result.Ok.continuation.kind==='consultation-return')result.Ok.continuation.consultation_record_id='record-foreign';}
 if(config.mode==='consult-foreign-retained'&&result.Ok?.value?.retained)result.Ok.value.retained[0].id='foreign';
 if(config.mode==='consult-duplicate'&&result.Ok?.value?.retained)result.Ok.value.admitted[0].id=result.Ok.value.retained[0].id;
 if(config.mode==='wrong-participant') { const p=result.Ok?.value??result.Ok;if(p?.conversationId)p.conversationId='foreign'; }
 if(config.mode==='wrong-session') { const v=result.Ok?.value??result.Ok; const h=v?.session??v;if(h?.conversationId)h.conversationId='foreign'; }
 if(config.mode==='wrong-session-decision'&&result.Ok?.value?.session)result.Ok.value.state='not-the-decision';
 if(config.mode==='wrong-connection') {const v=result.Ok?.value??result.Ok;if(v?.connection)v.connection.id='foreign';else if(v?.id)v.id='foreign';}
 if(config.mode==='missing-bindings'&&result.Ok?.value)delete result.Ok.value.bindings;
 if(config.mode==='binding-generation'&&result.Ok?.value?.bindings?.[0])result.Ok.value.bindings[0].connectionGeneration=2;
 if(config.mode==='binding-local-handle'&&result.Ok?.value?.bindings?.[0])result.Ok.value.bindings[0].parameters.streamLocalId=0;
 if(config.mode==='bad-clock'&&result.Ok?.clock)result.Ok.clock.unitsPerSecond='18446744073709551616';
 if(config.mode==='wrong-reference')result.Ok.operation_id='foreign';
 if(config.mode==='wrong-scope') { const k=result.Ok?.snapshot?.key??result.Ok?.key; if(k)k.scope.conversationId='foreign'; }
 if(config.mode==='wrong-result'&&result.Ok?.value)result.Ok.value={id:'room'};
 if(config.mode==='record-change'&&n>1) { const s=result.Ok?.snapshot??result.Ok;if(s?.recordId)s.recordId='record-foreign'; }
 if(config.mode==='error-kind-array'||config.mode==='error-certainty-array')result={Err:{kind:config.mode==='error-kind-array'?['protocol']:'application',certainty:config.mode==='error-certainty-array'?['refused']:'refused',operation:null,problem:null,observation:null}};
 if(config.mode==='binding-state-array'&&result.Ok?.value?.outcome?.streams)result.Ok.value.outcome.streams[0].state=['active'];
 if(config.mode==='caps-no-receipts'&&a.type==='capabilities_refresh')result.Ok.features=result.Ok.features.filter(f=>f!=='message-receipts');
 if(config.mode==='caps-no-receipts'&&a.type==='message_receipts')result={Err:{kind:'unsupported',certainty:'read-only',operation:null,problem:null,observation:null}};
 let phase='request';
 if(config.mode==='startup'){phase='startup';result={Err:{kind:config.kind??'authentication',certainty:config.certainty??'read-only',operation:null,problem:null,observation:null}};}
 if(config.mode==='phase-array')phase=['request'];
 if(config.mode==='startup-success')phase='startup';
 if(config.mode==='startup-uncertain'){phase='startup';result={Err:{kind:'authentication',certainty:'uncertain',operation:null,problem:null,observation:null}};}
 if(config.mode==='startup-observation'){phase='startup';result={Err:{kind:'authentication',certainty:'read-only',operation:null,problem:null,observation:{private:'untrusted'}}};}
 const response={phase,profile:config.mode==='profile'?'foreign':process.argv[2]==='sdk-admin-stdio'?'connect-sdk-administration/2':'connect-sdk-helper/3',id:config.mode==='wrong-id'?'foreign':req.id,result};
 if(config.mode==='phase-missing')delete response.phase;
 if(config.mode==='profile-old')response.profile=process.argv[2]==='sdk-admin-stdio'?'connect-sdk-administration/1':'connect-sdk-helper/2';
 let raw=JSON.stringify(response);
 if(config.mode==='duplicate')raw='{"id":"duplicate",'+raw.slice(1);
 if(config.mode==='extra')raw+='\n'+raw;
 const send=()=>process.stdout.write(raw+'\n');
 const delay=a.type==='incoming_poll'?(incomingPolls===1?(config.incomingDelayMs??config.delayMs):(config.incomingLaterDelayMs??config.incomingDelayMs??config.delayMs)):config.delayMs;
 if(delay)setTimeout(send,delay);else send();
});
