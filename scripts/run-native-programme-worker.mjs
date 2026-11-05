import assert from 'node:assert/strict';
import {writeSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {runNativeProgrammeWorker} from './lib/native_programme_controller.mjs';
assert.equal(process.argv.length,5);assert.equal(process.argv[4],'--media-workload');
try{
 const result=await runNativeProgrammeWorker({inputPath:process.argv[2],outputPath:process.argv[3],
  programmeUrl:new URL('./check-transport-conversation.mjs',import.meta.url),privateRoot:fileURLToPath(new URL('../.local',import.meta.url))});
 writeSync(1,JSON.stringify(result)+'\n');if(!result.passed)process.exitCode=1;
}catch(error){writeSync(2,JSON.stringify({passed:false,phase:'native-programme-worker-controller',failure:error?.name??'Error'})+'\n');process.exitCode=1;}
