import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { movementCompletion } from '../typescript/dist/browser_sdk_movements.js';
import { moveResult } from '../typescript/dist/node_movements.js';
import { transitionObservation } from '../typescript/dist/browser_sdk_consultations.js';
const ready = JSON.parse(await readFile(new URL('./fixtures/native_move_activation.json', import.meta.url)));
const context = {kind:'replacement',retained_connection_id:'remote',target:{kind:'endpoint',id:'endpoint_new'}};
const key = {operationId:'original',command:'connection.replace',scope:{conversationId:'conversation',sessionId:'session',connectionId:'source'},continuation:context};
const fail = () => { throw new Error('refused'); };
const stopped = {...ready,activation_state:'stopped',readiness:null,retired_connection_ids:['source','candidate']};
const handle = id => ({id,conversationId:'conversation',sessionId:'session',receiptStreamRevision:null});
const helper = v => ({retained:v.retained_connection_ids.map(handle),admitted:v.admitted_connection_ids.map(handle),retired:v.retired_connection_ids,
 topologyRevision:v.session_revision,conversationRevision:null,connection:handle(v.connection_id),activationState:v.activation_state,
 binding:{generation:v.generation,participantId:v.participant_id,membershipEpoch:v.membership_epoch,endpointId:v.endpoint_id,deviceId:v.device_id}});
const decode = v => moveResult(v,{key,connection:handle},{continuation:context});
test('native activation proof is bound to original intent and completed record', () => {
 movementCompletion(ready,key,fail);
 for(const [path,value] of [
  [['native_activation'],null],
  [['native_activation','operation_id'],'foreign'],
  [['native_activation','source_connection_id'],'foreign'],
  [['native_activation','target_connection_id'],'foreign'],
  [['native_activation','target_generation'],3],
  [['native_activation','stream_revision'],5],
  [['native_activation','profile'],'unknown'],
  [['native_activation','permission'],true],
  [['native_activation','audience','streams',0,'stream_id'],'foreign'],
  [['native_activation','audience','streams',0,'physical_incarnation'],'0'.repeat(32)],
  [['native_activation','audience','streams',0,'public_sequence_floor'],5],
  [['native_activation','audience','streams',0,'submission_observed'],true],
  [['native_activation','audience','streams'],[]],
 ]) {
  const changed=structuredClone(ready);let at=changed;
  for(const p of path.slice(0,-1))at=at[p];at[path.at(-1)]=value;
  assert.throws(()=>movementCompletion(changed,key,fail),/refused/,path.join('/'));
 }
 const missing=structuredClone(ready);delete missing.native_activation;
 assert.throws(()=>movementCompletion(missing,key,fail),/refused/);
 const duplicate=structuredClone(ready);duplicate.native_activation.audience.streams.push(duplicate.native_activation.audience.streams[0]);
 assert.throws(()=>movementCompletion(duplicate,key,fail),/refused/);
 const noProof={...stopped,native_activation:null};movementCompletion(noProof,key,fail);
 delete noProof.native_activation;assert.throws(()=>movementCompletion(noProof,key,fail),/refused/);
 for(const audience of [{scope:'private-consultation',original_operation:'parent'},{scope:'unsupported',reason:'non-native-adapter'}]) {
  const value=structuredClone(ready);value.native_activation.audience=audience;movementCompletion(value,key,fail);
 }
 const observed={state:'completed',command_type:'connection.replace',record_id:'record_original',expected_revision:3,result:ready};
 transitionObservation(observed,key,undefined,fail);
 assert.throws(()=>transitionObservation({...observed,record_id:'foreign'},key,undefined,fail),/refused/);
});
test('browser native activation completion preserves stopped history and refuses contradictory readiness', () => {
 for (const v of [ready,stopped]) {
  const result = movementCompletion(v,key,fail); assert.equal(result.activationState,v.activation_state);
  assert.deepEqual(result.retired,v.retired_connection_ids);assert.equal(result.binding.generation,2);
 }
 const telephone=structuredClone(ready);delete telephone.activation_state;delete telephone.physical_readiness;delete telephone.native_activation;
 assert.equal(movementCompletion(telephone,key,fail).activationState,null);
 for(const [base,field,value] of [
  [ready,'retired_connection_ids',['source','candidate']], [ready,'physical_readiness',{streams:['foreign']}],
  [ready,'readiness',null], [ready,'activation_state',null], [ready,'activation_state','prepared'],
  [stopped,'readiness',ready.readiness], [stopped,'physical_readiness',null],
  [stopped,'retired_connection_ids',['source']], [stopped,'retired_connection_ids',['source','candidate','candidate']],
  [stopped,'retired_connection_ids',['source','candidate','remote']], [stopped,'retained_connection_ids',['candidate']],
  [stopped,'admitted_connection_ids',['foreign']], [stopped,'endpoint_id','foreign']
 ]) assert.throws(()=>movementCompletion({...base,[field]:value},key,fail), /refused/,field);
 for(const field of ['activation_state','physical_readiness','readiness']) {
  const v=structuredClone(stopped);delete v[field];assert.throws(()=>movementCompletion(v,key,fail),/refused/);
 }
});
test('Node decoded activation state allows only the exact admitted-then-retired target', () => {
 for(const v of [ready,stopped]) {const result=decode(helper(v));assert.equal(result.activationState,v.activation_state);assert.deepEqual(result.retired,v.retired_connection_ids);}
 const telephone=helper(ready);telephone.activationState=null;assert.equal(decode(telephone).activationState,null);
 for(const [base,field,value] of [
  [ready,'activationState','prepared'],[ready,'retired',['source','candidate']],
  [stopped,'retired',['source']],[stopped,'retired',['source','candidate','candidate']],
  [stopped,'retired',['source','candidate','remote']],[stopped,'admitted',[handle('foreign')]],
  [stopped,'activationState',null]
 ]) assert.throws(()=>decode({...helper(base),[field]:value}));
 const missing=helper(ready);delete missing.activationState;assert.throws(()=>decode(missing));
});
