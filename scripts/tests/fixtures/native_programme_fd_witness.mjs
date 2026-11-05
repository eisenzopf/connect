// Actual original Node process and child-stdio-loop witness, never a cell pass.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {readdirSync,readlinkSync,writeSync,writeFileSync,existsSync} from 'node:fs';
import {join} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
import {Worker,isMainThread,parentPort,threadId,workerData} from 'node:worker_threads';
import {ownWorkerStdio} from '../../lib/native_programme_profile.mjs';
async function originalChild(){
 const child=spawn(process.execPath,['-e',"process.stdin.resume();process.stdin.once('end',()=>{process.stdout.write('joined\\n');});"],{stdio:['pipe','pipe','pipe']});
 let bytes=0;child.stdout.on('data',v=>{bytes+=v.length;assert.ok(bytes<=128);});child.stderr.resume();child.stdin.end();
 const [code]=await once(child,'close');assert.equal(code,0);return {child_pid:child.pid,child_joined:true,stdio_bytes:bytes};
}
async function boundary(directory,phase,pid){
 const path=join(directory,phase+'.json');writeFileSync(path,JSON.stringify({phase,pid})+'\n',{flag:'wx',mode:0o600});
 const until=performance.now()+5000;while(!existsSync(join(directory,phase+'-observed.json'))){assert.ok(performance.now()<until,'Original external descriptor observer deadline');await sleep(5);}
}
function descriptors(){
 if(process.platform==='linux'){
  const rows=[];for(const fd of readdirSync('/proc/self/fd'))try{rows.push([Number(fd),readlinkSync('/proc/self/fd/'+fd)]);}catch(error){if(error.code!=='ENOENT')throw error;}
  return {kind:'proc-fd',rows:rows.sort((a,b)=>a[0]-b[0])};
 }
 // Read-only descriptor inspection belongs outside the observed Node loop in
 // the actual Mac proof runner. It must supply bounded libproc snapshots.
 return {kind:'external-macos-libproc-required',rows:null};
}
if(!isMainThread){
 assert.equal(workerData.profile,'native-worker-fd-witness');parentPort.postMessage({kind:'ready',logical_worker_thread_id:threadId,node_pid:process.pid});
 await once(parentPort,'message');const result=await originalChild();parentPort.postMessage({kind:'child',...result});parentPort.close();
}else{
 const mode=process.argv[2],directory=process.argv[3];assert.ok(['before','after'].includes(mode));const pid=process.pid;let baseline,final,child,logicalId=null;
 if(mode==='before'){
  baseline=descriptors();writeSync(1,JSON.stringify({phase:'baseline',mode,pid})+'\n');await boundary(directory,'baseline',pid);child=await originalChild();await new Promise(setImmediate);final=descriptors();
 }else{
  const worker=new Worker(new URL(import.meta.url),{stdout:true,stderr:true,workerData:{profile:'native-worker-fd-witness'}});
  const stdout=ownWorkerStdio(worker.stdout,1),stderr=ownWorkerStdio(worker.stderr,2);
  const [ready]=await once(worker,'message');assert.equal(ready.kind,'ready');assert.equal(ready.node_pid,pid);logicalId=ready.logical_worker_thread_id;
  worker.on('message',v=>{if(v.kind==='child')child=v;});baseline=descriptors();writeSync(1,JSON.stringify({phase:'baseline',mode,pid,logical_worker_thread_id:logicalId})+'\n');
  await boundary(directory,'baseline',pid);worker.postMessage({start:true});const [code]=await once(worker,'exit');assert.equal(code,0);await Promise.all([stdout.joined,stderr.joined]);stdout.release();stderr.release();
  assert.ok(stdout.receipt().ended&&!stdout.receipt().failure&&stderr.receipt().ended&&!stderr.receipt().failure);
  await new Promise(setImmediate);final=descriptors();
 }
 assert.equal(process.pid,pid);assert.equal(child.child_joined,true);await boundary(directory,'final',pid);
 writeSync(1,JSON.stringify({phase:'final',mode,pid,node:process.version,architecture:process.arch,logical_worker_thread_id:logicalId,
  child,baseline,final,scope:'Original process descriptor witness only; no RSS, media, idle or matrix qualification.'})+'\n');
}
