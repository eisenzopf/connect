// Copy into an installed external Node project. Uses exported SDK API only.
import assert from 'node:assert/strict';
import {readFile,writeFile,realpath} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {NodeClient,NodeSdkError,CanonicalHttpClient} from '@rudeless/connect';
const spec=JSON.parse(await readFile(process.argv[2],'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project+'/node_modules/'));
const opts={timeoutMs:4000},mutation={...opts,operationId:'typed-node-original-send'};
const lines=createInterface({input:process.stdin})[Symbol.asyncIterator]();
async function resume(expected) {
 let timer;try {const next=await Promise.race([lines.next(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('phase deadline')),10000);})]);assert.equal(next.value,expected);}finally{clearTimeout(timer);}
}
const emit=v=>process.stdout.write(JSON.stringify(v)+'\n');
const stopped=async()=>{
 const pids=(await readFile(spec.pid_file,'utf8')).trim().split('\n').filter(Boolean).map(Number);
 for(const pid of pids)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');return pids.length;
};
const record=frames=>{const p=frames.find(f=>f.type==='operation.updated')?.payload;assert.ok(p);return p;};
let client=new NodeClient({config:spec.config,executable:spec.executable});
const http=new CanonicalHttpClient({endpoint:spec.http,credential:spec.credential});
try {
 const created=await client.conversations.create({participants:[spec.peer_identity],policy:'persistent'},{...opts,operationId:'typed-node-room'});
 const createSaved=created.referenceJson(),room=(await created.wait(opts)).value;
 assert.equal(created.reference.command,'conversation.create');assert.equal((await client.operations.restore(createSaved,client.operationKinds.conversation_create).wait(opts)).value.id,room.id);
 const refreshed=await room.refresh(opts);assert.equal(refreshed.id,room.id);
 const peer=refreshed.participants.find(p=>p.identityId===spec.peer_identity);assert.ok(peer);
 const first=room.participants.byId(peer.id);
 await (await first.remove({expectedConversationRevision:refreshed.revision},{...opts,operationId:'typed-node-member-reset'})).wait(opts);
 await assert.rejects(first.refresh(opts),e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,'receipt-unavailable');assert.equal(e.certainty,'read-only');return true;});
 const reset=await room.refresh(opts),addInput={identityId:spec.peer_identity,kind:peer.kind,role:'participant',expectedRevision:reset.revision};
 const added=await room.participants.add(addInput,{...opts,operationId:'typed-node-member-add'}),addedSaved=added.referenceJson(),member=(await added.wait(opts)).value;
 assert.equal(member.id,peer.id);const epoch=(await member.refresh(opts)).membershipEpoch;
 const beforeRole=await room.refresh(opts);
 await (await member.update({role:'observer',expectedConversationRevision:beforeRole.revision},{...opts,operationId:'typed-node-member-role'})).wait(opts);
 assert.equal((await member.refresh(opts)).role,'observer');
 // Explicit replay returns the original logical operation, without another admission.
 assert.equal((await (await room.participants.add(addInput,{...opts,operationId:'typed-node-member-add'})).wait(opts)).value.id,peer.id);
 await assert.rejects(member.update({role:'participant',expectedConversationRevision:reset.revision},{...opts,operationId:'typed-node-member-stale'}),e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.certainty,'refused');return true;});
 const beforeRemove=await room.refresh(opts);await (await member.remove({expectedConversationRevision:beforeRemove.revision},{...opts,operationId:'typed-node-member-remove'})).wait(opts);
 await assert.rejects(member.refresh(opts),e=>{assert.equal(e.kind,'receipt-unavailable');return true;});
 const beforeReturn=await room.refresh(opts),returned=(await (await room.participants.add({...addInput,expectedRevision:beforeReturn.revision},{...opts,operationId:'typed-node-member-return'})).wait(opts)).value;
 assert.equal(returned.id,peer.id);assert.ok((await returned.refresh(opts)).membershipEpoch>epoch);
 await assert.rejects(client.operations.restore(addedSaved,client.operationKinds.conversation_participant_add).status(opts),e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.certainty,'refused');return true;});
 const intent={messageId:'typed-node-original-message',to:{first:peer.id,rest:[]},text:'synthetic typed Node journey'};
 const sent=await room.send(intent,mutation),saved=sent.referenceJson();
 assert.throws(()=>sent.reference.scope.conversationId='foreign');assert.throws(()=>client.operations.restore(saved,client.operationKinds.conversation_create));
 const ready=await sent.waitUntilReady(opts),completion=await sent.wait(opts);
 assert.equal(ready.state,'completed');assert.equal(completion.value.messageId,intent.messageId);assert.equal(completion.value.boundary,'local-mutation');
 const original=completion.snapshot,viaHttp=record(await http.request('operation.get',{operation_id:mutation.operationId},{cid:room.id},opts));
 assert.equal(original.recordId,viaHttp.record_id);assert.equal(viaHttp.state,'completed');
 await room.send(intent,mutation);
 const history=await http.request('message.history',{limit:100},{cid:room.id},opts);
 assert.equal(history.filter(f=>f.type==='message.send'&&f.payload.msg_id===intent.messageId).length,1);
 const cancel=await sent.cancel({...opts,operationId:'typed-node-cancel'}),cancelSaved=cancel.referenceJson();
 const decision=await cancel.wait(opts);assert.equal(decision.value.target,mutation.operationId);assert.equal(decision.value.decision.operationId,'typed-node-cancel');
 assert.equal((await sent.status(opts)).state,'completed');
 await writeFile(spec.config,'invalid TOML after fixed SDK helper loaded\n',{mode:0o600});
 assert.equal((await sent.status(opts)).recordId,original.recordId);
 await client.close(opts);assert.equal(await stopped(),1);
 emit({phase:'initial',record_id:original.recordId,state:original.state,cid:room.id,typed_result:true,same_id_one_message:true,native_http_same_record:true,decision_target:decision.value.target,decision_id:decision.value.decision.operationId,participant_stable_id:true,membership_epoch_advanced:true,removed_receipt_unavailable:true,old_membership_operation_refused:true,stale_revision_refused:true,first_helper_joined:true});
 await resume('restart');
 client=new NodeClient({config:spec.config,executable:spec.executable});
 const recovered=client.operations.restore(saved,client.operationKinds.message_send),status=await recovered.status(opts);
 assert.equal(status.recordId,original.recordId);assert.equal((await recovered.wait(opts)).value.messageId,intent.messageId);
 assert.equal((await client.operations.recover(recovered.reference).status(opts)).recordId,original.recordId);
 const restoredDecision=await client.operations.restore(cancelSaved,client.operationKinds.operation_cancel).wait(opts);assert.equal(restoredDecision.value.target,mutation.operationId);
 const after=await http.request('message.history',{limit:100},{cid:room.id},opts);assert.equal(after.filter(f=>f.type==='message.send'&&f.payload.msg_id===intent.messageId).length,1);
 emit({phase:'recovered',record_id:status.recordId,state:status.state,resubmitted:false,typed_wait:true,typed_decision_recovered:true});
 await resume('revoked');
 let nativeFailure,httpFailure;
 await assert.rejects(client.conversations.byId(room.id).send(intent,mutation),e=>{
  assert.ok(e instanceof NodeSdkError);assert.ok(['authentication','transport-uncertain'].includes(e.kind));assert.ok(['refused','uncertain'].includes(e.certainty));assert.equal(e.operation.operationId,mutation.operationId);nativeFailure={kind:e.kind,certainty:e.certainty};return true;
 });
 await assert.rejects(http.request('operation.get',{operation_id:mutation.operationId},{cid:room.id},opts),e=>{assert.equal(e.kind,'authentication');httpFailure={kind:e.kind,certainty:e.certainty};return true;});
 await client.close(opts);await http.close(opts);assert.equal(await stopped(),2);
 emit({phase:'completed',algorithm:spec.algorithm,native_failure:nativeFailure,http_failure:httpFailure,helpers_joined:2,query_recovery_without_replay:true});
} finally {await client.close(opts);await http.close(opts);process.stdin.destroy();}
