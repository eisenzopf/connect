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
        const result=report[kind]={passed:false};let owner,reopened,stage='original-issue';
        try{
            const issued=await provision(kind);
            result.original_issued_at=issued.issued_at;result.original_expires_at=issued.expires_at;
            stage='original-open';owner=await connect(issued.actor);
            stage='original-current-read';
            await owner.conversations.byId(state.conversation.id).refresh({timeoutMs:3000});
            result.current_read_before_boundary=true;
            if(kind==='expiry'){
                stage='original-expiry-wait';
                const startedDate=Date.now(),startedPerformance=performance.now();
                const expiryDate=issued.expires_at*1000,targetDate=expiryDate+100;
                const wait=targetDate-startedDate;
                result.expiry_boundary={started_date_ms:startedDate,started_performance_ms:startedPerformance,
                    original_issued_at:issued.issued_at,original_expires_at:issued.expires_at,requested_wait_ms:wait,
                    original_target_date_ms:targetDate,original_observation_margin_ms:100};
                check(wait>0&&wait<=13000,'Original current expiry window');
                const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
                const cutoff=startedPerformance+13000;
                await sleep(wait);
                // A timer can wake while the original wall clock is still
                // before the original observation target. Preserve its 100 ms
                // margin through every fallback, with the same credential and
                // original 13 second wait budget.
                while(Date.now()<targetDate){
                    const remaining=cutoff-performance.now();
                    if(remaining<=0)break;
                    await sleep(Math.min(remaining,Math.max(1,targetDate-Date.now())));
                }
                const observedDate=Date.now(),observedPerformance=performance.now();
                Object.assign(result.expiry_boundary,{observed_date_ms:observedDate,observed_performance_ms:observedPerformance,
                    observed_wall_elapsed_ms:observedDate-startedDate,observed_monotonic_elapsed_ms:observedPerformance-startedPerformance});
                result.original_expiry_elapsed=observedDate>=expiryDate;
                result.original_observation_margin_elapsed=observedDate>=targetDate;
                stage='original-expiry-boundary';
                check(result.original_expiry_elapsed,'Original credential expiry has not elapsed');
                check(result.original_observation_margin_elapsed,'Original credential expiry observation margin has not elapsed');
            }else {stage='original-revocation';check((await provision('revoke')).revoked===true,'Original credential revocation');}
            stage='original-live-read';
            try{await owner.conversations.byId(state.conversation.id).refresh({timeoutMs:3000});}
            catch(error){result.live_read_refusal={kind:error.kind??null,certainty:error.certainty??null};}
            check(result.live_read_refusal,'Current live read remained authorized');
            // A fresh physical open with the exact same credential must refuse
            // authentication; a request timeout alone cannot pass this case.
            stage='original-fresh-open';
            try{reopened=await connect(issued.actor);}
            catch(error){result.authentication_refusal={kind:error.kind??null,certainty:error.certainty??null};}
            check(!reopened&&result.authentication_refusal?.kind==='authentication','Expired/revoked credential must explicitly fail authentication');
            result.passed=true;
        }catch(error){result.failure={stage,name:error.name,kind:error.kind??null,certainty:error.certainty??null,message:String(error.message??'').slice(0,256)};}
        finally{
            const joins=await Promise.allSettled([owner,reopened].filter(Boolean).map(client=>client.close({timeoutMs:5000})));
            result.original_owners_joined=joins.every(value=>value.status==='fulfilled');
            result.passed&&=result.original_owners_joined;
        }
    }
    report.passed=['expiry','revocation','conflict'].every(kind=>report[kind]?.passed===true);
    return report;
}
