import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserTransportClient} from '../typescript/dist/browser_transport_client.js';
import {BrowserMediaPeer} from '../typescript/dist/browser_media.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
// Actual signed control codec/lifetime model, not network, TLS or media proof.
for(const transport of ['webtransport','websocket'])for(const algorithm of ['EdDSA','ES256'])
 test(`${algorithm} ${transport} original native ready timeout retains only its late-reply identity`,async()=>{
  let original;
  await fixture(async f=>{
   const client=await BrowserClient.open({...f.options,transport,url:transport==='webtransport'?'https://fixture.invalid/connect':f.options.url});
   const scope={cid:'room',sid:'session',connid:'target',operation_id:'move',expected_revision:0};
   try {
    await assert.rejects(client.requestReplacementContinuation('connection.offer',{
     replacement_preparation:{profile:'connect-replacement-ready-v1',operation_record_id:'record',readiness:{nonce:'nonce'}},
    },scope,{timeoutMs:30}),{code:'timeout'});
    assert.ok(original);assert.equal(client.closed,false);
    await original.peer.emit('operation.updated',{operation_id:'move',record_id:'record',command_type:'connection.replace',state:'preparing',revision:2},original.frame.id,original.frame);
    await original.peer.emit('ack',{},original.frame.id,original.frame);
    const result=await client.requestReplacementContinuation('operation.get',{operation_id:'move'},
      {cid:'room',sid:'session',connid:'source'},{timeoutMs:200});
    assert.equal(result.find(value=>value.type==='operation.updated').payload.state,'completed');
    assert.equal(client.closed,false);assert.equal(f.sent.filter(value=>value.type==='connection.offer').length,1);
    assert.equal(f.sent.filter(value=>value.type==='operation.get').length,1);
    assert.equal(transport==='webtransport'?f.transports.length:f.sockets.length,1);
   } finally {await client.shutdown();}
   assert.equal(client.closed,true);
   assert.equal(transport==='webtransport'?f.transports[0].didClose:f.sockets[0].readyState===3,true);
  },{algorithm,wtMode:transport==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
    if(frame.type==='connection.offer'){original={peer,frame};return true;}
    if(frame.type!=='operation.get')return false;
    await peer.emit('operation.updated',{operation_id:'move',record_id:'record',command_type:'connection.replace',state:'completed',revision:3},frame.id,frame);
    await peer.emit('ack',{},frame.id,frame);return true;
  }});
 });

// Real signed control and actual local Connection lifetime owners only. Data
// ports avoid fake decoder output; this does not establish real media quality.
const replacementReference=()=>({operationId:'original-source-move',command:'connection.replace',
 scope:{conversationId:'room',sessionId:'session-a',connectionId:'source-a'}});
const transportOptions=f=>({identityId:f.options.identityId,deviceId:f.options.device.id,
 endpoints:{webtransport:'https://fixture.invalid/connect'},preferredTransport:'webtransport',allowFallback:false,
 timeoutMs:500,attemptTimeoutMs:200,resolveEnrollment:async()=>f.options});
const controlTick=()=>new Promise(resolve=>setTimeout(resolve,0));
async function sourceSessionOwners(f,owner){
 const physical=owner.client;
 f.transports[0].datagrams.maxDatagramSize=1200;
 const peer=new BrowserMediaPeer(physical),closed=[0,0],connections=[];
 try{
  for(const [index,suffix]of ['a','b'].entries()){
   const connection=await peer.offerConnection({conversationId:'room',sessionId:'session-'+suffix,
    connectionId:'source-'+suffix,operationId:'source-offer-'+suffix,data:[{id:'source-data-'+suffix,protocol:'text'}]},{timeoutMs:500});
   connection.onClose(()=>closed[index]++);connections.push(connection);
  }
  return {physical,peer,closed,connections};
 }catch(error){await peer.shutdown();throw error;}
}
async function sourceOwnerControl({peer,frame}){
 if(frame.type==='connection.offer'){
  const offered=frame.payload.streams_offered[0];
  const parameters={by_participant:'participant',substrate:'webtransport',substrate_setup:{kind:'native-prepared/1',
   offer_id:frame.id,revision:1,streams:[{strm_id:offered.id,kind:'data',direction:'sendrecv',stream_local_id:frame.connid==='source-a'?11:12,
    codec:{name:'text',params:{reliability:'reliable_ordered'}}}]}};
  await peer.emit('connection.offer',parameters,undefined,{cid:frame.cid,sid:frame.sid,connid:frame.connid,operation_id:frame.operation_id});
  await peer.emit('connection.answer',{},frame.id,frame);return true;
 }
 if(frame.type==='operation.get'&&frame.payload.operation_id.startsWith('source-offer-')){
  await peer.emit('operation.updated',{operation_id:frame.payload.operation_id,record_id:'record-'+frame.payload.operation_id,
   command_type:'connection.offer',state:'completed',revision:1},frame.id,frame);
  await peer.emit('ack',{},frame.id,frame);return true;
 }
 return false;
}
for(const algorithm of ['EdDSA','ES256']){
 test(`${algorithm} native original replacement status timeout preserves two Session owners and safely recovers after its late reply`,async()=>{
  let original,denied=false;
  await fixture(async f=>{
   const owner=await BrowserTransportClient.connect(transportOptions(f)),owners=await sourceSessionOwners(f,owner),reference=replacementReference();
   const saved=structuredClone(reference);let lateEvents=0;
   const stop=owners.physical.onEvent(frame=>{if(frame.type==='operation.updated')lateEvents++;});
   try{
    await assert.rejects(owner.recoverOperation(reference,{timeoutMs:80}),{code:'timeout'});
    assert.ok(original);assert.equal(owner.client,owners.physical);assert.equal(owner.status.physicalEpoch,1);
    assert.equal(owners.physical.closed,false);assert.deepEqual(owners.closed,[0,0]);
    assert.ok(owners.connections.every(connection=>connection.revision===1));
    // A signed old response must be discarded by its original request ID; it
    // cannot complete a later status read or become an unsolicited event.
    await original.peer.emit('operation.updated',{operation_id:reference.operationId,record_id:'source-record',
     command_type:'connection.replace',state:'preparing',revision:2},original.frame.id,original.frame);
    await original.peer.emit('ack',{},original.frame.id,original.frame);await controlTick();
    assert.equal(lateEvents,0);assert.deepEqual(reference,saved);
    const current=await owner.recoverOperation(reference,{timeoutMs:200});
    assert.equal(current.operation_id,reference.operationId);assert.equal(current.state,'completed');assert.equal(current.revision,3);
    assert.deepEqual(owners.closed,[0,0]);assert.equal(owner.status.physicalEpoch,1);assert.equal(f.transports.length,1);
    denied=true;await assert.rejects(owner.recoverOperation(reference,{timeoutMs:200}),error=>error.code==='refused'&&error.problem?.code===403);
    assert.equal(owner.client,owners.physical);assert.deepEqual(owners.closed,[0,0]);
    const reads=f.sent.filter(frame=>frame.type==='operation.get'&&frame.payload.operation_id===reference.operationId);
    assert.equal(reads.length,3);assert.equal(new Set(reads.map(frame=>frame.id)).size,3);
    assert.ok(reads.every(frame=>frame.cid==='room'&&frame.sid==='session-a'&&frame.connid==='source-a'));
    assert.equal(f.sent.filter(frame=>frame.type==='connection.replace').length,0);
   }finally{stop();await owners.peer.shutdown();await owner.shutdown();}
  },{algorithm,wtMode:'ok',handler:async context=>{
   if(await sourceOwnerControl(context))return true;
   const {peer,frame}=context;if(frame.type!=='operation.get'||frame.payload.operation_id!=='original-source-move')return false;
   if(!original){original={peer,frame};return true;}
   if(denied){await peer.emit('error',{code:403,category:'auth',reason:'current-permission-refused'},frame.id,frame);return true;}
   await peer.emit('operation.updated',{operation_id:frame.payload.operation_id,record_id:'source-record',command_type:'connection.replace',state:'completed',revision:3},frame.id,frame);
   await peer.emit('ack',{},frame.id,frame);return true;
  }});
 });
 test(`${algorithm} native original replacement recovery still closes both Session owners on a protocol or physical failure`,async()=>{
  for(const mode of ['protocol','physical'])await fixture(async f=>{
   const owner=await BrowserTransportClient.connect(transportOptions(f)),owners=await sourceSessionOwners(f,owner);
   try{
    if(mode==='protocol')await assert.rejects(owner.recoverOperation(replacementReference(),{timeoutMs:200}),{code:'protocol'});
    else{f.transports[0].close();for(let i=0;i<100&&!owners.physical.closed;i++)await controlTick();}
    assert.equal(owners.physical.closed,true);assert.deepEqual(owners.closed,[1,1]);assert.equal(owner.status.state,'disconnected');
    assert.equal(f.transports.length,1);assert.equal(f.sent.filter(frame=>frame.type==='connection.replace').length,0);
   }finally{await owners.peer.shutdown();await owner.shutdown();}
  },{algorithm,wtMode:'ok',handler:async context=>{
   if(await sourceOwnerControl(context))return true;
   const {peer,frame}=context;if(frame.type!=='operation.get')return false;
   await peer.emit('operation.updated',{operation_id:frame.payload.operation_id,record_id:'source-record',command_type:'session.handoff',state:'completed',revision:3},frame.id,frame);
   await peer.emit('ack',{},frame.id,frame);return true;
  }});
 });
 test(`${algorithm} native uncertain mutation reply timeout keeps its existing physical retirement semantics`,async()=>{
  await fixture(async f=>{
   const owner=await BrowserTransportClient.connect(transportOptions(f)),owners=await sourceSessionOwners(f,owner);
   try{
    await assert.rejects(owner.client.request('connection.end',{reason_code:200,reason:'test-original-mutation'},
     {cid:'room',sid:'session-a',connid:'source-a',operation_id:'new-original-end'},{timeoutMs:80}),{code:'timeout'});
    assert.equal(owners.physical.closed,true);assert.deepEqual(owners.closed,[1,1]);assert.equal(f.transports.length,1);
    assert.equal(f.sent.filter(frame=>frame.type==='connection.end').length,1);
   }finally{await owners.peer.shutdown();await owner.shutdown();}
  },{algorithm,wtMode:'ok',handler:async context=>await sourceOwnerControl(context)||context.frame.type==='connection.end'});
 });
}
