import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,browserOperationKinds,sdkId,sdkRevision} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const opts={timeoutMs:300},one=first=>({first,rest:[]});
const binding=(extra={})=>({stream_id:'screen',medium:'screen',direction:'sendrecv',codec:{name:'vp8',params:{sample_rate:90000,channels:0,payload_type:96}},stream_local_id:2,stream_generation:1,connection_generation:1,...extra});
test('historical ICE receipt restores without a local WebRTC peer or control mutation',()=>scenario(async({client,f,records})=>{
 records.set('ice-original',{operation_id:'ice-original',command_type:'connection.update',state:'completed',revision:2,record_id:'ice-record',result:{action:'ice-restart',connection_id:'connection',connection_generation:4,stream_revision:7,resources_changed:false,ice_generation:3}});
 const op=client.operations.restore({schema:'connect-operation-reference/1',operation_id:'ice-original',command:'connection.update',scope:{conversation_id:'room',session_id:'session',connection_id:'connection'}},browserOperationKinds.connectionUpdate);
 const value=(await op.wait(opts)).value;assert.deepEqual(value.outcome,{kind:'ice-restart',iceGeneration:3});assert.equal(value.connectionGeneration,4);assert.equal(value.streamRevision,7);
 assert.equal(f.sent.filter(v=>v.type==='connection.offer'||v.type==='connection.update').length,0);
 await assert.rejects(()=>value.connection.capture({audio:true},opts),{kind:'receipt-unavailable'});
}));
async function scenario(run,{fault,large=false}={}){
 const records=new Map();let mode=fault;
 return fixture(async f=>{
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{'wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:'wss-webrtc',resolveEnrollment:async()=>f.options,timeoutMs:500});
  const connection=client.conversations.byId(sdkId('room')).sessions.byId(sdkId('session')).connections.byId(sdkId('connection'));
  try{return await run({client,connection,f,records,setFault:value=>mode=value});}finally{await client.close({timeoutMs:500});}
 },{handler:async({peer,frame})=>{
  const reply=async(type,payload)=>{await peer.emit(type,payload,frame.id,frame);await peer.emit('ack',{},frame.id,frame);return true;};
  if(frame.type==='operation.get'){
   const record=large?{operation_id:frame.payload.operation_id,command_type:'create',state:'completed',revision:1,record_id:'record-large',result:{kind:'conversation',data:{id:'room',receipt:{operation_id:frame.payload.operation_id},large:'x'.repeat(70000)}}}:records.get(frame.payload.operation_id);
   return reply('operation.updated',record);
  }
  if(!['stream.subscribe','stream.unsubscribe','connection.update'].includes(frame.type))return false;
  let result;
  if(frame.type==='connection.update'){
   if(frame.payload.action==='quality')result={connection_id:'connection',connection_generation:1,stream_revision:2,resources_changed:false,quality_interval_ms:frame.payload.quality_interval_ms};
   else if(frame.payload.action==='codec-renegotiate')result={connection_id:'connection',stream_revision:2,resources_changed:true,codec_bindings:[binding()]};
   else result={connection_id:'connection',connection_generation:1,stream_revision:2,resources_changed:false,action:frame.payload.action,gate_streams:[{binding:binding(),state:frame.payload.action==='hold'&&frame.payload.details.direction==='inactive'?'paused':'active',source_muted:frame.payload.action==='mute',held_direction:frame.payload.action==='hold'&&frame.payload.details.direction!=='inactive'?'recvonly':null}]};
  }else result={connection_id:'connection',command_type:frame.type,connection_generation:1,expected_revision:1,stream_revision:2,resources_changed:false,stream_bindings:mode==='null'?null:mode==='empty'?[]:[binding()]};
  if(mode==='foreign')result.connection_id='other';
  if(mode==='revision')result.stream_revision=0;
  if(mode==='missing')delete result.stream_bindings;
  if(mode==='generation')result.stream_bindings[0].connection_generation=2;
  if(mode==='alias')result.stream_bindings.push(binding({stream_id:'other'}));
  if(mode==='wrong-command')result.command_type='stream.reconcile';
  const record={operation_id:frame.operation_id,command_type:frame.type,state:'completed',revision:1,record_id:'record-'+frame.operation_id,result};records.set(frame.operation_id,record);return reply('operation.updated',record);
 }});
}
for(const command of ['subscribe','unsubscribe'])test('typed '+command+' captures original wire intent and restores only its scoped record',()=>scenario(async({connection,client,f})=>{
 const input=command==='subscribe'?{selectors:{first:{participant:sdkId('source'),kinds:one('screen')},rest:[{stream:sdkId('other-screen')}]},expectedStreamRevision:sdkRevision(1)}:{streams:one(sdkId('other-screen')),expectedStreamRevision:sdkRevision(1)};
 const pending=connection.streams[command](input,{...opts,operationId:sdkId('original')});if(command==='subscribe')input.selectors.first.kinds.first='video';else input.streams.first='changed';
 const operation=await pending;assert.equal(f.sent.filter(v=>v.type==='operation.get').length,0);assert.deepEqual(operation.reference.scope,{conversationId:'room',sessionId:'session',connectionId:'connection'});
 const saved=operation.referenceJson(),kind=command==='subscribe'?browserOperationKinds.streamSubscribe:browserOperationKinds.streamUnsubscribe;const value=(await client.operations.restore(saved,kind).wait(opts)).value;
 assert.equal(value.connection.id,'connection');assert.equal(value.streamRevision,2);assert.equal(value.bindings[0].parameters.medium,'screen');assert.equal(value.bindings[0].parameters.streamLocalId,2);assert(Object.isFrozen(value.bindings[0].parameters));assert.equal(f.sent.filter(v=>v.type==='stream.'+command).length,1);
 const wire=f.sent.find(v=>v.type==='stream.'+command);assert.equal(wire.expected_revision,1);assert.deepEqual(JSON.parse(JSON.stringify(wire.payload)),command==='subscribe'?{subscriptions:[{from_participant:'source',kinds:['screen']},{strm_id:'other-screen'}]}:{strm_ids:['other-screen']});
}));
for(const mode of ['null','empty'])test('selection '+mode+' binding capture is distinct and creates no media custody',()=>scenario(async({connection})=>{
 const op=await connection.streams.unsubscribe({streams:one(sdkId('source')),expectedStreamRevision:sdkRevision(1)},{...opts,operationId:sdkId('original')});const value=(await op.wait(opts)).value;assert.deepEqual(value.bindings,mode==='null'?null:[]);await assert.rejects(()=>value.connection.capture({audio:true},opts),{kind:'receipt-unavailable'});
},{fault:mode}));
for(const fault of ['foreign','revision','missing','generation','alias','wrong-command'])test('selection refuses malformed/foreign '+fault+' original receipt and retires control',()=>scenario(async({connection,client})=>{
 const op=await connection.streams.subscribe({selectors:one({participant:sdkId('source'),kinds:one('screen')}),expectedStreamRevision:sdkRevision(1)},{...opts,operationId:sdkId('original')});await assert.rejects(()=>op.wait(opts),{kind:'protocol',certainty:'recorded'});assert.equal(client.transport.state,'disconnected');
},{fault}));
for(const input of [{action:'mute',streams:one('screen')},{action:'unmute',streams:one('screen')},{action:'hold',direction:'inactive'},{action:'hold',direction:'sendonly',streams:one('screen')},{action:'resume'},{action:'quality',intervalMs:250},{action:'codec-renegotiate',codecPreferences:one('vp8')}])test('typed Connection '+input.action+' uses an observable original control with captured typed outcome',()=>scenario(async({connection,f})=>{
 const operation=await connection.update(input,sdkRevision(1),{...opts,operationId:sdkId('original')});assert.equal(f.sent.filter(v=>v.type==='operation.get').length,0);const value=(await operation.wait(opts)).value;assert.equal(value.connection.id,'connection');assert.equal(value.streamRevision,2);assert.equal(value.connectionGeneration,1);assert.equal(value.outcome.kind,input.action==='quality'?'quality':input.action==='codec-renegotiate'?'codecs':'gates');
 if(value.outcome.kind==='gates'){assert.equal(value.outcome.streams[0].sourceMuted,input.action==='mute');if(input.direction==='sendonly')assert.equal(value.outcome.streams[0].heldDirection,'recvonly');}
 assert.equal(f.sent.filter(v=>v.type==='connection.update').length,1);
}));
test('invalid/ambiguous selector or control input refuses locally without dropping requested fields',()=>scenario(async({connection,f})=>{
 const n=f.sent.length;
 for(const selectors of [[],{first:{stream:'x',participant:'p'},rest:[]},{first:{participant:'p',kinds:{first:'screen',rest:['screen']}},rest:[]},{first:{participant:'p'},rest:[{participant:'p',kinds:one('video')}]},{first:{stream:'x',kinds:one('screen')},rest:[]}])assert.throws(()=>connection.streams.subscribe({selectors,expectedStreamRevision:1},{...opts,operationId:sdkId('original')}),{kind:'validation'});
 for(const input of [{action:'mute',streams:one('x'),codecPreferences:one('vp8')},{action:'quality',intervalMs:0},{action:'quality',intervalMs:60001},{action:'hold',direction:'sendrecv'},{action:'resume',streams:{first:'x',rest:['x']}},{action:'codec-renegotiate',codecPreferences:{first:'vp8',rest:['vp8']}}])assert.throws(()=>connection.update(input,sdkRevision(1),{...opts,operationId:sdkId('original')}),{kind:'validation'});
 assert.equal(f.sent.length,n);
}));
test('browser original operation results retain the wire-size budget above the old64KiB copy bound',()=>scenario(async({client})=>{
 const operation=client.operations.restore({schema:'connect-operation-reference/1',operation_id:'large',command:'conversation.create',scope:{}},browserOperationKinds.conversationCreate);assert.equal((await operation.wait(opts)).value.id,'room');
},{large:true}));
