import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {NativeClient,HttpClient,ConnectError}=await import(process.env.CONNECT_TYPESCRIPT_SDK ? pathToFileURL(process.env.CONNECT_TYPESCRIPT_SDK).href : '../typescript/dist/index.js');
const [path,url,executable]=process.argv.slice(2);
// Test credentials are supplied over stdin, never through a process argument.
let data='';for await(const chunk of process.stdin)data+=chunk;
const http=new HttpClient(url,JSON.parse(data).credential);
const native=new NativeClient(path,executable);
try{
 assert.deepEqual(await native.health(),await http.health());
 const peer=await native.resolve('connect:agent@local');
 const c=await native.create('typescript-create',[peer.id]);
 assert.deepEqual(c,await http.create('typescript-create',[peer.id]));
 const r=await native.send('typescript-send',c.id,'TypeScript over QUIC');
 assert.deepEqual(r,await http.send('typescript-send',c.id,'TypeScript over QUIC'));
 assert.deepEqual(await native.history(c.id),await http.history(c.id));
 assert.equal((await http.history(c.id)).messages.length,1);
 assert.equal((await native.requestStatus('typescript-send')).state,'accepted');
 assert.deepEqual((await native.providerStatus(c.id)).deliveries,[]);
 await assert.rejects(()=>http.send('typescript-send',c.id,'changed'),e=>e instanceof ConnectError&&e.code==='conflict');
 await assert.rejects(()=>new HttpClient(url,'invalid').health(),e=>e instanceof ConnectError&&e.code==='unauthorized');
 console.log('TypeScript native QUIC and HTTP SDK parity passed');
}finally{await native.close();}
