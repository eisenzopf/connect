import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixProgramme} from '../lib/browser_matrix_programme.mjs';
import {browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
const deferred=()=>{let resolve;const promise=new Promise(value=>resolve=value);return {promise,resolve};};
const flush=async()=>{for(let i=0;i<80;i++)await Promise.resolve();};
async function fixture({invalidReceipt=false,oldRefusal=false}={}){
 const originalClock=globalThis.performance,originalTimer=globalThis.setTimeout,originalDocument=globalThis.document;
 let now=0;const timers=[];globalThis.performance={now:()=>now};globalThis.setTimeout=(resolve,ms)=>{timers.push({at:now+ms,resolve});};
 globalThis.document={createElement:()=>({getContext:()=>({})})};
 const commit=deferred(),oldRead=deferred(),prepared=deferred(),journals=[],byOperation=new Map(),actors={};
 const plan={programmeMs:62000,sessions:[{id:'session-a'},{id:'session-b'}],routes:Array.from({length:6},(_,index)=>({index,session:index<4?0:1,
  role:`member-${index%4+1}`,audio:`audio-${index}`,video:`video-${index}`,connection:`initial-${index}`})),moves:[]};
 // Members0 and4 need distinct original control owners even though media IDs repeat.
 plan.routes[4].role='other-1';plan.routes[5].role='other-2';
 for(const [ordinal,at]of [[0,15000],[1,40000]])for(const index of [0,4])plan.moves.push({source:index,ordinal,scheduledStartMs:at,
  targetRole:ordinal===0?'target':plan.routes[index].role,operation:`move-${index}-${ordinal}`,audio:`move-audio-${index}-${ordinal}`,video:`move-video-${index}-${ordinal}`,receiveVideo:`move-receive-${index}-${ordinal}`});
 const sources={sources:plan.routes.map(route=>({index:route.index,stream:{getTracks:()=>[{readyState:'live'}]},attachments:[]})),
  beginProgramme:()=>({originMs:0,deadlineMs:62000}),async start({route,connection}){this.sources[route.index].attachments.push({connectionId:connection.id,capture:{closed:false}});},
  retireConnection(){},observation:()=>({})};
 const makeConnection=(id,index)=>({id,streams:{byId:id=>({id})},render:async()=>({close(){},finished:Promise.resolve()}),
  async replaceDevice(command,options){const intent=plan.moves.find(value=>value.operation===options.operationId),target=actors[intent.targetRole],replacement=makeConnection('target-'+intent.operation,index);
   const receipt={connection:{id:replacement.id},activationState:'ready',binding:{deviceId:target.deviceId,endpointId:invalidReceipt&&index===0?'wrong':target.endpointId},retired:[id],retained:[{id:command.retain}]};
   const completion=(async()=>{if(intent.ordinal===0)await commit.promise;const binding=sources.sources[index].attachments.find(value=>value.connectionId===id);binding.capture.closed=true;return {value:receipt};})();
   byOperation.set(intent.operation,{connection:replacement,completion});
   return {reference:{operation:intent.operation},referenceJson:()=>({operation:intent.operation}),wait:()=>completion};},
  async refresh(){if(id.startsWith('initial-')&&[0,4].includes(index)){await oldRead.promise;return {state:oldRefusal&&index===0?'active':'retired'};}return {state:'retired'};}});
 for(const role of [...plan.routes.map(route=>route.role),'target']){
  const session={refresh:async()=>({topologyRevision:1}),connections:{prepareReplacement:async({operation})=>{
   prepared.resolve();return byOperation.get(operation.operation);
  }}};
  actors[role]={deviceId:role+'-device',endpointId:role+'-endpoint',client:{conversations:{byId:()=>({sessions:{byId:()=>session}})}}};
 }
 const state={conversation:{id:'conversation'},connections:plan.routes.map(route=>makeConnection(route.connection,route.index))};
 for(const route of plan.routes)sources.sources[route.index].attachments.push({connectionId:route.connection,capture:{closed:false}});
 class Renderer{audio(){return true;}video(){return true;}}
 const observer=browserMatrixObservation(Renderer,plan),renderer=new Renderer(),report={};
 const work=browserMatrixProgramme({state,actors,plan,sources,journal:async value=>journals.push(value),keepalive:{check(){}},report,observer,selectedTransport:'webtransport'});
 const advance=async at=>{now=at;for(const timer of timers.splice(0).filter(timer=>timer.at<=at))timer.resolve();await flush();};
 const audio=(connectionId,timestamp)=>renderer.audio({connectionId,streamId:'move-audio-0-0'}, {sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp,
  copyTo:out=>out.set(Float32Array.from({length:960},(_,i)=>.7*Math.sin(2*Math.PI*1100*i/48000)))});
 try{
  await flush();await advance(15000);await prepared.promise;await flush();
  audio('target-move-0-0',0);assert.equal(observer.report().routes[0].audio.decoded_seconds,0,'Prepared private output stays excluded');
  commit.resolve();await flush();now=15020;audio('target-move-0-0',20000);
  const observed=observer.report().routes[0];
  assert.equal(observed.audio.decoded_seconds,invalidReceipt?0:.02,'Only a validated postcommit target is observed while old refresh is blocked');
  if(!invalidReceipt){assert.equal(observed.sync.audio_envelope.raw_threshold_transitions[0].at,15.02,'Keep actual receive callback time');
   audio('target-move-0-0',20000);assert.equal(observer.report().routes[0].audio.duplicate_frames,1,'Original duplicate detection remains strict');}
  oldRead.resolve();await flush();await advance(40000);await advance(62000);await work;
  if(invalidReceipt||oldRefusal)assert.ok(report.failures.some(value=>value.operation_id==='move-0-0'),'Verification failure remains a failed original move');
  else assert.equal(report.moves.length,4);
 }finally{commit.resolve();oldRead.resolve();await flush();await advance(40000);await advance(62000);await work.catch(()=>{});await observer.close();globalThis.performance=originalClock;globalThis.setTimeout=originalTimer;globalThis.document=originalDocument;}
}
test('validated target output is observed before a slow old-owner verification read',()=>fixture());
test('invalid ready receipt never activates prepared output',()=>fixture({invalidReceipt:true}));
test('old-owner verification refusal still fails after actual postcommit output is observed',()=>fixture({oldRefusal:true}));
