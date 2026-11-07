import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,existsSync,rmSync,openSync,closeSync,realpathSync,lstatSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
import {NATIVE_PROGRAMME_PROFILE,now,sealedFile,workerJoinVerified} from '../lib/native_programme_profile.mjs';
import {runNativeProgrammeWorker} from '../lib/native_programme_controller.mjs';
const owner={profile:NATIVE_PROGRAMME_PROFILE,original_node_pid:42,logical_worker_thread_id:1,created_monotonic_seconds:1,started_monotonic_seconds:2,original_global_deadline_monotonic_seconds:300};
const receipt=()=>({owner,original_terminal_monotonic_seconds:20,original_exit_observed:true,exit_code:0,forced:false,
 exit_monotonic_seconds:20.2,joined_monotonic_seconds:20.3,stdio_joined:true,report_digest_verified:true,
 report_sha256:'a'.repeat(64),report_bytes:1000,failure:null});
test('only the exact original natural Worker join can qualify its unchanged terminal',()=>{
 assert.equal(workerJoinVerified(receipt(),owner,20),true);
 for(const [field,value]of [['original_exit_observed',false],['exit_code',1],['forced',true],['stdio_joined',false],
  ['report_digest_verified',false],['report_sha256','bad'],['failure','error'],['joined_monotonic_seconds',30.001],
  ['original_terminal_monotonic_seconds',21]])assert.equal(workerJoinVerified({...receipt(),[field]:value},owner,20),false,field);
 assert.equal(workerJoinVerified({...receipt(),owner:{...owner,logical_worker_thread_id:2}},owner,20),false);
 assert.equal(workerJoinVerified({...receipt(),exit_monotonic_seconds:19.9},owner,20),false);
});
test('sealed work report digest changes for changed bytes and rejects nonprivate artifacts',()=>{
 const root=mkdtempSync(join(tmpdir(),'native-worker-seal-')),path=join(root,'report.json'),buffer=Buffer.alloc(65536);
 try{writeFileSync(path,'{"passed":false}\n',{mode:0o600});const first=sealedFile(path,buffer);
  writeFileSync(path,'{"passed":true}\n');const second=sealedFile(path,buffer);assert.notEqual(first.sha256,second.sha256);
  const publicPath=join(root,'public.json');writeFileSync(publicPath,'{}',{mode:0o644});assert.throws(()=>sealedFile(publicPath,buffer));
 }finally{rmSync(root,{recursive:true,force:true});}
});
for (const qualityPassed of [true,false]) test(`one original Node parent retains quality=${qualityPassed} independently of natural Worker/child joins and original T+10`,{timeout:25000},async()=>{
 const root=realpathSync(mkdtempSync(join(tmpdir(),'native-worker-original-'))),work=join(root,'work');mkdirSync(work);
 const resources=join(work,'resources'),input=join(work,'input.json'),output=join(work,'report.json');mkdirSync(resources);
 const fd=openSync(join(root,'original.log'),'wx',0o600),pid=process.pid;
 writeFileSync(input,JSON.stringify({format:'connect/transport-conversation-input/1',matrix_programme_profile:NATIVE_PROGRAMME_PROFILE,
  matrix_programme_budget_seconds:20,matrix_resource_directory:resources,reported_quality_passed:qualityPassed}),{mode:0o600});
 let stopping=false,terminal,retirement,baselineOwner;
 const observer=(async()=>{while(!stopping){
  for(const phase of['baseline','terminal','retired']){
   const request=join(resources,`resources-${phase}.json`),response=join(resources,`resources-${phase}-response.json`);
   if(!existsSync(request)||existsSync(response))continue;const value=JSON.parse(readFileSync(request,'utf8'));
   if(phase==='baseline'){baselineOwner=value.programme_owner;assert.equal(baselineOwner.original_node_pid,pid);assert.ok(baselineOwner.started_monotonic_seconds<=value.monotonic_seconds);}
   if(phase==='terminal')terminal=value.monotonic_seconds;
   if(phase==='retired'){retirement=value.retirement;assert.ok(workerJoinVerified(retirement.programme_worker,baselineOwner,terminal));
    if(now()<terminal+10)continue;}
   writeFileSync(response,JSON.stringify({passed:true}),{flag:'wx',mode:0o600});
  }await sleep(5);
 }})();
 try{const result=await runNativeProgrammeWorker({inputPath:input,outputPath:output,privateRoot:root,
  programmeUrl:new URL('./fixtures/native_programme_framework_workload.mjs',import.meta.url),logFds:[fd,fd],originalDeadline:now()+19});
  assert.equal(result.passed,qualityPassed);assert.equal(process.pid,pid);assert.equal(result.original_node_pid,pid);
  const report=JSON.parse(readFileSync(output,'utf8'));assert.equal(report.original_node_pid,pid);
  assert.ok(report.programme_report_loaded_monotonic_seconds>=terminal+10);assert.ok(report.programme_worker.joined_monotonic_seconds-terminal<10);
  assert.equal(report.passed,qualityPassed);assert.equal(report.programme_worker.exit_code,0);
  assert.equal(report.programme_worker.failure,null);assert.equal(report.retirement.passed,true);
  assert.equal(report.quality_failure,qualityPassed?null:'retained-framework-quality-failure');
  assert.equal(report.retirement.original_helpers_joined,true);assert.equal(report.all_observed_helpers_joined,true);
  assert.equal(report.programme_worker.logical_worker_thread_id,undefined,'Logical ID remains in its explicit original owner receipt');
  assert.ok(report.programme_worker.owner.logical_worker_thread_id>0);assert.equal(report.programme_worker.forced,false);
  assert.deepEqual(report.retirement,retirement);
 }finally{stopping=true;await observer;closeSync(fd);rmSync(root,{recursive:true,force:true});}
});

test('existing private capture inspector retains exact byte custody when its caller is the programme Worker',{timeout:10000},async()=>{
 const {Worker}=await import('node:worker_threads');const root=mkdtempSync(join(tmpdir(),'native-worker-inspector-')),path=join(root,'capture.json');
 writeFileSync(path,JSON.stringify({received:[{original_root_completed_observed:false,payload:[1,2,3]}]}),{mode:0o600});
 let worker;
 try{
  worker=new Worker(`const {parentPort,workerData}=require('node:worker_threads');(async()=>{const m=await import(workerData.module);const r=await m.inspectReplacementCapture(workerData.path,4096,1);parentPort.postMessage(r);parentPort.close();})();`,
   {eval:true,workerData:{profile:'programme-inspector-caller-test',module:String(new URL('../lib/transport_capture_inspection.mjs',import.meta.url)),path}});
  let result,error;worker.on('message',v=>result=v);worker.on('error',v=>error=v);
  const code=await new Promise(resolve=>worker.once('exit',resolve));assert.equal(error,undefined);assert.equal(code,0);
  assert.equal(result.worker_joined,true);assert.deepEqual(result.observation.received,[{original_root_completed_observed:false}]);assert.match(result.sha256,/^[a-f0-9]{64}$/);
 }finally{if(worker&&worker.threadId!==-1)await worker.terminate();rmSync(root,{recursive:true,force:true});}
});


test('three independent original cycles keep one Node process and retain the first quality failure after later natural joins',{timeout:45000},async()=>{
 const root=realpathSync(mkdtempSync(join(tmpdir(),'native-worker-three-'))),work=join(root,'work');mkdirSync(work);
 const resources=join(work,'resources'),media=join(work,'media'),input=join(work,'input.json'),output=join(work,'report.json');mkdirSync(resources);
 const fd=openSync(join(root,'original.log'),'wx',0o600),pid=process.pid;
 writeFileSync(input,JSON.stringify({format:'connect/transport-conversation-input/1',matrix_programme_profile:NATIVE_PROGRAMME_PROFILE,
  matrix_programme_budget_seconds:20,matrix_resource_cycles:3,matrix_resource_directory:resources,
  media_output_directory:media,reported_quality_passed:[false,true,true]}),{mode:0o600});
 let stopping=false;const owners=new Map(),terminals=new Map();
 const observer=(async()=>{while(!stopping){
  for(const index of[1,2,3])for(const phase of['baseline','terminal','retired']){
   const prefix='cycle-'+index+'-',request=join(resources,`resources-${prefix}${phase}.json`),response=join(resources,`resources-${prefix}${phase}-response.json`);
   if(!existsSync(request)||existsSync(response))continue;const value=JSON.parse(readFileSync(request,'utf8'));
   assert.equal(value.programme_owner.original_node_pid,pid);
   if(phase==='baseline')owners.set(index,value.programme_owner);
   if(phase==='terminal')terminals.set(index,value.monotonic_seconds);
   if(phase==='retired'){
    assert.ok(workerJoinVerified(value.retirement.programme_worker,owners.get(index),terminals.get(index)));
    if(now()<terminals.get(index)+10)continue;
   }
   writeFileSync(response,JSON.stringify({passed:true}),{flag:'wx',mode:0o600});
  }await sleep(5);
 }})();
 try{
  const result=await runNativeProgrammeWorker({inputPath:input,outputPath:output,privateRoot:root,
   programmeUrl:new URL('./fixtures/native_programme_framework_workload.mjs',import.meta.url),logFds:[fd,fd],originalDeadline:now()+59});
  const report=JSON.parse(readFileSync(output,'utf8'));
  assert.equal(result.passed,false);assert.equal(report.passed,false);assert.equal(report.original_node_pid,pid);
  assert.equal(report.resource_cycles.length,3);assert.equal(new Set([...owners.values()].map(v=>v.logical_worker_thread_id)).size,3);
  assert.deepEqual(report.resource_cycles.map(v=>v.passed),[false,true,true]);
  assert.equal(new Set(report.resource_cycles.map(v=>v.workload_id)).size,3);
  for(const cycle of report.resource_cycles){
   const original=JSON.parse(readFileSync(cycle.output_path,'utf8'));assert.equal(original.original_node_pid,pid);
   assert.equal(original.programme_worker.forced,false);assert.equal(original.programme_worker.exit_code,0);
   assert.ok(original.programme_report_loaded_monotonic_seconds>=terminals.get(cycle.index)+10);
   assert.equal(original.retirement.original_helpers_joined,true);
   assert.equal(cycle.media_output_directory,join(media,'cycle-'+cycle.index));
   const leaf=lstatSync(cycle.media_output_directory);assert.equal(leaf.isDirectory(),true);assert.equal(leaf.mode&0o777,0o700);
   assert.deepEqual(original.exclusive_media_output,{directory:cycle.media_output_directory,cycle_index:cycle.index,
    workload_id:cycle.workload_id,programme_owner:owners.get(cycle.index)});
   assert.equal(original.quality_failure,cycle.index===1?'retained-framework-quality-failure':null);
  }
 }finally{stopping=true;await observer;closeSync(fd);rmSync(root,{recursive:true,force:true});}
});
