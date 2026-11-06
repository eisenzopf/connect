import assert from 'node:assert/strict';
import {test} from 'node:test';
import {preserveRenderRetirement} from '../typescript/dist/browser_render_retirement.js';
const receipt=id=>Object.freeze({connectionId:id,scope:'renderer-local',audioGraph:'closed',outputsDetached:true});
test('SDK retirement preservation waits for the exact original result and returns its immutable receipt',async()=>{
 let resolve;const original=new Promise(yes=>{resolve=yes;}),result=preserveRenderRetirement('original',original);let settled=false;
 void result.then(()=>settled=true);await new Promise(setImmediate);assert.equal(settled,false);
 const physical=receipt('original');resolve(physical);assert.equal(await result,physical);
});
test('SDK retirement preservation propagates actual cleanup failure without a receipt',async()=>{
 const failure=Error('original graph close rejected');await assert.rejects(preserveRenderRetirement('original',Promise.reject(failure)),error=>error===failure);
});
for(const foreign of [receipt('foreign'),Object.freeze({...receipt('original'),outputsDetached:false}),{...receipt('original')}])
 test('SDK retirement preservation refuses foreign or incomplete or mutable cleanup metadata '+JSON.stringify(foreign),async()=>{
  await assert.rejects(preserveRenderRetirement('original',Promise.resolve(foreign)),{code:'protocol'});
 });
