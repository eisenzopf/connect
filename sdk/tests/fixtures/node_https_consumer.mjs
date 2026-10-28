// Synthetic local TLS consumer. Trust is installed before this process starts.
import assert from 'node:assert/strict';
import {CanonicalHttpClient,CanonicalError,HttpClient,ConnectError} from '../../typescript/dist/index.js';
const spec=JSON.parse(process.env.CONNECT_HTTPS_FIXTURE),options={timeoutMs:1500},scope={cid:'room',operation_id:'original-https'},key={operationId:'original-https',command:'message.send',scope:{conversationId:'room'}};
const client=new CanonicalHttpClient({endpoint:spec.endpoint,credential:'synthetic-https-credential',transport:'remote-https'});
let failure;
try {
 if(spec.mode==='success') {
  const payload={text:'original'},pending=client.request('message.send',payload,scope,options);payload.text='changed';const frames=await pending;assert.equal(frames.at(-1).operation_id,'original-https');
  const read=await client.request('operation.get',{operation_id:'original-https'},{cid:'room'},options);assert.equal(read[0].payload.record_id,'record-original');
  const app=new HttpClient(spec.endpoint,'synthetic-https-credential',1500,'remote-https');
  await assert.rejects(app.messageReceipts('room','message'),e=>{assert.ok(e instanceof ConnectError);assert.equal(e.code,'forbidden');assert.equal(e.httpStatus,403);return true;});
 } else {
  const abort=new AbortController(),input={timeoutMs:spec.mode==='deadline'?100:1500,signal:abort.signal};
  const pending=client.request('message.send',{text:'original'},scope,input);
  if(spec.mode==='cancel')setTimeout(()=>abort.abort(),100);
  await assert.rejects(pending,e=>{
   assert.ok(e instanceof CanonicalError);assert.equal(e.kind,spec.kind);assert.equal(e.certainty,spec.certainty);assert.deepEqual(e.operation,key);
   assert.ok(!e.message.includes('synthetic-https-credential'));failure={kind:e.kind,certainty:e.certainty};return true;
  });
 }
} finally {await client.close(options);}
process.stdout.write(JSON.stringify({passed:true,mode:spec.mode,failure})+'\n');
