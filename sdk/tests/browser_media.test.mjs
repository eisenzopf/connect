import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaPeer} from '../typescript/dist/browser_media.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
import {RtpSender,decodeRtpDatagram} from '../typescript/dist/browser_rtp.js';
import {BrowserMediaRenderer} from '../typescript/dist/browser_media_renderer.js';

class Control {
  closed=false;events=new Set();lifetimes=new Set();sent=[];index=0;
  datagrams={maxDatagramSize:1200,readable:new ReadableStream(),writable:new WritableStream()};
  constructor(mode){this.mode=mode;}
  onEvent(fn){this.events.add(fn);return()=>this.events.delete(fn);}
  onClose(fn){this.lifetimes.add(fn);return()=>this.lifetimes.delete(fn);}
  close(){if(this.closed)return;this.closed=true;for(const fn of [...this.lifetimes])fn(Error('closed'));}
  begin(type,payload,scope){
    const id='wire_'+ ++this.index;let finish;
    const response=type==='connection.offer'?Promise.resolve([envelope('connection.answer',{}, {...scope,in_reply_to:id})]):new Promise(r=>{finish=r;});
    queueMicrotask(()=>{
      const changed=this.mode==='scope'?{...scope,connid:'foreign'}:scope;
      const frame=envelope('connection.offer',{by_participant:'participant_test',substrate:'webtransport',substrate_setup:{kind:'native-prepared/1',revision:scope.expected_revision+1,offer_id:this.mode==='offer'?'foreign':id,streams:[{strm_id:'data',kind:'data',direction:'sendrecv',stream_local_id:scope.expected_revision+1,codec:{name:'binary',params:{reliability:'reliable_ordered'}}}]}},changed);
      if(this.mode==='receiver'&&type==='stream.subscribe')frame.payload.substrate_setup.streams.push({strm_id:'remote_data',kind:'data',direction:'sendonly',stream_local_id:9,codec:{name:'binary',params:{reliability:'reliable_ordered'}}});
      if(this.clockProfile)frame.payload.capabilities={source_clock:this.clockProfile===true?'rtcp-sr-sdes/1':this.clockProfile};
      if(this.mode==='audio')frame.payload.substrate_setup.streams=[{strm_id:'audio',kind:'audio',direction:'sendrecv',stream_local_id:1,codec:{name:'opus',params:{sample_rate:48000,channels:1,payload_type:111}}}];
      this.confirm=()=>finish?.([envelope('ack',{}, {...scope,in_reply_to:id})]);
      for(const fn of this.events)fn(frame);
    });
    return{id,response};
  }
  async send(type,payload,scope){this.sent.push({type,payload,scope});this.confirm();}
  async request(type,payload,scope){this.sent.push({type,payload,scope});return[envelope('ack',{},scope)];}
  async operationStatus(cid,op){return{operation_id:op,state:this.mode==='failure'?'failed':'completed'};}
}
const input={conversationId:'room',sessionId:'session',connectionId:'connection',operationId:'offer',data:[{id:'data',protocol:'binary'}]};
for(const failure of [false,true])test(`native media shutdown joins renderer audio cleanup${failure?' and reports its failure':''}`,async()=>{
 const originals=['AudioContext','document'].map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);let finish;
 const held=new Promise((resolve,reject)=>{finish=failure?()=>reject(Error('synthetic cleanup failure')):resolve;});
 class Node{gain={value:1};connect(){}disconnect(){}}
 class Context{sampleRate=48000;state='running';destination={};createGain(){return new Node();}createAnalyser(){return new Node();}async resume(){}close(){return held;}}
 Object.defineProperty(globalThis,'AudioContext',{configurable:true,value:Context});Object.defineProperty(globalThis,'document',{configurable:true,value:{visibilityState:'visible',addEventListener(){},removeEventListener(){}}});
 const client=new Control(),peer=new BrowserMediaPeer(client);
 try{
  const connection=await peer.offerConnection(input),renderer=await BrowserMediaRenderer.open(connection);let joined=false;
  const closing=peer.shutdown().finally(()=>joined=true);void closing.catch(()=>{});await new Promise(setImmediate);
  assert.equal(joined,false);assert.equal(client.closed,true);assert.equal(peer.diagnostics.activeBindings,0);
  finish();if(failure){await assert.rejects(closing,/synthetic cleanup failure/);await assert.rejects(renderer.finished,/synthetic cleanup failure/);}else{await closing;await renderer.finished;}
  assert.equal(client.datagrams.readable.locked,false);assert.equal(client.datagrams.writable.locked,false);
 }finally{finish();peer.close();for(const[n,d]of originals)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});
test('video ownership ends exactly once on missing sender and retired Connection',async()=>{
  const client=new Control(),peer=new BrowserMediaPeer(client);let closed=0;
  const frame=()=>({close(){closed++;}});
  try{
    const connection=await peer.offerConnection(input);
    assert.throws(()=>connection.encodeVideo('missing',frame()),{code:'unsupported-media-profile'});
    assert.equal(closed,1);
    await connection.end('end-video-owner');
    assert.throws(()=>connection.encodeVideo('missing',frame()),{code:'closed'});
    assert.equal(closed,2);
    peer.close();
    assert.throws(()=>connection.encodeVideo('missing',frame()),{code:'closed'});
    assert.equal(closed,3);
  }finally{peer.close();}
});
test('media control binds native parameters, confirms once and completes from status without imaginary root ACK',async()=>{
  const client=new Control(),received=[];const peer=new BrowserMediaPeer(client,{data:m=>received.push(m)});
  try{
    const connection=await peer.offerConnection(input);assert.equal(connection.revision,1);
    assert.equal(client.sent.length,1);assert.equal(client.sent[0].type,'connection.answer');assert.equal(client.sent[0].scope.operation_id,'offer');
    await connection.subscribe('subscription','other',['data']);assert.equal(connection.revision,2);
    await connection.sendData('publication','original-message','data',{type:'binary',value:new Uint8Array([0,255,8])});
    assert.equal(client.sent.at(-1).payload.body,'AP8I');assert.equal(client.sent.at(-1).payload.msg_id,'original-message');
    const message=envelope('message.send',{msg_id:'message_from_other',from:'other',label:'data',content_type:'application/octet-stream',body:'AP8I',body_encoding:'base64'},{cid:'room',sid:'session',connid:'connection',operation_id:'other-operation'});
    for(const fn of client.events)fn(message);
    assert.deepEqual(received[0].body.value,new Uint8Array([0,255,8]));assert.equal(received[0].messageId,'message_from_other');
    await connection.end('terminal');assert.throws(()=>connection.revision,{code:'closed'});
    const end=client.sent.find(x=>x.type==='connection.end');
    assert.equal(end.scope.connid,'connection');assert.equal(end.scope.expected_revision,undefined);
    assert.equal(peer.diagnostics.activeBindings,0);
  }finally{peer.close();}
  assert.equal(client.closed,true);assert.equal(client.events.size,0);assert.equal(client.lifetimes.size,0);
});
for(const mode of ['scope','offer'])test('media refuses signed parameter '+mode+' mismatch before confirmation and retires physical peer',async()=>{
  const c=new Control(mode),p=new BrowserMediaPeer(c);
  try{await assert.rejects(()=>p.offerConnection(input),{code:'protocol'});assert.equal(c.sent.length,0);assert.equal(c.closed,true);}
  finally{p.close();}
});
test('operation failure cannot become usable media and reliable data preserves bounds/negotiated format',async()=>{
  const c=new Control(),p=new BrowserMediaPeer(c);try{
    const connection=await p.offerConnection(input);
    await assert.rejects(()=>connection.sendData('data-op','message','data',{type:'text',value:'wrong codec'}),{code:'invalid-request'});
    await assert.rejects(()=>connection.sendData('data-op','message','data',{type:'binary',value:new Uint8Array(65537)}),{code:'invalid-request'});
    c.mode='failure';await assert.rejects(()=>connection.sendData('data-op','message','data',{type:'binary',value:new Uint8Array([1])}),{code:'refused'});
  }finally{p.close();}
});
test('queued command body and RTP binding retain the original intention despite caller mutation',()=>{
  const payload={nested:{value:'original'}},frame=envelope('message.send',payload);payload.nested.value='changed';assert.equal(frame.payload.nested.value,'original');
  const binding={handle:1,streamId:'audio',connectionId:'connection',sessionId:'session',generation:1,kind:'audio',codec:'opus',payloadType:111,direction:'sendrecv'};
  const sender=new RtpSender(binding);binding.handle=0;binding.payloadType=96;
  const packet=decodeRtpDatagram(sender.packetize(new Uint8Array([1]),0)[0]);assert.equal(packet.handle,1);assert.equal(packet.payloadType,111);
});
test('managed server reconciliation preserves selected receiver ports and cannot create another publisher',async()=>{
  const c=new Control('receiver'),p=new BrowserMediaPeer(c);try{
    const connection=await p.offerConnection(input);await connection.subscribe('select-source','other',['data']);
    const setup={kind:'native-prepared/1',revision:3,offer_id:'server-wire',streams:[
      {strm_id:'data',kind:'data',direction:'sendrecv',stream_local_id:2,codec:{name:'binary',params:{reliability:'reliable_ordered'}}},
      {strm_id:'remote_data',kind:'data',direction:'sendonly',stream_local_id:9,codec:{name:'binary',params:{reliability:'reliable_ordered'}}}]};
    for(const fn of c.events)fn(envelope('connection.offer',{by_participant:'participant_test',substrate:'webtransport',substrate_setup:setup},{cid:'room',sid:'session',connid:'connection',operation_id:'reconcile_source'}));
    await connection.sendData('stable-message-op','stable-message','data',{type:'binary',value:new Uint8Array([5])});
    assert.equal(connection.revision,3);assert.ok(c.sent.some(x=>x.type==='connection.answer'&&x.scope.operation_id==='reconcile_source'));
    await assert.rejects(()=>connection.sendData('foreign-publish','foreign-message','remote_data',{type:'binary',value:new Uint8Array([1])}),{code:'invalid-request'});
    setup.revision=4;setup.offer_id='new-server-wire';setup.streams[1].direction='recvonly';
    for(const fn of c.events)fn(envelope('connection.offer',{by_participant:'participant_test',substrate:'webtransport',substrate_setup:setup},{cid:'room',sid:'session',connid:'connection',operation_id:'bad-publisher'}));
    await assert.rejects(()=>connection.sendData('next-op','next-message','data',{type:'binary',value:new Uint8Array([5])}),{code:'protocol'});
    assert.equal(c.closed,true);
  }finally{p.close();}
});
test('bounded media read bursts yield to codec output tasks before dropping decoded audio',async()=>{
  const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
  let decoded=0,closed=0,resolve;const done=new Promise(r=>{resolve=r;});
  class Encoder {static async isConfigSupported(){return{supported:true};}state='unconfigured';encodeQueueSize=0;configure(){this.state='configured';}close(){this.state='closed';}}
  class Decoder extends Encoder {
    decodeQueueSize=0;constructor(callbacks){super();this.callbacks=callbacks;}
    decode(chunk){this.decodeQueueSize++;setTimeout(()=>{this.decodeQueueSize--;if(this.state!=='closed')this.callbacks.output({timestamp:chunk.timestamp,sampleRate:48000,numberOfChannels:1,numberOfFrames:960,close(){closed++;}});},0);}
  }
  const values=[Encoder,Decoder,class{},class{constructor(init){Object.assign(this,init);}}];
  for(let i=0;i<names.length;i++)Object.defineProperty(globalThis,names[i],{configurable:true,value:values[i]});
  const client=new Control('audio');let ingress;client.datagrams.readable=new ReadableStream({start:c=>{ingress=c;}});
  const peer=new BrowserMediaPeer(client,{audio:()=>{if(++decoded===64)resolve();}});let timeout;
  try{
    await peer.offerConnection({...input,data:undefined,audio:'audio'});
    const sender=new RtpSender({handle:1,streamId:'audio',connectionId:'connection',sessionId:'session',generation:1,kind:'audio',codec:'opus',payloadType:111,direction:'sendrecv'});
    for(let n=0;n<64;n++)ingress.enqueue(sender.packetize(new Uint8Array([1]),n*960)[0]);
    await Promise.race([done,new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('codec tasks received '+decoded+' of 64 burst frames')),500);})]);
    assert.equal(decoded,64);assert.equal(closed,64);assert.equal(peer.diagnostics.received,64);
  }finally{clearTimeout(timeout);peer.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});

test('media peer sends current capture reports only after RTP writes and receives clocks without media readiness counts',async()=>{
  const {wrapRtcpSenderReport,isRtcpDatagram,decodeRtcpSenderReport}=await import('../typescript/dist/browser_rtcp.js');
  const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk'];const old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
  class Encoder {static async isConfigSupported(){return{supported:true};} state='configured';encodeQueueSize=0;constructor(c){this.c=c;}configure(){}close(){this.state='closed';}encode(frame){queueMicrotask(()=>this.c.output({timestamp:frame.timestamp,type:'key',byteLength:1,copyTo:b=>b.set([9])}));}}
  class Decoder extends Encoder {decodeQueueSize=0;decode(chunk){queueMicrotask(()=>this.c.output({timestamp:chunk.timestamp,sampleRate:48000,numberOfChannels:1,numberOfFrames:960,close(){}}));}}
  for(const [i,value] of [Encoder,Decoder,class{constructor(x){Object.assign(this,x);}close(){}},class{constructor(x){Object.assign(this,x);}}].entries())Object.defineProperty(globalThis,names[i],{configurable:true,value});
  const client=new Control('audio');client.clockProfile=true;let ingress;const writes=[];
  client.datagrams.readable=new ReadableStream({start:c=>{ingress=c;}});
  client.datagrams.writable=new WritableStream({write:b=>{writes.push(b.slice());}});
  let decoded;const audio=new Promise(r=>{decoded=r;});const peer=new BrowserMediaPeer(client,{audio:frame=>decoded(frame.timestamp)});
  try{
    const connection=await peer.offerConnection({...input,data:undefined,audio:'audio'});
    const sender=new RtpSender({handle:1,streamId:'audio',connectionId:'connection',sessionId:'session',generation:1,kind:'audio',codec:'opus',payloadType:111,direction:'sendrecv'});
    const packet=sender.packetize(new Uint8Array([1]),0)[0];const parsed=decodeRtpDatagram(packet);
    const report=wrapRtcpSenderReport(1,7,{ssrc:parsed.ssrc,ntp:100n<<32n,rtpTimestamp:0,packets:1,octets:1,cname:'server-common-clock'});
    ingress.enqueue(report);ingress.enqueue(packet);
    await Promise.race([audio,new Promise((_,reject)=>setTimeout(()=>reject(Error('clocked audio not decoded')),500))]);
    assert.equal(peer.diagnostics.received,1);assert.equal(peer.routeDiagnostics[0].counters.clockReports,1);assert.equal(peer.routeDiagnostics[0].counters.synchronized,1);
    assert.equal(connection.encodeAudio('audio',new Float32Array(960),0),true);
    for(let n=0;n<100&&writes.length<2;n++)await new Promise(r=>setTimeout(r,1));
    assert.equal(writes.length,2);assert.equal(isRtcpDatagram(writes[0]),false);assert.equal(isRtcpDatagram(writes[1]),true);
    const sr=decodeRtcpSenderReport(writes[1].subarray(8));assert.equal(sr.packets,1);assert.equal(sr.octets,1);assert.equal(sr.ssrc,decodeRtpDatagram(writes[0]).ssrc);assert.match(sr.cname,/^connect-/);
    await connection.end('clock-end');const before=peer.diagnostics.rejected;ingress.enqueue(report);
    await new Promise(r=>setTimeout(r,10));assert.equal(peer.diagnostics.rejected,before+1);assert.equal(writes.length,2,'retired clock timer sent idle metadata');
    const bad=new Control('audio');bad.clockProfile='foreign-clock/2';const badPeer=new BrowserMediaPeer(bad);try{await assert.rejects(()=>badPeer.offerConnection({...input,data:undefined,audio:'audio'}),{code:'protocol'});}finally{badPeer.close();}
  }finally{peer.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
});

test('begin offer exposes accepted current parameters separately from recorded completion and installed resources',async()=>{
 const c=new Control(),p=new BrowserMediaPeer(c);let complete,observed=false,ready=false;
 c.operationStatus=()=>{observed=true;return new Promise(resolve=>complete=()=>resolve({state:'completed'}));};
 try{
  const begun=p.beginOfferConnection(input,{timeoutMs:500});void begun.ready.then(()=>ready=true);
  await begun.accepted;assert.equal(begun.submissionAttempted,true);assert.equal(ready,false);
  while(!observed)await new Promise(resolve=>setTimeout(resolve,1));assert.equal(ready,false);complete();
  const connection=await begun.ready;assert.equal(connection.revision,1);assert.equal(c.sent.filter(x=>x.type==='connection.answer').length,1);
 }finally{p.close();}
});
for(const mode of ['deadline','abort'])test('native offer codec preflight '+mode+' is bounded before submission and cannot reset the offer budget',async()=>{
 const names=['AudioEncoder','AudioDecoder','AudioData'],saved=names.map(n=>Object.getOwnPropertyDescriptor(globalThis,n));
 try{
  for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,writable:true,value:class {static isConfigSupported(){return new Promise(()=>{});}}});
  const c=new Control(),p=new BrowserMediaPeer(c),abort=new AbortController();
  try{
   const begun=p.beginOfferConnection(input,{timeoutMs:50,signal:abort.signal});if(mode==='abort')setTimeout(()=>abort.abort(),5);
   await assert.rejects(begun.accepted,{code:mode==='deadline'?'timeout':'cancelled'});await assert.rejects(begun.ready,{code:mode==='deadline'?'timeout':'cancelled'});
   assert.equal(begun.submissionAttempted,false);assert.equal(c.sent.length,0);assert.equal(c.index,0);
  }finally{p.close();}
 }finally{names.forEach((n,i)=>saved[i]?Object.defineProperty(globalThis,n,saved[i]):delete globalThis[n]);}
});

test('retirement after accepted parameters prevents late completion from reinstalling local media and preserves the physical peer',async()=>{
 const c=new Control(),p=new BrowserMediaPeer(c);let complete,observed=false;
 c.operationStatus=()=>{observed=true;return new Promise(resolve=>complete=()=>resolve({state:'completed'}));};
 try{
  const begun=p.beginOfferConnection(input,{timeoutMs:500});await begun.accepted;
  while(!observed)await new Promise(resolve=>setTimeout(resolve,1));p.retireLocalConnection('connection');complete();
  await assert.rejects(begun.ready,{code:'closed'});assert.equal(c.closed,false);assert.equal(p.diagnostics.activeBindings,0);assert.throws(()=>p.revision('connection'),{code:'closed'});
 }finally{p.close();}
});

test('retirement before prepared parameters arrives prevents any late confirmation and leaves other physical control usable',async()=>{
 const c=new Control(),p=new BrowserMediaPeer(c),begin=c.begin.bind(c);
 c.begin=(...args)=>{const started=begin(...args);p.retireLocalConnection('connection');return started;};
 try{
  const begun=p.beginOfferConnection(input,{timeoutMs:500});await assert.rejects(begun.accepted,{code:'closed'});await assert.rejects(begun.ready,{code:'closed'});
  assert.equal(c.sent.length,0);assert.equal(c.closed,false);assert.equal(p.diagnostics.activeBindings,0);
 }finally{p.close();}
});

test('begin offer captures original scope and stream intent before asynchronous codec preflight',async()=>{
 const c=new Control(),p=new BrowserMediaPeer(c),draft=structuredClone(input),begin=c.begin.bind(c),offered=[];
 c.begin=(...args)=>{offered.push({scope:structuredClone(args[2]),payload:structuredClone(args[1])});return begin(...args);};
 try{
  const begun=p.beginOfferConnection(draft,{timeoutMs:500});draft.operationId='replacement';draft.connectionId='foreign';draft.conversationId='foreign-room';draft.data[0].id='foreign-source';draft.data[0].protocol='text';
  const connection=await begun.ready;assert.equal(connection.id,'connection');assert.equal(offered.length,1);assert.equal(offered[0].scope.operation_id,'offer');assert.equal(offered[0].scope.cid,'room');assert.equal(offered[0].scope.connid,'connection');assert.equal(offered[0].payload.streams_offered[0].id,'data');assert.deepEqual(offered[0].payload.streams_offered[0].codec_preferences,['binary']);
 }finally{p.close();}
});
class TypedControl extends Control {
 records=new Map();begun=[];hold=false;
 begin(type,payload,scope){
  this.begun.push({type,payload,scope});
  if(type==='connection.update'){
   const id='typed_'+ ++this.index;
   const result={connection_id:scope.connid,stream_revision:scope.expected_revision+(payload.action==='quality'?0:1),resources_changed:false,...(payload.action==='quality'?{quality_interval_ms:payload.quality_interval_ms}:{action:payload.action})};
   this.records.set(scope.operation_id,result);
   return{id,response:Promise.resolve([envelope('ack',{}, {...scope,in_reply_to:id})])};
  }
  if(type==='stream.subscribe')this.records.set(scope.operation_id,{connection_id:scope.connid,stream_revision:scope.expected_revision+1,resources_changed:true});
  return super.begin(type,payload,scope);
 }
 async operationStatus(cid,op){return this.records.has(op)?{operation_id:op,state:this.hold?'unknown':'completed',result:this.records.get(op)}:super.operationStatus(cid,op);}
}
test('typed WT gate and correlated subscription preserve original keys and keep the local revision current for the next mutation',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client);
 try{
  const connection=await peer.offerConnection(input),body={action:'mute',streams:['data']};
  const gate=peer.beginControl(connection.id,'gate-original','connection.update',body,1,{timeoutMs:300});body.streams[0]='changed';await gate.response;await gate.ready;assert.equal(connection.revision,2);assert.equal(client.begun.find(v=>v.type==='connection.update').payload.streams[0],'data');
  const selection=peer.beginControl(connection.id,'select-original','stream.subscribe',{subscriptions:[{from_participant:'other',kinds:['data']}]},2,{timeoutMs:300});await selection.response;await selection.ready;assert.equal(connection.revision,3);
  assert.equal(client.begun.filter(v=>v.type==='stream.subscribe').length,1);assert.equal(client.begun.find(v=>v.type==='stream.subscribe').scope.expected_revision,2);assert.equal(client.sent.filter(v=>v.type==='connection.answer'&&v.scope.operation_id==='select-original').length,1);assert.equal(client.closed,false);
 }finally{peer.close();}
});
test('typed WT queued control deadline cannot reset behind another original control or submit late intent',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client);
 try{
  const connection=await peer.offerConnection(input);client.hold=true;
  const first=peer.beginControl(connection.id,'first-control','connection.update',{action:'mute',streams:['data']},1,{timeoutMs:300});await first.response;
  const queued=peer.beginControl(connection.id,'queued-control','connection.update',{action:'unmute',streams:['data']},2,{timeoutMs:15});await assert.rejects(queued.response,{code:'timeout'});await assert.rejects(queued.ready,{code:'timeout'});assert.equal(queued.submissionAttempted,false);assert.equal(client.begun.some(v=>v.scope.operation_id==='queued-control'),false);assert.equal(client.closed,false);
  client.hold=false;await first.ready;assert.equal(connection.revision,2);
  const next=peer.beginControl(connection.id,'fresh-control','connection.update',{action:'quality',quality_interval_ms:250},2,{timeoutMs:100});await next.ready;assert.equal(connection.revision,2);assert.equal(client.begun.filter(v=>v.type==='connection.update').length,2);
 }finally{peer.close();}
});
test('typed WT stale control revision refuses before submission and leaves the current owner usable',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client);
 try{const connection=await peer.offerConnection(input);const stale=peer.beginControl(connection.id,'stale','connection.update',{action:'mute',streams:['data']},0,{timeoutMs:100});await assert.rejects(stale.response,{code:'conflict'});await assert.rejects(stale.ready,{code:'conflict'});assert.equal(stale.submissionAttempted,false);assert.equal(client.begun.length,1);assert.equal(client.closed,false);assert.equal(connection.revision,1);}finally{peer.close();}
});

test('target-owned native handoff preparation is captured once and mixed media refuses before IO',async()=>{
 const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk'],old=names.map(n=>[n,Object.getOwnPropertyDescriptor(globalThis,n)]);
 class Codec {static async isConfigSupported(){return {supported:true};}state='unconfigured';configure(){this.state='configured';}close(){this.state='closed';}}
 for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:Codec});
 const client=new Control('audio'),peer=new BrowserMediaPeer(client),offer={conversationId:'room',sessionId:'session',connectionId:'connection',operationId:'prepare',audio:'audio',purpose:'handoff-preparation'};
 client.operationStatus=async()=>({operation_id:'prepare',state:'completed',result:{stream_revision:2,handoff_preparation:{profile:'connect-handoff-preparation-v1',prepared_muted:true}}});
 let original;const begin=client.begin;client.begin=function(type,payload,...args){if(type==='connection.offer'&&payload.substrate==='webtransport')original=structuredClone(payload);return begin.call(this,type,payload,...args);};
 try{const pending=peer.offerConnection(offer);offer.purpose='forged';const prepared=await pending;assert.equal(prepared.revision,2);
 assert.deepEqual(original.handoff_preparation,{profile:'connect-handoff-preparation-v1'});
 for(const result of [{stream_revision:1,handoff_preparation:{profile:'connect-handoff-preparation-v1',prepared_muted:true}},{stream_revision:2,handoff_preparation:{profile:'connect-handoff-preparation-v1',prepared_muted:false}},{stream_revision:2}]){
  const client=new Control('audio'),peer=new BrowserMediaPeer(client);client.operationStatus=async()=>({operation_id:'prepare',state:'completed',result});
  try{await assert.rejects(()=>peer.offerConnection({...offer,purpose:'handoff-preparation'}),{code:'protocol'});assert.equal(client.closed,true);}finally{peer.close();}
 }

 }finally{peer.close();for(const[n,d]of old)if(d)Object.defineProperty(globalThis,n,d);else delete globalThis[n];}
 for(const patch of [{purpose:'forged'},{video:'video'},{screen:'screen'},{data:[{id:'data',protocol:'binary'}]},{audio:undefined}]){const client=new Control('audio'),peer=new BrowserMediaPeer(client);try{await assert.rejects(()=>peer.offerConnection({...offer,purpose:'handoff-preparation',...patch}),{code:'invalid-request'});assert.equal(client.sent.length,0);}finally{peer.close();}}
});

test('typed WT later observed control assertion waits for the original completed result before advancing local custody',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client);
 try{
  const connection=await peer.offerConnection(input);client.hold=true;
  const control=peer.beginControl(connection.id,'after-audience','connection.update',{action:'mute',streams:['data']},5,{timeoutMs:500});
  await control.response;assert.equal(control.submissionAttempted,true);assert.equal(connection.revision,1);assert.equal(client.begun.at(-1).scope.expected_revision,5);
  client.hold=false;await control.ready;assert.equal(connection.revision,6);assert.equal(client.closed,false);
  const stale=peer.beginControl(connection.id,'old-receipt','connection.update',{action:'unmute',streams:['data']},5,{timeoutMs:100});await assert.rejects(stale.ready,{code:'conflict'});assert.equal(stale.submissionAttempted,false);assert.equal(client.begun.filter(v=>v.type==='connection.update').length,1);
 }finally{peer.close();}
});
test('typed WT resource negotiation binds its signed counteroffer to the later asserted server revision',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client);
 try{
  const connection=await peer.offerConnection(input),selection=peer.beginControl(connection.id,'after-route-change','stream.subscribe',{subscriptions:[{from_participant:'other',kinds:['data']}]},7,{timeoutMs:300});
  await selection.ready;assert.equal(connection.revision,8);assert.equal(client.begun.at(-1).scope.expected_revision,7);assert.equal(client.sent.filter(v=>v.type==='connection.answer'&&v.scope.operation_id==='after-route-change').length,1);assert.equal(client.closed,false);
 }finally{peer.close();}
});
test('typed WT server refusal of a future assertion preserves the original physical revision without retry',async()=>{
 const client=new TypedControl(),peer=new BrowserMediaPeer(client),status=client.operationStatus.bind(client);
 client.operationStatus=(cid,op)=>op==='future-conflict'?Promise.resolve({operation_id:op,state:'failed',problem:{code:'conflict'}}):status(cid,op);
 try{
  const connection=await peer.offerConnection(input),control=peer.beginControl(connection.id,'future-conflict','connection.update',{action:'mute',streams:['data']},5,{timeoutMs:300});
  await control.response;await assert.rejects(control.ready,{code:'refused'});assert.equal(control.submissionAttempted,true);assert.equal(client.begun.filter(v=>v.type==='connection.update').length,1);assert.equal(connection.revision,1);assert.equal(client.closed,false);
 }finally{peer.close();}
});
