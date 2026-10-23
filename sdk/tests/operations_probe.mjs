import assert from 'node:assert/strict';
import {HttpClient} from '../typescript/dist/index.js';
const v=JSON.parse(process.argv[2]),client=new HttpClient(v.base,v.token);
assert.equal((await client.admitTransition(v.request)).record_id,v.record_id);
const get={action:'get',operation_id:v.request.operation_id};
assert.equal((await client.operationAction(get)).state,'accepted');
await assert.rejects(new HttpClient(v.base,v.foreign_token).operationAction(get),e=>e.code==='forbidden');
await assert.rejects(client.admitTransition({...v.request,expires_at:v.request.expires_at+1}),e=>e.code==='conflict');
console.log(JSON.stringify({passed:true}));
