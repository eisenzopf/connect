import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {NodeClient,NodeSdkError,sdkRevision} from '../typescript/dist/index.js';
const root=fileURLToPath(new URL('../../',import.meta.url)),executable=fileURLToPath(new URL('./fixtures/sdk_helper.mjs',import.meta.url));
const opts={timeoutMs:1500},mutation={...opts,operationId:'original'},revision=sdkRevision(2);
const caps={audio:['g.711-mu']};
const invite=()=>({sessionId:'session',to:[],medium:'voice',intent:'synthetic',capabilities:caps});
const reference=(command='session.invite',scope={conversation_id:'room',session_id:'session'})=>({schema:'connect-operation-reference/1',operation_id:'original',command,scope});
const calls=async dir=>(await readFile(join(dir,'requests.jsonl'),'utf8').catch(()=>'')).trim().split('\n').filter(Boolean).map(JSON.parse);
const error=(kind,certainty)=>e=>{assert.ok(e instanceof NodeSdkError);assert.equal(e.kind,kind);assert.equal(e.certainty,certainty);return true;};
async function fixture(config,run){
 const base=join(root,'.local/gate-6/node-session-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'unit-')),path=join(dir,'config.json');await writeFile(path,JSON.stringify(config),{mode:0o600});
 const client=new NodeClient({config:path,executable});try{await run(client,dir);}finally{await client.close(opts);const pid=Number(await readFile(join(dir,'pid'),'utf8').catch(()=>'0'));if(pid)assert.throws(()=>process.kill(pid,0),e=>e.code==='ESRCH');await rm(dir,{recursive:true,force:true});}
}
test('typed Session controls preserve scoped original operations, topology assertions and decision result types',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room'),source=invite(),submitted=room.sessions.invite(source,mutation);source.intent='changed';
  const operation=await submitted,saved=operation.referenceJson(),session=(await operation.wait(opts)).value;
  assert.equal(session.id,'session');assert.equal(session.conversationId,'room');assert.throws(()=>session.id='foreign');
  assert.equal((await session.refresh(opts)).topologyRevision,2);assert.equal((await client.operations.restore(saved,client.operationKinds.session_invite).wait(opts)).value.id,'session');
  await (await session.inviteParticipants({to:{first:'participant',rest:[]},medium:'voice',intent:'synthetic',capabilities:caps,expectedTopologyRevision:revision},{...opts,operationId:'existing-invite'})).wait(opts);
  await (await session.accept({capabilities:caps,expectedTopologyRevision:revision},{...opts,operationId:'accept'})).wait(opts);
  for(const [i,input]of [{action:'participant-leave',participantId:'participant'},{action:'role-change',participantId:'participant',role:'observer'},{action:'medium-upgrade',media:{first:'audio',rest:['video']}},{action:'codec-renegotiate',codecPreferences:{first:'g.711-mu',rest:['vp8']}}].entries())
   await (await session.update(input,revision,{...opts,operationId:'update-'+i})).wait(opts);
  const decisionInput={reasonCode:486,reason:'unavailable',expectedTopologyRevision:revision};
  assert.equal((await (await session.reject(decisionInput,{...opts,operationId:'reject'})).wait(opts)).value.state,'rejected');
  assert.equal((await (await session.cancelInvitation({...decisionInput,reasonCode:487},{...opts,operationId:'cancel'})).wait(opts)).value.state,'cancelled');
  assert.equal((await (await session.end({...decisionInput,reasonCode:1000},{...opts,operationId:'end'})).wait(opts)).value.id,'session');
  const requests=await calls(dir),controls=requests.filter(r=>r.request.action.type==='session'&&r.request.action.action.type!=='refresh');
  assert.equal(controls[0].request.action.action.intent,'synthetic');assert.equal(controls[0].request.action.action.expected_revision,undefined);
  assert.ok(controls.slice(1).every(r=>r.request.action.action.expected_revision===2));assert.ok(controls.every(r=>r.request.action.conversation_id==='room'&&r.request.action.session_id==='session'));
 });
});
test('invalid Session target/revision/update and restoration shapes never write helper input',async()=>{
 await fixture({},async(client,dir)=>{
  const room=client.conversations.byId('room'),session=room.sessions.byId('session');
  for(const input of [null,{...invite(),sessionId:'bad!'},{...invite(),to:['bad!']},{...invite(),medium:'phone'}])assert.throws(()=>room.sessions.invite(input,mutation),error('validation','not-submitted'));
  assert.throws(()=>session.inviteParticipants({to:{first:'',rest:[]},medium:'voice',intent:'join',capabilities:caps,expectedTopologyRevision:revision},mutation),error('validation','not-submitted'));
  for(const input of [{action:'unknown'},{action:'role-change',participantId:'bad!',role:'observer'},{action:'medium-upgrade',media:{first:'camera',rest:[]}},{action:'medium-upgrade',media:{first:Object.create(null),rest:[]}},{action:'codec-renegotiate',codecPreferences:{first:'',rest:[]}}])assert.throws(()=>session.update(input,revision,mutation),error('validation','not-submitted'));
  assert.throws(()=>session.end({reasonCode:65536,reason:'bad',expectedTopologyRevision:revision},mutation),error('validation','not-submitted'));
  assert.throws(()=>session.end({reasonCode:1000,reason:'bad',expectedTopologyRevision:1.5},mutation),error('validation','not-submitted'));
  for(const scope of [{},{conversation_id:'room'},{conversation_id:'room',session_id:'session',connection_id:'connection'}])assert.throws(()=>client.operations.restore(reference('session.invite',scope),client.operationKinds.session_invite),error('validation','not-submitted'));
  assert.equal((await calls(dir)).length,0);
 });
});
test('Session results, current snapshots and decision metadata cannot cross their original scope',async()=>{
 for(const action of ['refresh','wait'])await fixture({mode:'wrong-session'},async client=>{
  const session=client.conversations.byId('room').sessions.byId('session');
  const p=action==='refresh'?session.refresh(opts):client.operations.restore(reference(),client.operationKinds.session_invite).wait(opts);
  await assert.rejects(p,error('protocol','read-only'));
 });
 await fixture({mode:'wrong-session-decision'},async client=>{
  const op=client.operations.restore(reference('session.reject'),client.operationKinds.session_reject);
  await assert.rejects(op.wait(opts),error('protocol','read-only'));
 });
});
