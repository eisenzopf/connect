import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {decodeResponse} from '../typescript/dist/protocol.js';
import {exchangeTimeout} from '../typescript/dist/task_operations.js';
const matrix=JSON.parse(readFileSync(new URL('./events_matrix.json',import.meta.url)));
const envelope=outcome=>({profile:'connect-control/1',result:{Ok:outcome}});
for(const vector of matrix.valid)test(`event result accepts: ${vector.name}`,()=>assert.equal(decodeResponse(envelope(vector.outcome),vector.outcome.kind).kind,vector.outcome.kind));
for(const vector of matrix.invalid)test(`event result rejects: ${vector.name}`,()=>assert.throws(()=>decodeResponse(envelope(vector.outcome),vector.outcome.kind),e=>e.code==='protocol'));
test('event waits are bounded independently from the exchange timeout',()=>{
 assert.equal(exchangeTimeout(100,{op:'conversation_events',conversation_id:'cid',query:{wait_ms:999999}}),30100);
 assert.equal(exchangeTimeout(100,{op:'session_observations',conversation_id:'cid',session_id:'sid',query:{wait_ms:true}}),100);
});
test('nonfinite or invalid Unicode observation content is a protocol rejection',()=>{
 for(const value of [Infinity,'\ud800']){const o=structuredClone(matrix.valid[5].outcome);o.data.observations[0].data.arguments.value=value;assert.throws(()=>decodeResponse(envelope(o),o.kind),e=>e.code==='protocol');}
});
