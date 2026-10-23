import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createServer} from 'node:http';
import {HttpClient} from '../typescript/dist/index.js';
test('HTTP send requires its exact local mutation reference and refuses bad receipt without retry',async()=>{
 const valid={request_id:'logical-message',message_id:'message-one',conversation_id:'room-one',receipt:{operation_id:'logical-message',conversation_id:'room-one',state:'completed',completion_boundary:'local-mutation',status_reference:{action:'get',operation_id:'logical-message'}}};
 const values=Array.from({length:5},()=>structuredClone(valid));delete values[1].receipt;
 values[2].receipt.status_reference.operation_id='other-message';values[3].receipt.status_reference.action='cancel';values[4].receipt.completion_boundary='provider-delivered';
 const requests=[];const server=createServer(async(req,res)=>{const chunks=[];for await(const chunk of req)chunks.push(chunk);requests.push(JSON.parse(Buffer.concat(chunks)));res.setHeader('content-type','application/json');res.end(JSON.stringify({profile:'connect-control/1',result:{Ok:{kind:'accepted',data:values[requests.length-1]}}}));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 try{const client=new HttpClient(`http://127.0.0.1:${server.address().port}`,'isolated-fixture');assert.deepEqual(await client.send('logical-message','room-one','message'),valid);for(let i=1;i<values.length;i++)await assert.rejects(client.send('logical-message','room-one','message'),e=>e.code==='protocol');assert.equal(requests.length,5);assert.deepEqual([...new Set(requests.map(r=>r.request_id))],['logical-message']);}
 finally{await new Promise(resolve=>server.close(resolve));}
});
