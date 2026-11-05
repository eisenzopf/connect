import {readFile,writeFile,rename,stat} from 'node:fs/promises';
import {join} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';

import {nativeProgrammeBridge} from './native_programme_bridge.mjs';
import {now,workerJoinVerified} from './native_programme_profile.mjs';
/** Same-kernel original programme boundaries. Public health polling is owned by
 * the parent; this helper never creates another transport or changes media. */
export function nativeMatrixLifecycle(directory,{owner,originalTerminal,workerReceipt,cutoff=Infinity}={}){
    const bridge=nativeProgrammeBridge();owner??=bridge?.owner;cutoff=Math.min(cutoff,bridge?.cutoff??Infinity);
    let terminal=originalTerminal;
    const announce=async(phase,extra={},when=now())=>{
        const request=join(directory,`resources-${phase}.json`),temporary=request+'.tmp';
        await writeFile(temporary,JSON.stringify({phase,monotonic_seconds:when,...(owner?{programme_owner:owner}:{}),...extra})+'\n',{flag:'wx',mode:0o600});
        await rename(temporary,request);
        return when;
    };
    const exchange=async(phase,extra={})=>{
        const when=await announce(phase,extra);
        const until=Math.min(cutoff,phase==='retired'&&owner?terminal+13:when+(phase==='retired'?13:5)),response=join(directory,`resources-${phase}-response.json`);
        while(now()<until){
            try{
                const info=await stat(response);if(info.size>2048)throw Error('Original resource response bound');
                const value=JSON.parse(await readFile(response,'utf8'));
                if(typeof value.passed!=='boolean')throw Error('Original resource response shape');
                return value;
            }catch(error){if(error.code!=='ENOENT')throw error;}
            await sleep(20);
        }
        throw Error('Original resource response deadline');
    };
    const safely=async work=>{try{return await work();}catch(error){return {passed:false,failure:error.name};}};
    return {
        baseline:()=>safely(()=>exchange('baseline')),
        terminal:()=>{if(terminal!==undefined)throw Error('Original terminal cannot restart');terminal=now();bridge?.terminal(terminal);return safely(async()=>{
            await announce('terminal',{},terminal);
            // A missing observer must never postpone original client cleanup.
            return {passed:true};
        });},
        retired:async({observedHelpers,helpersJoined,closeFailed})=>{
            const local={passed:terminal!==undefined&&observedHelpers>0&&helpersJoined&&!closeFailed,
                observed_helpers:observedHelpers,original_helpers_joined:helpersJoined,
                original_clients_closed:!closeFailed,
                all_local_owners_elapsed_ms:terminal===undefined?null:Math.round((now()-terminal)*1000)};
            local.passed&&=local.all_local_owners_elapsed_ms<=10000;
            if(bridge){bridge.retired(local);return {retirement:local,worker_parent_idle_required:true};}
            if(owner){
                local.original_terminal_monotonic_seconds=terminal;
                local.programme_worker=workerReceipt;
                local.passed&&=workerJoinVerified(workerReceipt,owner,terminal);
            }
            const observation=await safely(()=>exchange('retired',{retirement:local}));
            return {retirement:local,idle_observer:observation};
        },
    };
}
