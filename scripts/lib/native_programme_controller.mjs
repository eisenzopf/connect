import assert from 'node:assert/strict';
import {Worker} from 'node:worker_threads';
import {readFileSync,writeFileSync,realpathSync,lstatSync,fstatSync,openSync,closeSync,fsyncSync,mkdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as sleep} from 'node:timers/promises';
import {nativeMatrixLifecycle} from './native_matrix_lifecycle.mjs';
import {NATIVE_PROGRAMME_PROFILE,now,sealedFile,ownWorkerStdio,originalWorkerOwner,workerJoinVerified} from './native_programme_profile.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const safeName=error=>['AssertionError','Error','TypeError','RangeError','SyntaxError'].includes(error?.name)?error.name:'Error';

/** Original parent owns only bounded control/file summaries until idle finishes.
 * programmeUrl is a private framework test injection; the production launcher
 * always supplies the repository's unchanged full native programme entrypoint. */
async function runNativeProgrammeCycle({inputPath,outputPath,programmeUrl,privateRoot,logFds=[1,2],originalDeadline=Number(process.env.CONNECT_NATIVE_PROGRAMME_DEADLINE_MONOTONIC_SECONDS)}){
 const created=now(),buffer=Buffer.allocUnsafe(65536);
 inputPath=realpathSync(inputPath);outputPath=resolve(outputPath);privateRoot=realpathSync(privateRoot);
 assert.ok(inputPath.startsWith(privateRoot+'/')&&realpathSync(dirname(outputPath)).startsWith(privateRoot+'/'));
 const inputSeal=sealedFile(inputPath,buffer);
 const config=(()=>{const input=JSON.parse(readFileSync(inputPath,'utf8'));
  assert.equal(input.matrix_programme_profile,NATIVE_PROGRAMME_PROFILE);
  assert.equal(input.format,'connect/transport-conversation-input/1');
  assert.ok(Number.isFinite(input.matrix_programme_budget_seconds)&&input.matrix_programme_budget_seconds>=20&&input.matrix_programme_budget_seconds<=300);
  const directory=realpathSync(input.matrix_resource_directory);assert.ok(directory.startsWith(privateRoot+'/'));
  return {directory,budget:input.matrix_programme_budget_seconds,prefix:input.matrix_resource_prefix??''};})();
 for(const fd of logFds)assert.ok(fstatSync(fd).isFile(),'Inherited ordinary log descriptors required');
 assert.ok(Number.isFinite(originalDeadline)&&originalDeadline>created&&originalDeadline<=created+config.budget,'Existing original supervisor deadline is required');
 const cutoff=originalDeadline,workReportPath=outputPath+'.worker.json';
 const programmePath=fileURLToPath(programmeUrl);
 let worker=new Worker(new URL('./native_programme_worker.mjs',import.meta.url),{
  stdout:true,stderr:true,stdin:false,workerData:{profile:NATIVE_PROGRAMME_PROFILE,originalNodePid:process.pid,
   createdMonotonicSeconds:created,cutoffMonotonicSeconds:cutoff,programmeUrl:String(programmeUrl),programmePath,inputPath,workReportPath}});
 const logicalId=worker.threadId,stdout=ownWorkerStdio(worker.stdout,logFds[0]),stderr=ownWorkerStdio(worker.stderr,logFds[1]);
 let owner,terminal,cleanup,sealed,failure=null,forced=false,exitCode=null,exited;
 const refuse=name=>{failure??=name;};
 const message=value=>{try{
  assert.ok(value&&Buffer.byteLength(JSON.stringify(value))<=4096);
  if(value.kind==='started'){
   assert.equal(owner,undefined);owner=originalWorkerOwner(value.owner);
   assert.equal(owner.original_node_pid,process.pid);assert.equal(owner.logical_worker_thread_id,logicalId);assert.equal(owner.created_monotonic_seconds,created);assert.equal(owner.original_global_deadline_monotonic_seconds,cutoff);
  }else if(value.kind==='terminal'){
   assert.ok(owner&&terminal===undefined&&Number.isFinite(value.monotonic_seconds));
   assert.ok(value.monotonic_seconds>=owner.started_monotonic_seconds&&value.monotonic_seconds<=now());terminal=value.monotonic_seconds;
  }else if(value.kind==='cleanup'){
   assert.ok(owner&&terminal!==undefined&&cleanup===undefined&&value.retirement&&typeof value.retirement.passed==='boolean');cleanup=value.retirement;
  }else if(value.kind==='sealed'){
   assert.ok(owner&&terminal!==undefined&&cleanup&&sealed===undefined);assert.equal(value.path,workReportPath);assert.equal(value.original_terminal_monotonic_seconds,terminal);
   assert.match(value.sha256,/^[a-f0-9]{64}$/);assert.ok(Number.isSafeInteger(value.bytes)&&value.bytes>0);sealed=value;
  }else if(value.kind==='failed')refuse('worker-'+String(value.failure).slice(0,32));
  else throw Error('Unknown original worker control');
 }catch{refuse('worker-control-mismatch');}};
 const error=()=>refuse('original-worker-error');worker.on('message',message);worker.on('error',error);
 const joined=new Promise(resolve=>worker.once('exit',code=>{exitCode=code;exited=now();resolve();}));
 let deadlineReached;
 const deadlineSignal=new Promise(resolve=>{deadlineReached=resolve;});
 const deadline=setTimeout(()=>{
  forced=true;refuse('original-global-deadline');deadlineReached();
  // Cleanup cannot turn this failed original budget into qualification.
  if(exited===undefined)void worker.terminate().catch(()=>refuse('original-worker-terminate-failed'));
 },Math.max(1,(cutoff-now())*1000));
 await joined;
 await Promise.race([Promise.all([stdout.joined,stderr.joined]),deadlineSignal]);clearTimeout(deadline);
 const joinedAt=now();
 worker.off('message',message);worker.off('error',error);stdout.release();stderr.release();
 if(!stdout.receipt().ended)worker.stdout.destroy();if(!stderr.receipt().ended)worker.stderr.destroy();worker=undefined;
 const stdio={stdout:stdout.receipt(),stderr:stderr.receipt()},stdioJoined=Object.values(stdio).every(v=>v.ended&&!v.failure);
 if(!stdioJoined)refuse('owned-worker-stdio-failed');if(exitCode!==0)refuse('original-worker-exit');
 let digestVerified=false;
 try{
  assert.ok(sealed);assert.equal(sealed.exit_code,exitCode);const actual=sealedFile(workReportPath,buffer);
  assert.equal(actual.sha256,sealed.sha256);assert.equal(actual.bytes,sealed.bytes);assert.deepEqual(actual.identity,sealed.identity);
  assert.equal(sealedFile(inputPath,buffer).sha256,inputSeal.sha256);digestVerified=true;
 }catch{refuse('sealed-original-report-mismatch');}
 const receipt={owner:owner??null,original_terminal_monotonic_seconds:terminal??null,
  original_exit_observed:exited!==undefined,exit_code:exitCode,forced,exit_monotonic_seconds:exited??null,
  joined_monotonic_seconds:joinedAt,stdio_joined:stdioJoined,report_digest_verified:digestVerified,
  report_sha256:sealed?.sha256??null,report_bytes:sealed?.bytes??null,failure};
 let observation={retirement:{passed:false,programme_worker:receipt},idle_observer:{passed:false,failure:'original-worker-boundary-unavailable'}};
 if(owner&&terminal!==undefined){
  const lifecycle=nativeMatrixLifecycle(config.directory,{owner,originalTerminal:terminal,workerReceipt:receipt,cutoff,prefix:config.prefix});
  observation=await lifecycle.retired({observedHelpers:cleanup?.observed_helpers??0,
   helpersJoined:cleanup?.original_helpers_joined===true,closeFailed:cleanup?.original_clients_closed!==true,
   sessionRetirement:cleanup?.original_session_retirement,workloadObservation:cleanup?.workload_observation});
  // Even a failed/early observer reply cannot move the original T+10 boundary.
  if(now()<terminal+10&&now()<cutoff)await sleep(Math.max(0,(Math.min(terminal+10,cutoff)-now())*1000));
 }
 const loadedAt=now();let report;
 try{
  assert.ok(digestVerified&&owner&&terminal!==undefined&&loadedAt>=terminal+10&&loadedAt<=cutoff);
  const bytes=readFileSync(workReportPath);assert.equal(bytes.length,sealed.bytes);assert.equal(hash(bytes),sealed.sha256);
  report=JSON.parse(bytes);assert.equal(report.format,'connect/transport-conversation-control/3');
  assert.equal(report.programme_profile,NATIVE_PROGRAMME_PROFILE);assert.deepEqual(report.programme_owner,owner);
  assert.deepEqual(report.programme_cleanup,cleanup);assert.equal(report.input_sha256,inputSeal.sha256);
  assert.equal(report.all_observed_helpers_joined,cleanup.original_helpers_joined);
 }catch(error){refuse('original-work-report-'+safeName(error));report={format:'connect/transport-conversation-control/3',programme_profile:NATIVE_PROGRAMME_PROFILE,passed:false,
  failure:{phase:'original-worker-report',name:safeName(error)},input_sha256:inputSeal.sha256,checks:[],cell_qualified:false};}
 receipt.failure=failure;observation.retirement.passed=Boolean(observation.retirement.passed&&workerJoinVerified(receipt,owner,terminal));
 report.programme_worker=receipt;report.programme_worker_stdio=stdio;report.programme_report_loaded_monotonic_seconds=loadedAt;
 report.retirement=observation.retirement;report.native_idle_observation=observation.idle_observer;
 report.passed=Boolean(report.passed&&observation.retirement.passed&&observation.idle_observer.passed&&!failure);
 writeFileSync(outputPath,JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});
 const fd=openSync(outputPath,'r');try{fsyncSync(fd);}finally{closeSync(fd);}
 const summary={passed:report.passed,checks:report.checks.length,cell_qualified:false,profile:NATIVE_PROGRAMME_PROFILE,
  original_node_pid:process.pid,logical_worker_thread_id:logicalId,natural_worker_joined:!forced&&exitCode===0,report_sha256:hash(readFileSync(outputPath))};
 report=undefined;return summary;
}

/** Three actual programmes share this original Node process and server. Each
 * programme gets fresh persisted mutation IDs and original Worker/SDK owners;
 * failure remains sticky while subsequent independently owned cycles run. */
export async function runNativeProgrammeWorker(options){
 const input=JSON.parse(readFileSync(options.inputPath,'utf8'));
 if((input.matrix_resource_cycles??1)===1)return runNativeProgrammeCycle(options);
 assert.equal(input.matrix_resource_cycles,3);
 assert.ok(Number.isFinite(input.matrix_programme_budget_seconds)&&input.matrix_programme_budget_seconds>=20&&input.matrix_programme_budget_seconds<=300);
 const deadline=options.originalDeadline??Number(process.env.CONNECT_NATIVE_PROGRAMME_DEADLINE_MONOTONIC_SECONDS);
 assert.ok(Number.isFinite(deadline)&&deadline>now()&&deadline<=now()+3*input.matrix_programme_budget_seconds);
 const originalSeal=sealedFile(options.inputPath,Buffer.allocUnsafe(65536));
 const cycles=[];let passed=true;
 for(let index=1;index<=3;index++){
  // A failed quality verdict does not restart or repeat the preceding cycle.
  // Missing original ownership/deadline aborts safely and leaves an incomplete
  // plateau proof rather than manufacturing a third observation.
  if(now()>=deadline){passed=false;break;}
  const directory=join(dirname(options.inputPath),'native-cycle-'+index);mkdirSync(directory,{mode:0o700});
  const inputPath=join(directory,'input.json'),outputPath=options.outputPath+'.cycle-'+index+'.json';
  const workloadId='native-cycle-'+index+'-'+process.pid;
  const mediaOutput=join(input.media_output_directory,'cycle-'+index);mkdirSync(dirname(mediaOutput),{recursive:true,mode:0o700});
  writeFileSync(inputPath,JSON.stringify({...input,matrix_resource_cycles:1,matrix_resource_prefix:'cycle-'+index+'-',
   matrix_workload_id:workloadId,matrix_cycle_index:index,media_output_directory:mediaOutput})+'\n',{flag:'wx',mode:0o600});
  const fd=openSync(inputPath,'r');try{fsyncSync(fd);}finally{closeSync(fd);}
  let result;
  try{result=await runNativeProgrammeCycle({...options,inputPath,outputPath,originalDeadline:Math.min(deadline,now()+input.matrix_programme_budget_seconds)});}
  catch(error){cycles.push({index,workload_id:workloadId,passed:false,failure:safeName(error)});passed=false;break;}
  let bytes=readFileSync(outputPath),report=JSON.parse(bytes);passed&&=result.passed;
  cycles.push({index,workload_id:workloadId,input_path:inputPath,output_path:outputPath,
   media_output_directory:mediaOutput,report_sha256:hash(bytes),passed:result.passed,
   original_node_pid:process.pid,programme_worker:report.programme_worker});
  report=undefined;bytes=undefined;
 }
 assert.equal(sealedFile(options.inputPath,Buffer.allocUnsafe(65536)).sha256,originalSeal.sha256);
 const first=cycles[0]?.output_path?JSON.parse(readFileSync(cycles[0].output_path,'utf8')):undefined;
 const report={...(first??{format:'connect/transport-conversation-control/3',checks:[]}),
  passed:Boolean(passed&&cycles.length===3),matrix_resource_cycles:3,resource_cycles:cycles,
  input_sha256:originalSeal.sha256,original_node_pid:process.pid};
 writeFileSync(options.outputPath,JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});
 const fd=openSync(options.outputPath,'r');try{fsyncSync(fd);}finally{closeSync(fd);}
 return {passed:report.passed,checks:report.checks.length,cell_qualified:false,profile:NATIVE_PROGRAMME_PROFILE,
  original_node_pid:process.pid,resource_cycles:cycles.length,report_sha256:hash(readFileSync(options.outputPath))};
}
