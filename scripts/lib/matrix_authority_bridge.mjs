import fs from 'node:fs/promises';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {fixturePrivateJwk,fixtureServerJwk} from './browser_matrix_keys.mjs';

export function matrixAuthorityBridge(directory,{cycles=1}={}){
    if(![1,3].includes(cycles))throw Error('Declared original authority cycles');
    const sequences=Array(cycles).fill(0);
    return async (kind,cycle=1)=>{
        if(!Number.isInteger(cycle)||cycle<1||cycle>cycles)throw Error('Original authority cycle');
        const folder=cycles===1?directory:path.join(directory,'cycles',`cycle-${cycle}`);
        await fs.mkdir(folder,{recursive:true,mode:0o700});
        let sequence=sequences[cycle-1];
        if(!['expiry','revocation','revoke'].includes(kind)||++sequence>3)throw Error('Bounded original authority probe');
        const id=sequence;sequences[cycle-1]=sequence;
        const temporary=path.join(folder,`authority-${id}-request.tmp`);
        await fs.writeFile(temporary,JSON.stringify({sequence:id,kind}),{flag:'wx',mode:0o600});
        await fs.rename(temporary,path.join(folder,`authority-${id}-request.json`));
        const cutoff=performance.now()+30000;
        while(performance.now()<cutoff){
            try{
                const file=path.join(folder,`authority-${id}-response.json`),stat=await fs.lstat(file);
                if(!stat.isFile()||stat.isSymbolicLink()||stat.size>65536)throw Error('Bounded private authority response');
                const result=JSON.parse(await fs.readFile(file,'utf8'));
                if(result.failure)throw Error('Authority fixture '+result.failure);
                const actor=result.actor;
                if(actor){
                    actor.signing.jwk=fixturePrivateJwk(Buffer.from(actor.signing.privatePkcs8,'base64url'),actor.signing.algorithm,
                        Buffer.from(actor.signing.publicRaw,'base64url'));
                    actor.server.jwk=await fixtureServerJwk(Buffer.from(actor.server.publicRaw,'base64url'),actor.server.algorithm);
                    delete actor.signing.privatePkcs8;delete actor.signing.publicRaw;delete actor.server.publicRaw;
                }
                return result;
            }catch(error){if(error.code!=='ENOENT')throw error;}
            await delay(25);
        }
        throw Error('Original authority fixture deadline');
    };
}
