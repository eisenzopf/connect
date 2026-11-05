// One isolated public Node SDK credential boundary, driven by the native cell.
import {fileSha256} from "./lib/transport_file_hash.mjs";
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile,writeFile,stat,realpath} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
import {NodeClient} from '../sdk/typescript/dist/index.js';
import {nativeAuthorityCase} from './lib/native_matrix_authority.mjs';
import {childPids,executableHash,processExists,nativeHost} from './lib/transport_host.mjs';

const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const inputPath=resolve(process.argv[2]),output=resolve(process.argv[3]);
assert.equal(process.argv.length,4);
const input=JSON.parse(await readFile(inputPath,'utf8')),directory=dirname(inputPath);
assert.equal(input.format,'connect/native-matrix-authority-input/1');
assert.equal((await stat(inputPath)).mode&0o077,0);
assert.equal(dirname(output),directory);
assert.equal(await realpath(inputPath),inputPath);
assert.equal(await fileSha256(input.executable),input.binary_sha256);
const configHash=hash(await readFile(input.config)),helpers=new Set();
const observeHelpers=async()=>{
    for(const pid of childPids(process.pid)){
        try{if(await executableHash(pid)===input.binary_sha256)helpers.add(pid);}
        catch(error){if(!['ENOENT','ESRCH'].includes(error.code))throw error;}
    }
    assert.ok(helpers.size>0);
};
const result=await nativeAuthorityCase({kind:input.kind,operationId:input.operation_id,expiresAt:input.expires_at,
    open:()=>new NodeClient({config:input.config,executable:input.executable}),
    saveReference:reference=>writeFile(join(directory,'original-operation.json'),JSON.stringify(reference)+'\n',{flag:'wx',mode:0o600}),
    boundary:async()=>{
        await writeFile(join(directory,'revoke-request.json'),JSON.stringify({kind:'revoke',operation_id:input.revoke_operation_id})+'\n',{flag:'wx',mode:0o600});
        const path=join(directory,'revoke-response.json'),cutoff=Date.now()+10000;
        while(Date.now()<cutoff){
            try{assert.ok((await stat(path)).size<=1024);return JSON.parse(await readFile(path,'utf8'));}
            catch(error){if(error.code!=='ENOENT')throw error;}await sleep(20);
        }
        throw Error('Original revocation response deadline');
    },observeHelpers,
    helpersJoined:async()=>{
        for(const pid of helpers)if(await processExists(pid))return false;
        for(const pid of childPids(process.pid)){
            try{if(await executableHash(pid)===input.binary_sha256)return false;}
            catch(error){if(!['ENOENT','ESRCH'].includes(error.code))throw error;}
        }
        return helpers.size>0;
    }});
result.observed_helpers=helpers.size;result.host=nativeHost();
result.source_revision=input.source_revision;result.binary_sha256=input.binary_sha256;
result.signing_algorithm=input.signing_algorithm;result.network_profile=input.network_profile;
result.original_config_unchanged=hash(await readFile(input.config))===configHash;
result.executable_unchanged=await fileSha256(input.executable)===input.binary_sha256;
try{result.original_reference_sha256=hash(await readFile(join(directory,'original-operation.json')));}catch{}
result.passed&&=result.original_config_unchanged&&result.executable_unchanged;
await writeFile(output,JSON.stringify(result,null,2)+'\n',{flag:'wx',mode:0o600});
if(!result.passed)process.exitCode=1;
