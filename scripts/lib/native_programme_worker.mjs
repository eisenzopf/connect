import assert from 'node:assert/strict';
import {isMainThread,parentPort,threadId,workerData} from 'node:worker_threads';
import {installNativeProgrammeBridge} from './native_programme_bridge.mjs';
import {NATIVE_PROGRAMME_PROFILE,now,sealedFile,originalWorkerOwner} from './native_programme_profile.mjs';
assert.equal(isMainThread,false);assert.equal(workerData.profile,NATIVE_PROGRAMME_PROFILE);
const owner=originalWorkerOwner({profile:NATIVE_PROGRAMME_PROFILE,original_node_pid:process.pid,
 logical_worker_thread_id:threadId,created_monotonic_seconds:workerData.createdMonotonicSeconds,started_monotonic_seconds:now(),original_global_deadline_monotonic_seconds:workerData.cutoffMonotonicSeconds});
let terminal,cleanup;
const send=value=>{assert.ok(Buffer.byteLength(JSON.stringify(value))<=4096);parentPort.postMessage(value);};
try{
 assert.equal(process.pid,workerData.originalNodePid);
 assert.ok(workerData.cutoffMonotonicSeconds>now());
 send({kind:'started',owner});
 installNativeProgrammeBridge({owner,cutoff:workerData.cutoffMonotonicSeconds,
  terminal(when){assert.equal(terminal,undefined);terminal=when;send({kind:'terminal',monotonic_seconds:when});},
  retired(value){assert.equal(cleanup,undefined);cleanup=value;send({kind:'cleanup',retirement:value});}});
 process.argv=[process.execPath,workerData.programmePath,workerData.inputPath,workerData.workReportPath,'--media-workload'];
 await import(workerData.programmeUrl);
 assert.ok(terminal!==undefined&&cleanup!==undefined,'Original terminal and helper cleanup are required before sealing');
 const sealed=sealedFile(workerData.workReportPath,Buffer.allocUnsafe(65536),{sync:true});
 send({kind:'sealed',path:workerData.workReportPath,...sealed,exit_code:process.exitCode??0,
  original_terminal_monotonic_seconds:terminal});
}catch(error){
 process.exitCode=1;
 send({kind:'failed',failure:['AssertionError','Error','TypeError','RangeError','SyntaxError'].includes(error?.name)?error.name:'Error'});
}finally{
 parentPort.close();
 // No process.exit or Worker.terminate: the actual original event loop, child
 // handles and subordinate inspection workers must reach natural quiescence.
}
