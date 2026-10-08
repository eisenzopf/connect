import assert from 'node:assert/strict';
import {test} from 'node:test';
import {fileURLToPath} from 'node:url';
import {readFileSync,readdirSync} from 'node:fs';
import {createServer} from 'node:http';
import {NativeClient,HttpClient,ConnectError} from '../typescript/dist/index.js';
import {decodeResponse} from '../typescript/dist/protocol.js';
const helper=fileURLToPath(new URL('./helper_fixture.py',import.meta.url));
const rejects=(code)=>error=>error instanceof ConnectError&&error.code===code;
for(const mode of ['fragment','stall'])test(`native ${mode} obeys deadline and closes`,async()=>{
 const c=new NativeClient(mode,helper,400);const start=Date.now();
 try{await assert.rejects(()=>c.health(),rejects('transport-uncertain'));assert.ok(Date.now()-start<1500);await assert.rejects(()=>c.health(),rejects('transport-uncertain'));}finally{await c.close();await c.close();}
});
for(const mode of ['oversize','malformed','wrong-id','wrong-kind','bad-fields','invalid-utf8'])test(`native ${mode} rejects with protocol error`,async()=>{
 const c=new NativeClient(mode,helper);try{await assert.rejects(()=>c.health(),rejects('protocol'));await assert.rejects(()=>c.health(),rejects('transport-uncertain'));}finally{await c.close();}
});
test('chunked frames and concurrent calls serialize',async()=>{const c=new NativeClient('chunked',helper);try{const results=await Promise.all(Array.from({length:8},()=>c.health()));assert.ok(results.every(r=>r.installation_id==='fixture'));}finally{await c.close();}});
test('application denial does not close native client',async()=>{const c=new NativeClient('denied',helper);try{await assert.rejects(()=>c.health(),rejects('forbidden'));assert.equal((await c.health()).schema,6);}finally{await c.close();}});
test('missing executable settles close',async()=>{const c=new NativeClient('fixture','/no-such-connect-helper');await assert.rejects(()=>c.health(),rejects('transport-uncertain'));await c.close();});
test('invalid deadlines are rejected',()=>{for(const value of [0,-1,Infinity,NaN,60001])assert.throws(()=>new NativeClient('fixture',helper,value),RangeError);});
test('HTTP validation, bounds, deadline, no redirect/retry',async()=>{
 let mode='denied',count=0;
 const server=createServer((req,res)=>{count++;req.resume();res.writeHead(mode==='redirect'?302:mode==='denied'?403:200,{location:'/must-not-follow'});if(mode==='stall'){res.write('{');return;}res.end(mode==='oversize'?'x'.repeat(600001):mode==='malformed'?'{}':JSON.stringify({profile:'connect-control/1',result:{Err:{code:'forbidden',message:'fixture'}}}));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 try{for(const [next,code]of [['denied','forbidden'],['malformed','protocol'],['oversize','protocol'],['redirect','protocol'],['stall','transport-uncertain']]){mode=next;const before=count;await assert.rejects(()=>new HttpClient(`http://127.0.0.1:${server.address().port}`,'fixture',200).health(),rejects(code));assert.equal(count,before+1);}}
 finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
test('Rust result fixtures validate in TypeScript',()=>{const dir=new URL('../../contracts/fixtures/results/',import.meta.url);for(const name of readdirSync(dir)){const response=JSON.parse(readFileSync(new URL(name,dir),'utf8')).payload;const result=response.result;if(result.Err)assert.throws(()=>decodeResponse(response,'health'),rejects(result.Err.code));else assert.deepEqual(decodeResponse(response,result.Ok.kind),result.Ok);}});
