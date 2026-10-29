import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {inspect} from 'node:util';
import {NodeClient,NodeSdkError,SdkRtpPacket,SdkTextData,SdkJsonData,SdkBinaryData} from '../typescript/dist/index.js';
import {MediaFrameReader,encodeFrame} from '../typescript/dist/node_media_channel.js';
const helper=fileURLToPath(new URL('./fixtures/sdk_media_helper.py',import.meta.url));
const options={timeoutMs:1000};
const scope={conversationId:'room',sessionId:'session',connectionId:'connection'};
const error=(kind,certainty,id)=>e=>{assert(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);if(id)assert.equal(e.operation.operationId,id);return true;};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function fixture(mode,body){
 const dir=await mkdtemp(join(tmpdir(),'connect-node-media-')),path=join(dir,'config.json'),release=join(dir,'release');
 await writeFile(path,JSON.stringify({mode,pids:join(dir,'pids'),capture:join(dir,'capture'),release}));
 const client=new NodeClient({config:path,executable:helper});
 const calls=async()=>{try{return (await readFile(join(dir,'capture'),'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse);}catch{return [];}};
 const seen=async(action,count=1)=>{const expires=performance.now()+1000;while((await calls()).filter(r=>r.channel==='media' && r.request.command.action===action).length<count){assert(performance.now()<expires,'original request not observed');await sleep(5);}};
 try{await body(client,{dir,calls,seen,release:()=>writeFile(release,'ready')});}
 finally{
  await writeFile(release,'ready');await client.close({timeoutMs:2000});
  const pids=(await readFile(join(dir,'pids'),'utf8')).trim().split('\n');
  for(const pid of pids)assert.throws(()=>process.kill(Number(pid),0),{code:'ESRCH'});
  await rm(dir,{recursive:true,force:true});
 }
}
const stream=client=>client.media.connection(scope).streamById('audio');
function rtp(){const h=Buffer.alloc(12);h[0]=0x80;h[1]=0x80;h.writeUInt16BE(65535,2);h.writeUInt32BE(0xffffffff,4);h.writeUInt32BE(9,8);return Buffer.concat([h,Buffer.from('original-packet')]);}
const send=(client,payload=new SdkBinaryData(Buffer.from('original')),id='original',opts=options)=>client.media.connection(scope).streamById('application').sendData({messageId:'message',to:{first:'recipient',rest:[]},payload},{...opts,operationId:id});

test('C6M2 fragmented original boundaries and exact high-bit magic refusal before body allocation',()=>{
 const expected=encodeFrame(0x82,2,{type:'synthetic'},Buffer.alloc(65536,0xa5)),frames=[],failures=[];
 const reader=new MediaFrameReader(f=>frames.push(f),k=>failures.push(k));
 for(let i=0;i<expected.length;i+=7)reader.push(expected.subarray(i,i+7));
 assert.equal(frames.length,1);assert.equal(frames[0].payload.length,65536);assert.equal(frames[0].payload[65535],0xa5);assert.equal(failures.length,0);
 for(const field of [0,4,5,9,13]){
  const header=Buffer.from(expected.subarray(0,17));if(field===0)header[0]|=0x80;else if(field===4)header[4]=0x7f;else header.fill(field===5?0:255,field,field+4);
  let failed;const bad=new MediaFrameReader(()=>assert.fail('Invalid header accepted'),k=>failed=k);bad.push(header);assert.equal(failed,'protocol');
 }
 let failure;const truncated=new MediaFrameReader(()=>assert.fail(),k=>failure=k);truncated.push(expected.subarray(0,20));truncated.end();assert.equal(failure,'protocol');
});
test('captured RTP and typed data are immutable copies with bounded safe inspection',()=>{
 const bytes=rtp(),packet=new SdkRtpPacket(0xffffffff,bytes);bytes.fill(0);const exported=packet.bytes;exported.fill(1);
 assert.equal(packet.header.sequenceNumber,65535);assert.equal(packet.header.timestamp,0xffffffff);assert.equal(Buffer.from(packet.payload).toString(),'original-packet');
 const binary=new SdkBinaryData(Buffer.from('private'));binary.value.fill(0);assert.equal(Buffer.from(binary.value).toString(),'private');assert(!inspect(binary).includes('private'));
 const value={nested:{value:'private'}},json=new SdkJsonData(value);value.nested.value='changed';assert.equal(json.value.nested.value,'private');assert.throws(()=>json.value.nested.value='changed');assert(!inspect(json).includes('private'));
 for(const make of [()=>new SdkTextData('x'.repeat(65537)),()=>new SdkTextData('\ud800'),()=>new SdkBinaryData(Buffer.alloc(65537)),()=>new SdkJsonData({big:'x'.repeat(65536)}),()=>new SdkJsonData(Infinity),()=>new SdkRtpPacket(1,Buffer.alloc(12))])assert.throws(make,error('validation','not-submitted'));
});
test('Node packet negotiation exact binary RTP clock and full u64 stats keep control separate',async()=>{
 await fixture('normal',async(client,{calls})=>{
  const c=client.conversations.byId('room').sessions.byId('session').connections.byId('connection');
  const selected=await c.media.refresh(options);assert.equal(selected.streams[0].streamLocalId,7);assert.throws(()=>selected.streams[0].codec.params.sample_rate=16000);
  const packet=new SdkRtpPacket(7,rtp()),s=c.streams.byId('audio').media;
  assert.equal((await s.sendRtp(packet,options)).boundary,'local-media-queue');assert.deepEqual((await s.receivePacket(options)).bytes,packet.bytes);
  await s.sendClock(8,{ssrc:9,ntp:2n**64n-1n,rtp:2,packets:3,octets:4,cname:'synthetic'},options);
  assert.equal((await client.media.stats(options)).receivedDatagrams,2n**64n-1n);
  assert((await client.capabilities.refresh(options)).features.has('native-programmatic-media'));
  assert((await calls()).filter(r=>r.channel==='control').every(r=>!Object.hasOwn(r.action,'payload')));
 });
});
test('Node full64KiB text JSON binary and empty typed values preserve original data recovery',async()=>{
 await fixture('normal',async(client,{calls})=>{
  const data=client.media.connection(scope).streamById('application');
  for(const payload of [new SdkTextData('🦀'+'x'.repeat(65532)),new SdkJsonData({data:'x'.repeat(65525)}),new SdkBinaryData(Buffer.alloc(65536,0xa5)),new SdkTextData(''),new SdkBinaryData(Buffer.alloc(0)),new SdkJsonData(null)]){
   const operation=await send(client,payload),saved=operation.referenceJson();
   assert.throws(()=>client.operations.restore(saved,client.operationKinds.message_send),error('validation','not-submitted'));
   const restored=client.operations.restore(saved,client.operationKinds.data_send),done=await restored.wait(options);
   assert.equal(done.value.boundary,'core-queue-admission');assert.equal(done.value.messageId,'message');assert.equal(done.snapshot.recordId,'original-record');
   const received=await data.receiveData(options);assert.equal(received.messageId,'message');assert.equal(received.operationId,'original');assert.equal(received.fromParticipant,'sender');assert.equal(received.payload.kind,payload.kind);
   if(payload.kind==='binary')assert.deepEqual(received.payload.value,payload.value);else assert.deepEqual(received.payload.value,payload.value);
  }
  const captures=(await calls()).filter(r=>r.channel==='media' && r.request.command.action==='send_data');assert.deepEqual(captures.slice(0,3).map(r=>Buffer.from(r.payload,'base64').length),[65536,65536,65536]);
  assert.equal((await client.media.dataStats(options)).acceptedMessages,2n**64n-1n);
 });
});
test('Node cancelled data send preserves uncertain original key without mutation replay or helper restart',async()=>{
 await fixture('data-send-held',async(client,{seen,release,calls,dir})=>{
  const abort=new AbortController(),pending=send(client,undefined,'original',{...options,signal:abort.signal});await seen('send_data');abort.abort();
  await assert.rejects(pending,error('cancelled','uncertain','original'));await client.capabilities.refresh(options);await release();await sleep(20);
  assert.equal((await calls()).filter(r=>r.channel==='media' && r.request.command.action==='send_data').length,1);
  assert.equal((await readFile(join(dir,'pids'),'utf8')).trim().split('\n').length,1);
 });
});
test('Node cancelled data receive retains one original result across cloned handles and control stays live',async()=>{
 await fixture('data-receive-held',async(client,{seen,release,calls})=>{
  const a=new AbortController(),s=client.media.connection(scope).streamById('application'),pending=s.receiveData({...options,signal:a.signal});await seen('receive_data');
  await assert.rejects(client.media.connection(scope).streamById('application').receiveData(options),error('conflict','read-only'));
  a.abort();await assert.rejects(pending,error('cancelled','read-only'));await client.capabilities.refresh(options);await release();
  assert.equal((await s.receiveData(options)).messageId,'original-message');assert.equal((await calls()).filter(r=>r.channel==='media' && r.request.command.action==='receive_data').length,1);
 });
});
test('Node media version refusal and pre-cancel retain original intent before attachment',async()=>{
 await fixture('old-helper',async(client,{calls})=>{
  const abort=AbortSignal.abort();await assert.rejects(send(client,undefined,'never',{...options,signal:abort}),error('cancelled','not-submitted','never'));
  await assert.rejects(send(client),error('unsupported','not-submitted','original'));
  assert(!(await client.capabilities.refresh(options)).features.has('native-programmatic-media'));assert(!(await calls()).some(r=>r.channel==='control' && r.action.type==='media_attach'));
 });
});
test('Node recorded errors retain original snapshot while foreign references errors and observations fence media only',async()=>{
 for(const mode of ['data-refused','data-recorded','data-foreign-reference','data-foreign-error','data-foreign-observation'])await fixture(mode,async(client)=>{
  await assert.rejects(send(client),e=>{
   if(mode==='data-refused')error('authentication','refused','original')(e);else if(mode==='data-recorded'){error('application','recorded','original')(e);assert.equal(e.observation.recordId,'original-record');assert.throws(()=>e.observation.state='completed');}
   else error('protocol','uncertain','original')(e);return true;
  });await client.capabilities.refresh(options);
 });
});
test('Node invalid received bytes and foreign bindings fence reads without consuming control',async()=>{
 for(const mode of ['data-bad-utf8','data-bad-json','data-foreign-binding'])await fixture(mode,async(client)=>{
  await assert.rejects(client.media.connection(scope).streamById('application').receiveData(options),error('protocol','read-only'));await client.capabilities.refresh(options);
 });
});
test('Node channel window refuses seventeenth mutation and close settles original uncertain keys',async()=>{
 await fixture('data-send-held',async(client,{seen})=>{
  await client.media.dataStats(options);const pending=Array.from({length:16},(_,i)=>send(client,undefined,'op-'+i));for(const p of pending)void p.catch(()=>{});
  await seen('send_data',16);await assert.rejects(send(client,undefined,'excess'),error('application','not-submitted','excess'));
  await client.capabilities.refresh(options);await client.close({timeoutMs:2000});
  const results=await Promise.allSettled(pending);results.forEach((r,i)=>{assert.equal(r.status,'rejected');error('cancelled','uncertain','op-'+i)(r.reason);});
 });
});
test('Node helper death wakes a sent original data mutation and owned close joins its process',async()=>{
 await fixture('die-media',async client=>{await assert.rejects(send(client),error('transport-uncertain','uncertain','original'));});
});
test('Node fragmented packet response preserves original bytes and closes its listener with the socket owner',async()=>{
 await fixture('fragmented',async client=>{const packet=await stream(client).receivePacket(options);assert.equal(Buffer.from(packet.payload).toString(),'synthetic-packet');});
});

test('Node sent data deadline keeps the captured deadline and no original replay',async()=>{
 await fixture('data-send-deadline',async(client,{release,calls})=>{
  await client.media.dataStats(options);const changing={timeoutMs:40};const pending=send(client,undefined,'deadline',changing);changing.timeoutMs=60000;
  await assert.rejects(pending,error('deadline','uncertain','deadline'));await client.capabilities.refresh(options);await release();await sleep(20);
  assert.equal((await calls()).filter(r=>r.channel==='media' && r.request.command.action==='send_data').length,1);
 });
});
test('Node cancelled data receive slots bound original retained payloads independently of the request window',async()=>{
 await fixture('data-receive-held',async(client,{seen,release,calls})=>{
  await send(client,new SdkBinaryData(Buffer.alloc(65536,0xa5)));
  const streams=Array.from({length:17},(_,i)=>client.media.connection(scope).streamById('data-'+i)),abort=new AbortController();
  const waiting=streams.slice(0,16).map(s=>s.receiveData({...options,signal:abort.signal}));for(const p of waiting)void p.catch(()=>{});
  await seen('receive_data',16);abort.abort();for(const result of await Promise.allSettled(waiting)){assert.equal(result.status,'rejected');error('cancelled','read-only')(result.reason);}
  await release();const expires=performance.now()+1000;
  for(;;){try{await client.media.stats(options);break;}catch(e){error('application','read-only')(e);assert(performance.now()<expires);await sleep(5);}}
  await assert.rejects(streams[16].receiveData(options),error('application','read-only'));
  const values=await Promise.all(streams.slice(0,16).map(s=>s.receiveData(options)));assert(values.every(v=>v.payload.value.length===65536));
  assert.equal((await calls()).filter(r=>r.channel==='media' && r.request.command.action==='receive_data').length,16);
  assert.equal((await streams[16].receiveData(options)).payload.value.length,65536);
 });
});
test('Node malformed RTP header and foreign submitted binding fence media with correct uncertainty',async()=>{
 await fixture('bad-header',async client=>{await assert.rejects(stream(client).receivePacket(options),error('protocol','read-only'));await client.capabilities.refresh(options);});
 await fixture('foreign-reply',async client=>{await assert.rejects(stream(client).sendRtp(new SdkRtpPacket(1,rtp()),options),error('protocol','uncertain'));await client.capabilities.refresh(options);});
});
test('Node close owns an attachment still opening after the caller deadline',async()=>{
 await fixture('normal',async client=>{
  await assert.rejects(send(client,undefined,'opening',{timeoutMs:1}),e=>{assert(e instanceof NodeSdkError);assert.equal(e.kind,'deadline');assert.equal(e.operation.operationId,'opening');assert.equal(e.certainty,'not-submitted');return true;});
  await client.close({timeoutMs:2000});
 });
});
