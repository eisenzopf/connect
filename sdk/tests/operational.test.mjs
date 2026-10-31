import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { validOperationalHealth } from '../typescript/dist/operational.js';
import { decodeResponse } from '../typescript/dist/protocol.js';
const fixture=JSON.parse(readFileSync(new URL('../../contracts/fixtures/operational-health-v1.json',import.meta.url),'utf8'));
const decode=operational=>decodeResponse({profile:'connect-control/1',result:{Ok:{kind:'health',data:{installation_id:'fixture',schema:1,profile:'connect-control/1',operational}}}},'health');
test('administrator numeric observations preserve unavailable and over-limit samples',()=>{
 assert.equal(validOperationalHealth(fixture),true);
 assert.deepEqual(decode(fixture).data.operational,fixture);
 const v=structuredClone(fixture);v.resource_pools[0].usage.used=5;
 assert.equal(validOperationalHealth(v),true);
 v.runtime=null;v.resource_pools=null;assert.equal(validOperationalHealth(v),true);
});
test('malformed or sensitive operational fields and inconsistent lifecycle are refused',()=>{
 const edits=[
 v=>{v.operations.accepted=true;},v=>{v.operations.retained=1;},
 v=>{v.limits.connections=Number.MAX_SAFE_INTEGER+1;},v=>{v.observed_at=-1;},
 v=>{v.credential='private-diagnostic-secret';},v=>{v.operations.body='private-diagnostic-secret';},
 v=>{v.resource_pools[0].usage.remote_address='private-diagnostic-secret';},
 v=>{v.resource_pools.push(v.resource_pools[0]);},v=>{v.resource_pools=Array(33).fill(v.resource_pools[0]);},
 v=>{v.resource_pools[0].usage=null;},v=>{v.resource_pools[1].usage=v.resource_pools[0].usage;},
 v=>{v.resource_pools[0].kind='private-provider-id';},v=>{v.runtime.owned_tasks=1.5;},
 ];
 for(const edit of edits){const v=structuredClone(fixture);edit(v);assert.equal(validOperationalHealth(v),false);assert.throws(()=>decode(v),e=>e.code==='protocol'&&!e.message.includes('private-diagnostic-secret'));}
});
