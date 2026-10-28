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
const reference=(id,command,scope={})=>({schema:'connect-operation-reference/1',operation_id:id,command,scope});
let count=0;
createInterface({input:process.stdin}).on('line',line=>{
 const req=JSON.parse(line),r=req.request,a=r.action,n=++count;
 appendFileSync(join(dir,'requests.jsonl'),line+'\n',{mode:0o600});
 if(config.mode==='exit')return process.exit(0);
 if(config.mode==='partial')return process.stdout.write('{');
 if(config.mode==='utf8')return process.stdout.write(Buffer.from([255,10]));
 if(config.mode==='oversize')return process.stdout.write('x'.repeat(2097154)+'\n');
 let result;
 if(a.type==='conversation_create')result={Ok:reference(a.mutation.operation_id,'conversation.create')};
 else if(a.type==='conversation_send')result={Ok:reference(a.mutation.operation_id,'message.send',{conversation_id:a.conversation_id})};
 else if(a.type.startsWith('participant_')&&a.type!=='participant_refresh')result={Ok:reference(a.mutation.operation_id,'conversation.participant.'+a.type.slice(12),{conversation_id:a.conversation_id})};
 else if(a.type==='participant_refresh')result={Ok:{id:a.participant_id,conversationId:a.conversation_id,identityId:'identity',kind:'human',role:'participant',membershipEpoch:1,active:true,removalReason:null}};
 else if(a.type==='conversation_refresh')result={Ok:{id:a.conversation_id,closed:false,revision:1,participants:[]}};
 else {
   const snap={key:key(a.reference),state:config.state??'completed',revision:1,recordId:'record-original',cancelRequested:null,expiresAt:null,problem:null};
   if(a.action.type==='wait') {
     const value=a.kind==='message_send'?{messageId:'message',operation:key(a.reference),boundary:'local-mutation'}:
       a.kind==='conversation_create'?{id:'room'}:a.kind.startsWith('conversation_participant_')?{id:'participant',conversationId:a.reference.scope.conversation_id}:{decision:key(a.reference),target:'original',observedState:'completed',disposition:'already-terminal',evidenceId:a.kind==='operation_reconcile'?'evidence':null};
     result={Ok:{snapshot:snap,value}};
   } else if(['cancel','reconcile'].includes(a.action.type))result={Ok:reference(a.action.mutation.operation_id,'operation.'+a.action.type,a.reference.scope)};
   else result={Ok:snap};
 }
 if(config.mode==='refuse'&&n===1)result={Err:{kind:config.kind??'receipt-unavailable',certainty:config.certainty??'recorded',operation:null,problem:{code:410,category:'receipt-unavailable',reason:'receipt-expired'},observation:config.observation??null}};
 if(config.mode==='wrong-participant') { const p=result.Ok?.value??result.Ok;if(p?.conversationId)p.conversationId='foreign'; }
 if(config.mode==='wrong-reference')result.Ok.operation_id='foreign';
 if(config.mode==='wrong-scope') { const k=result.Ok?.snapshot?.key??result.Ok?.key; if(k)k.scope.conversationId='foreign'; }
 if(config.mode==='wrong-result'&&result.Ok?.value)result.Ok.value={id:'room'};
 if(config.mode==='record-change'&&n>1) { const s=result.Ok?.snapshot??result.Ok;if(s?.recordId)s.recordId='record-foreign'; }
 const response={profile:config.mode==='profile'?'foreign':'connect-sdk-helper/1',id:config.mode==='wrong-id'?'foreign':req.id,result};
 let raw=JSON.stringify(response);
 if(config.mode==='duplicate')raw='{"id":"duplicate",'+raw.slice(1);
 if(config.mode==='extra')raw+='\n'+raw;
 const send=()=>process.stdout.write(raw+'\n');
 if(config.delayMs)setTimeout(send,config.delayMs);else send();
});
