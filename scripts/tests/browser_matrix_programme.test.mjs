import {test} from 'node:test';
import assert from 'node:assert/strict';
import {browserMatrixProgramme} from '../lib/browser_matrix_programme.mjs';
import {browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
import {nativeIdentity,bindNativeStreams,nativeBoundary} from '../../sdk/tests/native_activation_fixture.mjs';
const deferred=()=>{let resolve;const promise=new Promise(value=>resolve=value);return {promise,resolve};};
const flush=async()=>{for(let i=0;i<80;i++)await Promise.resolve();};
async function fixture({invalidReceipt=false,oldRefusal=false,keepaliveFailure=false,snapshotRefusal=false}={}){
 const originalClock=globalThis.performance,originalTimer=globalThis.setTimeout,originalDocument=globalThis.document;
 let now=0;const timers=[],timeOrigin=1_800_000_000_000;globalThis.performance={now:()=>now,timeOrigin};globalThis.setTimeout=(resolve,ms)=>{timers.push({at:now+ms,resolve});};
 globalThis.document={createElement:()=>({getContext:()=>({})})};
 const commit=deferred(),oldRead=deferred(),prepared=deferred(),journals=[],byOperation=new Map(),renderers=new Map(),actors={};
 const reachabilityError=Object.assign(Error('Original reachability failure'),{kind:'transport-uncertain',certainty:'read-only'});
 const snapshotError=Object.assign(Error('Original snapshot refusal'),{kind:'authentication',certainty:'read-only'});
 const plan={programmeMs:62000,sessions:[{id:'session-a'},{id:'session-b'}],routes:Array.from({length:6},(_,index)=>({index,session:index<4?0:1,
  role:`member-${index%4+1}`,audio:`audio-${index}`,video:`video-${index}`,connection:`initial-${index}`})),moves:[]};
 // Members0 and4 need distinct original control owners even though media IDs repeat.
 plan.routes[4].role='other-1';plan.routes[5].role='other-2';
 for(const [ordinal,at]of [[0,15000],[1,40000]])for(const index of [0,4])plan.moves.push({source:index,ordinal,scheduledStartMs:at,
  targetRole:ordinal===0?'target':plan.routes[index].role,operation:`move-${index}-${ordinal}`,audio:`move-audio-${index}-${ordinal}`,video:`move-video-${index}-${ordinal}`,receiveVideo:`move-receive-${index}-${ordinal}`});
 const sources={sources:plan.routes.map(route=>({index:route.index,ownerLive:()=>true,attachments:[]})),
  beginProgramme:()=>({originMs:0,deadlineMs:62000}),async start({route,connection}){this.sources[route.index].attachments.push({connectionId:connection.id,capture:{closed:false}});},
  retireConnection(){},observation:()=>({})};
 // The fake renderer has an explicit original physical identity and cut. It
 // models custody only; it performs no real decoding or codec qualification.
 const makeConnection=(id,index)=>({id,streams:{byId:id=>({id})},async render(){
  const renderer=new Renderer();renderers.set(id,renderer);
  if(id.startsWith('target-')){
   const entry=byOperation.get(id.slice('target-'.length)),handle=32*(entry.intent.ordinal+1)+index*4+1;
   const streams=bindNativeStreams([{strm_id:entry.intent.audio,kind:'audio',direction:'sendrecv',stream_local_id:handle},
    {strm_id:entry.intent.receiveVideo,kind:'video',direction:'sendonly',stream_local_id:handle+1}],entry.intent.ordinal+2);
   const boundary=nativeBoundary({recordId:'record-'+entry.intent.operation,operationId:entry.intent.operation,
    sourceConnectionId:entry.source,targetConnectionId:id},streams,entry.intent.ordinal+2,3,
    {[entry.intent.audio]:1,[entry.intent.receiveVideo]:1});
   Object.defineProperty(renderer,'nativeActivationBoundary',{get:()=>entry.completed?boundary:null});
   audio(id,0,0);entry.preparedIO.resolve();
  }
  return {close(){renderers.delete(id);},finished:Promise.resolve()};},
  async replaceDevice(command,options){const intent=plan.moves.find(value=>value.operation===options.operationId),target=actors[intent.targetRole],replacement=makeConnection('target-'+intent.operation,index);
   const receipt={connection:{id:replacement.id},activationState:'ready',binding:{deviceId:target.deviceId,endpointId:invalidReceipt&&index===0?'wrong':target.endpointId},retired:[id],retained:[{id:command.retain}]};
   const entry={connection:replacement,source:id,index,intent,completed:false,preparedIO:deferred()};
   const completion=(async()=>{await entry.preparedIO.promise;if(intent.ordinal===0)await commit.promise;const binding=sources.sources[index].attachments.find(value=>value.connectionId===id);binding.capture.closed=true;entry.completed=true;return {value:receipt};})();
   entry.completion=completion;byOperation.set(intent.operation,entry);
   return {reference:{operation:intent.operation},referenceJson:()=>({operation:intent.operation}),wait:()=>completion};},
  async refresh(){if(id.startsWith('initial-')&&[0,4].includes(index)){await oldRead.promise;return {state:oldRefusal&&index===0?'active':'retired'};}return {state:'retired'};}});
 for(const role of [...plan.routes.map(route=>route.role),'target']){
  const session={refresh:async()=>{if(snapshotRefusal&&role==='member-1'&&now<40000)throw snapshotError;return {topologyRevision:1};},connections:{prepareReplacement:async({operation})=>{
   prepared.resolve();return byOperation.get(operation.operation);
  }}};
  actors[role]={deviceId:role+'-device',endpointId:role+'-endpoint',client:{conversations:{byId:()=>({sessions:{byId:()=>session}})}}};
 }
 const state={conversation:{id:'conversation'},connections:plan.routes.map(route=>makeConnection(route.connection,route.index))};
 for(const route of plan.routes)sources.sources[route.index].attachments.push({connectionId:route.connection,capture:{closed:false}});
 class Renderer{audio(){return true;}video(){return true;}}
 const observer=browserMatrixObservation(Renderer,plan),report={};
 const work=browserMatrixProgramme({state,actors,plan,sources,journal:async value=>journals.push(value),
  keepalive:{check(){if(keepaliveFailure&&now>=20000)throw reachabilityError;}},report,observer,selectedTransport:'webtransport'});
 const advance=async at=>{now=at;for(const timer of timers.splice(0).filter(timer=>timer.at<=at))timer.resolve();await flush();};
 const audio=(connectionId,timestamp,sequence=1)=>{
  const entry=byOperation.get(connectionId.slice('target-'.length)),index=entry.index;
  const handle=32*(entry.intent.ordinal+1)+index*4+1,rtpTimestamp=Math.round(now*48);
  const ntp=(BigInt(timeOrigin)*1000n*(1n<<32n))/1000000n+(2208988800n<<32n);
  const native={profile:'connect/native-decoded-sequence/1',identity:nativeIdentity(handle,entry.intent.ordinal+2),
   datagramSequences:[sequence],ssrc:7,rtpTimestamp,
   ...(sequence>0?{sourceClock:{ntp:ntp.toString(),rtpTimestamp:0}}:{})};
  return renderers.get(connectionId).audio({connectionId,streamId:entry.intent.audio},
   {sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp,
    copyTo:out=>out.set(Float32Array.from({length:960},(_,i)=>.7*Math.sin(2*Math.PI*(index===0?1100:660)*i/48000)))},native);
 };
 try{
  await flush();await advance(15000);await prepared.promise;await flush();
  if(!snapshotRefusal)audio('target-move-0-0',0,0);
  assert.equal(observer.report().routes[0].audio.decoded_seconds,0,'Prepared private output stays excluded');
  commit.resolve();await flush();now=15020;
  if(!snapshotRefusal){
   audio('target-move-0-0',20000);
   const observed=observer.report().routes[0];
   assert.equal(observed.audio.decoded_seconds,invalidReceipt?0:.02,'Only a validated postcommit target is observed while old refresh is blocked');
   if(!invalidReceipt){assert.equal(observed.sync.audio_envelope.raw_threshold_transitions[0].at,15.02,'Keep actual receive callback time');
    audio('target-move-0-0',20000);assert.equal(observer.report().routes[0].audio.duplicate_frames,1,'Original duplicate detection remains strict');}
  }else assert.equal(byOperation.has('move-0-0'),false,'A refused fresh snapshot must not submit a replacement');
  oldRead.resolve();await flush();await advance(40000);await advance(62000);await work;
  if(invalidReceipt||oldRefusal||snapshotRefusal)assert.ok(report.failures.some(value=>value.operation_id==='move-0-0'),'Verification failure remains a failed original move');
  else assert.equal(report.moves.length,4);
  if(keepaliveFailure){
   assert.equal(report.passed,false,'Original reachability failure cannot pass the programme');
   assert.equal(report.keepalive_failure.kind,reachabilityError.kind);
   assert.equal(report.keepalive_failure.certainty,'read-only');
   assert.equal(report.finished_ms,62000,'An unrelated owner failure cannot shorten the original clock');
   assert.ok(report.decoded_media,'Collect the independent observer even on a failed programme');
   assert.equal(journals.at(-1).kind,'programme-terminal');
  }
  if(snapshotRefusal){
   assert.deepEqual(report.blocked_moves.map(value=>value.operation_id),['move-0-1']);
   assert.equal(byOperation.has('move-0-1'),false,'No return mutation follows a failed outbound root');
   assert.ok(byOperation.has('move-4-1'),'The independent original route still executes its return');
   assert.equal(report.failed_move_attempts,1);
   assert.equal(report.failures[0].kind,snapshotError.kind,'Preserve a refusal before mutation submission');
  }
 }finally{commit.resolve();oldRead.resolve();await flush();await advance(40000);await advance(62000);await work.catch(()=>{});await observer.close();globalThis.performance=originalClock;globalThis.setTimeout=originalTimer;globalThis.document=originalDocument;}
}
test('validated target output is observed before a slow old-owner verification read',()=>fixture());
test('invalid ready receipt never activates prepared output',()=>fixture({invalidReceipt:true}));
test('old-owner verification refusal still fails after actual postcommit output is observed',()=>fixture({oldRefusal:true}));
test('reachability failure keeps both independent move slots and the original measurement terminal',()=>fixture({keepaliveFailure:true}));
test('refused outbound snapshot blocks only its own return despite a reachability failure',()=>fixture({keepaliveFailure:true,snapshotRefusal:true}));
