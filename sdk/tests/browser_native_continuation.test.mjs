import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserClient} from '../typescript/dist/browser_client.js';
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
