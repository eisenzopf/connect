// Copied into an installed external project; all traffic uses public SDK exports.
import assert from 'node:assert/strict';
import {readFile,realpath} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {createInterface} from 'node:readline';
import {CanonicalHttpClient,CanonicalError,HttpClient,ConnectError} from '@rudeless/connect';
const spec=JSON.parse(await readFile(process.argv[2],'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project+'/node_modules/'));
const options={timeoutMs:4000},client=new CanonicalHttpClient({endpoint:spec.endpoint,credential:spec.credential,transport:'remote-https'}),application=new HttpClient(spec.endpoint,spec.credential,4000,'remote-https');
const scope={cid:spec.cid,operation_id:'node-https-original-send'},payload={msg_id:'node-https-original-message',to:[spec.recipient],content_type:'text/plain',body:'synthetic installed HTTPS control',body_encoding:'utf8',label:'connect-messages',attachments:[],in_reply_to_msg:null};
const record=frames=>{const value=frames.find(v=>v.type==='operation.updated')?.payload;assert.ok(value);return value;};
const emit=value=>process.stdout.write(JSON.stringify(value)+'\n'),lines=createInterface({input:process.stdin})[Symbol.asyncIterator]();
let checkpoint='https-snapshot';
const progress=stage=>{checkpoint=stage;emit({phase:'progress',checkpoint:stage});};
try {
 const snapshot=await client.request('conversation.get',{}, {cid:spec.cid},options),cursor=snapshot.find(v=>v.type==='conversation.snapshot')?.payload.cursor;assert.equal(typeof cursor,'string');
 progress('https-send');await client.request('message.send',payload,scope,options);
 const original=record(await client.request('operation.get',{operation_id:scope.operation_id},{cid:spec.cid},options));assert.equal(original.state,'completed');
 await client.request('message.send',payload,scope,options);
 const same=record(await client.request('operation.get',{operation_id:scope.operation_id},{cid:spec.cid},options));assert.equal(same.record_id,original.record_id);
 progress('https-journal');const journal=await client.request('conversation.subscribe',{after:cursor,live:false,limit:63},{cid:spec.cid},options);
 const fact=journal.filter(v=>v.type==='conversation.event').map(v=>v.payload.data).find(v=>v.operation_id===scope.operation_id&&v.data?.kind==='message'&&v.data.data.id===payload.msg_id);assert.ok(fact);assert.equal(fact.conversation_id,spec.cid);assert.equal(fact.data.data.conversation_id,spec.cid);assert.ok(Array.isArray(original.causal_event_ids)&&original.causal_event_ids.includes(fact.event_id));
 progress('https-receipts');const receipts=await application.messageReceipts(spec.cid,payload.msg_id);assert.equal(receipts.conversation_id,spec.cid);assert.equal(receipts.message_id,payload.msg_id);assert.equal(receipts.receipts.length,0);
 emit({phase:'initial',record_id:original.record_id,current_https_journal:true,original_replay_record:true,application_receipts_empty:true});
 let timer;try{const next=await Promise.race([lines.next(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('bounded host phase missing')),15000);})]);assert.equal(next.value,'revoked');}finally{clearTimeout(timer);}
 progress('https-revocation');for(const [type,body,bound] of [['operation.get',{operation_id:scope.operation_id},{cid:spec.cid}],['message.send',payload,scope],['message.send',{...payload,msg_id:'node-https-forbidden-message'},{...scope,operation_id:'node-https-forbidden-send'}]])await assert.rejects(client.request(type,body,bound,options),error=>{assert.ok(error instanceof CanonicalError);assert.equal(error.kind,'authentication');assert.equal(error.certainty,type==='operation.get'?'read-only':'refused');assert.ok([401,403].includes(error.problem.code));return true;});
 await assert.rejects(application.messageReceipts(spec.cid,payload.msg_id),error=>{assert.ok(error instanceof ConnectError);assert.ok(['unauthorized','forbidden'].includes(error.code));assert.ok([401,403].includes(error.httpStatus));return true;});
 emit({phase:'completed',current_https_read_refused:true,current_https_replay_refused:true,current_https_new_mutation_refused:true,current_https_receipts_refused:true});
} catch(error) {
 const at=String(error?.stack??'').match(/node_https_server_consumer\.mjs:(\d+):(\d+)/);
 emit({phase:'failed',checkpoint,kind:error instanceof CanonicalError?error.kind:error?.code==='ERR_ASSERTION'?'assertion':'unexpected',certainty:error instanceof CanonicalError?error.certainty:null,command:error instanceof CanonicalError?error.operation?.command??null:null,observedState:null,observedKind:null,observedCertainty:null,observedHttpStatus:null,line:at?Number(at[1]):null,column:at?Number(at[2]):null});
 process.exitCode=1;
} finally {await client.close(options);process.stdin.destroy();}
