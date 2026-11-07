// Framework/ownership fixture only. Does not use a service or qualify media.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {nativeMatrixLifecycle} from '../../lib/native_matrix_lifecycle.mjs';
import {nativeProgrammeBridge} from '../../lib/native_programme_bridge.mjs';
import {NATIVE_PROGRAMME_PROFILE} from '../../lib/native_programme_profile.mjs';
const bytes=await readFile(process.argv[2]),input=JSON.parse(bytes),bridge=nativeProgrammeBridge();
const lifecycle=nativeMatrixLifecycle(input.matrix_resource_directory,{prefix:input.matrix_resource_prefix??''});
assert.equal((await lifecycle.baseline()).passed,true);
// Match nativeAv's exclusive ownership of each cycle's previously absent leaf.
let exclusiveMediaOutput;
if(input.media_output_directory!==undefined){
 await mkdir(input.media_output_directory,{mode:0o700});
 exclusiveMediaOutput={directory:input.media_output_directory,cycle_index:input.matrix_cycle_index,
  workload_id:input.matrix_workload_id,programme_owner:bridge.owner};
}
const child=spawn(process.execPath,['-e',"process.stdin.resume();process.stdin.on('end',()=>{process.stdout.write('original child closed\\n');});"],{stdio:['pipe','pipe','pipe']});
let count=0;child.stdout.on('data',value=>{count+=value.length;assert.ok(count<4096);});child.stderr.resume();
await lifecycle.terminal();child.stdin.end();const [code]=await once(child,'close');assert.equal(code,0);
const observed=await lifecycle.retired({observedHelpers:1,helpersJoined:true,closeFailed:false});
const report={format:'connect/transport-conversation-control/3',programme_profile:NATIVE_PROGRAMME_PROFILE,
 programme_owner:bridge.owner,input_sha256:createHash('sha256').update(bytes).digest('hex'),passed:(Array.isArray(input.reported_quality_passed)?input.reported_quality_passed[input.matrix_cycle_index-1]:input.reported_quality_passed)!==false,
 programme_cleanup:observed.retirement,all_observed_helpers_joined:true,checks:['framework-original-child-stdio-joined'],
 original_child_pid:child.pid,original_node_pid:process.pid,exclusive_media_output:exclusiveMediaOutput,
 quality_failure:(Array.isArray(input.reported_quality_passed)?input.reported_quality_passed[input.matrix_cycle_index-1]:input.reported_quality_passed)===false?'retained-framework-quality-failure':null};
await writeFile(process.argv[3],JSON.stringify(report)+'\n',{flag:'wx',mode:0o600});
