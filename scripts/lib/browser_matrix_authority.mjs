const check=(ok,message)=>{if(!ok)throw Error(message);};

/** Independent current revocation/expiry probes; never resubmit a mutation. */
export async function browserMatrixAuthority({Sdk,provision,endpoints,selected,pin,state,report}){
    const parameters=name=>name==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
    const connect=async configuration=>{
        const key=await crypto.subtle.importKey('jwk',configuration.signing.jwk,parameters(configuration.signing.algorithm),false,['sign']);
        const trust=await crypto.subtle.importKey('jwk',configuration.server.jwk,parameters(configuration.server.algorithm),false,['verify']);
        return Sdk.connect({tenantId:'tnt_default',identityId:configuration.identityId,deviceId:configuration.device.id,
            endpoints,preferredTransport:selected,allowFallback:false,timeoutMs:8000,attemptTimeoutMs:3000,
            resolveEnrollment:async()=>({identityId:configuration.identityId,device:configuration.device,credential:configuration.credential,
                signing:{key,keyId:configuration.signing.keyId,algorithm:configuration.signing.algorithm},
                server:{key:trust,keyId:configuration.server.keyId,algorithm:configuration.server.algorithm,
                    notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt},
                serverCertificateHashes:[{algorithm:'sha-256',value:pin.slice()}]})});
    };
    report.conflict=state.report.conflict??{passed:false};
    for(const kind of ['expiry','revocation']){
        const result=report[kind]={passed:false};let owner,reopened;
        try{
            const issued=await provision(kind);owner=await connect(issued.actor);
            await owner.conversations.byId(state.conversation.id).refresh({timeoutMs:3000});
            result.current_read_before_boundary=true;
            if(kind==='expiry'){
                const wait=issued.expires_at*1000-Date.now()+100;
                check(wait>0&&wait<=13000,'Original current expiry window');
                await new Promise(resolve=>setTimeout(resolve,wait));
                result.original_expiry_elapsed=Date.now()>=issued.expires_at*1000;
            }else check((await provision('revoke')).revoked===true,'Original credential revocation');
            try{await owner.conversations.byId(state.conversation.id).refresh({timeoutMs:3000});}
            catch(error){result.live_read_refusal={kind:error.kind??null,certainty:error.certainty??null};}
            check(result.live_read_refusal,'Current live read remained authorized');
            // A fresh physical open with the exact same credential must refuse
            // authentication; a request timeout alone cannot pass this case.
            try{reopened=await connect(issued.actor);}
            catch(error){result.authentication_refusal={kind:error.kind??null,certainty:error.certainty??null};}
            check(!reopened&&result.authentication_refusal?.kind==='authentication','Expired/revoked credential must explicitly fail authentication');
            result.passed=true;
        }catch(error){result.failure={name:error.name,kind:error.kind??null};}
        finally{
            const joins=await Promise.allSettled([owner,reopened].filter(Boolean).map(client=>client.close({timeoutMs:5000})));
            result.original_owners_joined=joins.every(value=>value.status==='fulfilled');
            result.passed&&=result.original_owners_joined;
        }
    }
    report.passed=['expiry','revocation','conflict'].every(kind=>report[kind]?.passed===true);
    return report;
}
