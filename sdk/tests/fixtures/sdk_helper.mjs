#!/usr/bin/env node
// Synthetic SDK helper framing/process fixture. No native verification claim.
import {readFileSync,writeFileSync,appendFileSync} from 'node:fs';
import {dirname,join} from 'node:path';
import {createInterface} from 'node:readline';
const path=process.argv[process.argv.indexOf('--config')+1], config=JSON.parse(readFileSync(path,'utf8')),dir=dirname(path);
writeFileSync(join(dir,'pid'),String(process.pid),{mode:0o600});
writeFileSync(join(dir,'argv.json'),JSON.stringify(process.argv.slice(2)),{mode:0o600});
process.on('exit',code=>writeFileSync(join(dir,'exit.json'),JSON.stringify({code}),{mode:0o600}));
if(config.mode==='hang-close')setInterval(()=>{},1000);
const key=reference=>({operationId:reference.operation_id,command:reference.command,scope:{conversationId:reference.scope.conversation_id??null,sessionId:reference.scope.session_id??null,connectionId:reference.scope.connection_id??null}});
const reference=(id,command,scope={},continuation)=>({schema:'connect-operation-reference/1',operation_id:id,command,scope,...(continuation?{continuation}:{})});
let count=0;const intents=new Map();
const conn=(a,rev=null)=>({id:a.connection_id??a.reference.scope.connection_id,conversationId:a.conversation_id??a.reference.scope.conversation_id,sessionId:a.session_id??a.reference.scope.session_id,receiptStreamRevision:rev});
const binding=()=>({parameters:{id:'audio',medium:'audio',direction:'sendrecv',codec:{name:'pcmu'},streamLocalId:1},streamGeneration:1,connectionGeneration:1});
createInterface({input:process.stdin}).on('line',line=>{
 const req=JSON.parse(line),r=req.request,a=r.action,n=++count;
 appendFileSync(join(dir,'requests.jsonl'),line+'\n',{mode:0o600});
 if(config.mode==='exit')return process.exit(0);
 if(config.mode==='partial')return process.stdout.write('{');
 if(config.mode==='utf8')return process.stdout.write(Buffer.from([255,10]));
 if(config.mode==='oversize')return process.stdout.write('x'.repeat(2097154)+'\n');
 let result;
 if(a.type==='capabilities_refresh')result={Ok:{profile:'connect/2',runtime_role:config.mode==='caps-foreign'?'node':'rust',selected_transport:config.mode==='caps-transport'?'http':'quic',fallback_reason:null,features:config.mode==='caps-duplicate'?['original-id-operations','original-id-operations']:['original-id-operations','native-programmatic-media','durable-events','incoming-invitations','raw-canonical','message-receipts']}};
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
 else if(a.type==='conversation_refresh')result={Ok:{id:a.conversation_id,closed:false,revision:1,participants:[]}};
 else {
   const snap={key:key(a.reference),state:config.state??'completed',revision:1,recordId:'record-original',cancelRequested:null,expiresAt:null,problem:null};
   if(a.action.type==='wait') {
     const value=a.kind==='message_send'?{messageId:'message',operation:key(a.reference),boundary:'local-mutation'}:
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
 const response={profile:config.mode==='profile'?'foreign':'connect-sdk-helper/1',id:config.mode==='wrong-id'?'foreign':req.id,result};
 let raw=JSON.stringify(response);
 if(config.mode==='duplicate')raw='{"id":"duplicate",'+raw.slice(1);
 if(config.mode==='extra')raw+='\n'+raw;
 const send=()=>process.stdout.write(raw+'\n');
 if(config.delayMs)setTimeout(send,config.delayMs);else send();
});
