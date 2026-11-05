// Framework/ownership fixture only. Does not use a service or qualify media.
import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {nativeMatrixLifecycle} from '../../lib/native_matrix_lifecycle.mjs';
import {nativeProgrammeBridge} from '../../lib/native_programme_bridge.mjs';
import {NATIVE_PROGRAMME_PROFILE} from '../../lib/native_programme_profile.mjs';
const bytes=await readFile(process.argv[2]),input=JSON.parse(bytes),bridge=nativeProgrammeBridge();
const lifecycle=nativeMatrixLifecycle(input.matrix_resource_directory);
assert.equal((await lifecycle.baseline()).passed,true);
const child=spawn(process.execPath,['-e',"process.stdin.resume();process.stdin.on('end',()=>{process.stdout.write('original child closed\\n');});"],{stdio:['pipe','pipe','pipe']});
let count=0;child.stdout.on('data',value=>{count+=value.length;assert.ok(count<4096);});child.stderr.resume();
await lifecycle.terminal();child.stdin.end();const [code]=await once(child,'close');assert.equal(code,0);
const observed=await lifecycle.retired({observedHelpers:1,helpersJoined:true,closeFailed:false});
const report={format:'connect/transport-conversation-control/3',programme_profile:NATIVE_PROGRAMME_PROFILE,
 programme_owner:bridge.owner,input_sha256:createHash('sha256').update(bytes).digest('hex'),passed:input.reported_quality_passed!==false,
 programme_cleanup:observed.retirement,all_observed_helpers_joined:true,checks:['framework-original-child-stdio-joined'],
 original_child_pid:child.pid,original_node_pid:process.pid,
 quality_failure:input.reported_quality_passed===false?'retained-framework-quality-failure':null};
await writeFile(process.argv[3],JSON.stringify(report)+'\n',{flag:'wx',mode:0o600});
