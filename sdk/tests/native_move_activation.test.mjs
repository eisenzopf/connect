import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { movementCompletion } from '../typescript/dist/browser_sdk_movements.js';
import { moveResult } from '../typescript/dist/node_movements.js';
const ready = JSON.parse(await readFile(new URL('./fixtures/native_move_activation.json', import.meta.url)));
const context = {kind:'replacement',retained_connection_id:'remote',target:{kind:'endpoint',id:'endpoint_new'}};
const key = {command:'connection.replace',scope:{conversationId:'conversation',sessionId:'session',connectionId:'source'},continuation:context};
const fail = () => { throw new Error('refused'); };
const stopped = {...ready,activation_state:'stopped',readiness:null,retired_connection_ids:['source','candidate']};
const handle = id => ({id,conversationId:'conversation',sessionId:'session',receiptStreamRevision:null});
const helper = v => ({retained:v.retained_connection_ids.map(handle),admitted:v.admitted_connection_ids.map(handle),retired:v.retired_connection_ids,
 topologyRevision:v.session_revision,conversationRevision:null,connection:handle(v.connection_id),activationState:v.activation_state,
 binding:{generation:v.generation,participantId:v.participant_id,membershipEpoch:v.membership_epoch,endpointId:v.endpoint_id,deviceId:v.device_id}});
const decode = v => moveResult(v,{key,connection:handle},{continuation:context});
test('browser native activation completion preserves stopped history and refuses contradictory readiness', () => {
 for (const v of [ready,stopped]) {
  const result = movementCompletion(v,key,fail); assert.equal(result.activationState,v.activation_state);
  assert.deepEqual(result.retired,v.retired_connection_ids);assert.equal(result.binding.generation,2);
 }
 const telephone=structuredClone(ready);delete telephone.activation_state;delete telephone.physical_readiness;
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
