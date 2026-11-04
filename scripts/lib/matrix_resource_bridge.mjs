// Private fixture handshake only. Both peers use the same Linux monotonic clock.
import fs from 'node:fs/promises';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
export function matrixResourceBridge(directory){
    const phases=['baseline','terminal','retired'];let next=0;
    return async (phase,terminalSeconds)=>{
        if(phase!==phases[next])throw Error('Original resource phase order');
        next++;
        const request=path.join(directory,'resources-'+phase+'.json');
        const temporary=request+'.tmp',response=path.join(directory,'resources-'+phase+'-response.json');
        const observed=Number(process.hrtime.bigint())/1e9;
        const when=phase==='terminal'?terminalSeconds:observed;
        if(!Number.isFinite(when)||when>observed||observed-when>5)throw Error('Original terminal clock required');
        await fs.writeFile(temporary,JSON.stringify({phase,monotonic_seconds:when})+'\n',{flag:'wx',mode:0o600});
        await fs.rename(temporary,request);
        const cutoff=performance.now()+15000;
        while(performance.now()<cutoff){
            try{
                const stat=await fs.lstat(response);
                if(!stat.isFile()||stat.isSymbolicLink()||stat.size>4096)throw Error('Bounded resource response');
                const result=JSON.parse(await fs.readFile(response,'utf8'));
                if(typeof result.passed!=='boolean')throw Error('Original resource verdict missing');
                return {...result,monotonic_seconds:Number(process.hrtime.bigint())/1e9};
            }catch(error){if(error.code!=='ENOENT')throw error;}
            await delay(25);
        }
        throw Error('Original resource observation deadline');
    };
}
