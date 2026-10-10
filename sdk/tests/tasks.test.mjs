import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { NativeClient, ConnectError } from '../typescript/dist/index.js';
import { decodeResponse } from '../typescript/dist/protocol.js';
import { exchangeTimeout } from '../typescript/dist/task_operations.js';
const fixtures=JSON.parse(readFileSync(new URL('./task_results.json',import.meta.url),'utf8'));
const wrap=value=>({profile:'connect-control/1',result:{Ok:{kind:'task',data:value}}});
const rejects=code=>error=>error instanceof ConnectError&&error.code===code;
const helper=fileURLToPath(new URL('./helper_fixture.py',import.meta.url));
for (const [name,value] of Object.entries(fixtures.valid))
 test(`task result validates: ${name}`,()=>assert.deepEqual(decodeResponse(wrap(value),'task').data,value));
for (const [name,value] of Object.entries(fixtures.invalid))
 test(`task result rejects: ${name}`,()=>assert.throws(()=>decodeResponse(wrap(value),'task'),rejects('protocol')));
test('task wait extends only its bounded wait budget',()=>{
 const operation={op:'task',request:{command:{action:'wait',wait_ms:1000}}};
 assert.equal(exchangeTimeout(1000,operation),2000);
 for(const [value,expected] of [[20000,11000],[-1,1000],[true,1000],['1000',1000]]){
  operation.request.command.wait_ms=value;assert.equal(exchangeTimeout(1000,operation),expected);
 }
 assert.equal(exchangeTimeout(1000,{op:'health'}),1000);
});
test('native tasks require discovery, preserve IDs and allow bounded waits',async()=>{
 const client=new NativeClient('tasks',helper,400);
 try{
  await assert.rejects(()=>client.task({action:'get',task_id:'task-a'}),rejects('unsupported-profile'));
  await client.negotiateTasks('connect:agent@local','conversation-a');
  const pending=await client.task({action:'wait',task_id:'task-a',after:null,limit:1,wait_ms:500});
  assert.equal(pending.kind,'inbox');assert.deepEqual(pending.data.events,[]);
  const command={action:'reply',request_id:'stable-id',conversation_id:'conversation-a',reply_to:'question-a',body:'Answer'};
  const result=await client.task(command);assert.equal(result.data.request_id,'stable-id');assert.equal(command.request_id,'stable-id');
 }finally{await client.close();}
});
test('older server rejects tasks but ordinary control stays usable',async()=>{
 const client=new NativeClient('tasks-old',helper);
 try{await assert.rejects(()=>client.negotiateTasks('connect:agent@local'),rejects('unsupported-profile'));assert.equal((await client.health()).schema,7);}finally{await client.close();}
});
test('malformed extension cannot enable tasks',async()=>{
 const client=new NativeClient('tasks-malformed',helper);
 try{await assert.rejects(()=>client.negotiateTasks('connect:agent@local'),rejects('protocol'));}finally{await client.close();}
});
