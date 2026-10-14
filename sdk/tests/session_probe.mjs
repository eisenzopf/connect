// Public typed Session methods; credentials enter through stdin only.
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {NativeClient,HttpClient,ConnectError}=await import(process.env.CONNECT_TYPESCRIPT_SDK ? pathToFileURL(process.env.CONNECT_TYPESCRIPT_SDK).href : '../typescript/dist/index.js');
const [ownerPath,peerPath,url,helper]=process.argv.slice(2);
let input='';for await(const chunk of process.stdin)input+=chunk;
const inputs=JSON.parse(input);
const originalWrite=process.stderr.write;
let diagnostics='';
process.stderr.write=function(chunk,encoding,callback){
 diagnostics+=Buffer.isBuffer(chunk)?chunk.toString('utf8'):String(chunk);
 if(diagnostics.length>600000)throw new Error('Helper diagnostics exceeded bound');
 const done=typeof encoding==='function'?encoding:callback;if(done)done();return true;
};
const native={owner:new NativeClient(ownerPath,helper),peer:new NativeClient(peerPath,helper)};
const http={owner:new HttpClient(url,inputs.owner),peer:new HttpClient(url,inputs.peer)};
function invoke(client,o) {
 switch(o.op) {
  case 'session_invite':return client.sessionInvite(o.request_id,o.conversation_id,o.participant_id,o.media);
  case 'session_accept':return client.sessionAccept(o.request_id,o.conversation_id,o.session_id);
  case 'session_reject':return client.sessionReject(o.request_id,o.conversation_id,o.session_id,o.reason);
  case 'session_cancel':return client.sessionCancel(o.request_id,o.conversation_id,o.session_id);
  case 'session_end':return client.sessionEnd(o.request_id,o.conversation_id,o.session_id,o.reason);
  case 'session_get':return client.sessionGet(o.conversation_id,o.session_id);
  default:throw new Error('Unexpected Session method');
 }
}
try {
 for(const clients of [native,http])for(const record of inputs.records) {
  if(record.error)await assert.rejects(()=>invoke(clients[record.actor],record.operation),e=>e instanceof ConnectError&&e.code===record.error);
  else assert.deepEqual(await invoke(clients[record.actor],record.operation),record.outcome.data);
 }
 const generated=[],cid=inputs.conversation_id,participant=inputs.participant_id;
 for(const [lane,clients,retries] of (inputs.replay_only ? [] : [['native',native,http],['http',http,native]])) {
  const prefix='typescript-av-'+lane;
  async function execute(actor,operation) {
   const actual=await invoke(clients[actor],operation);
   assert.deepEqual(actual,await invoke(retries[actor],operation));
   assert.deepEqual(actual.media,['audio','video']);
   generated.push({actor,operation,outcome:{kind:'session',data:actual}});
   return actual;
  }
  const invite=await execute('owner',{op:'session_invite',request_id:prefix+'-invite',conversation_id:cid,participant_id:participant,media:['audio','video']});
  assert.equal(invite.state,'invited');
  const accepted=await execute('peer',{op:'session_accept',request_id:prefix+'-accept',conversation_id:cid,session_id:invite.id});
  assert.equal(accepted.state,'accepted');
  const ended=await execute('owner',{op:'session_end',request_id:prefix+'-end',conversation_id:cid,session_id:invite.id,reason:'SDK video complete'});
  assert.equal(ended.state,'ended');
  assert.deepEqual(await execute('peer',{op:'session_get',conversation_id:cid,session_id:invite.id}),ended);
 }
 console.log(JSON.stringify(generated));
} finally {
 await native.owner.close();await native.peer.close();
 process.stderr.write=originalWrite;
 assert(![inputs.owner,inputs.peer].some(secret=>diagnostics.includes(secret)),'Helper leaked a credential');
}
