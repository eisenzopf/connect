import assert from 'node:assert/strict';

const refusal=error=>({kind:error.kind??null,certainty:error.certainty??null,
    ...(error.problem?{problem:{code:error.problem.code,category:error.problem.category,reason:error.problem.reason}}:{})});

/** One current credential boundary on one saved operation. No changed intent,
 * new operation, replacement credential or timeout-only authentication proof. */
export async function nativeAuthorityCase({kind,operationId,expiresAt,open,boundary,saveReference,
    observeHelpers,helpersJoined,now=()=>Date.now()/1000,monotonicNow=()=>performance.now(),
    sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms))}){
    assert.ok(['expiry','revocation'].includes(kind));
    const result={passed:false,executed:true,kind,mutation_submissions:0,opened_client_owners:0,closed_client_owners:0};
    const owners=[];let reference;
    const createOwner=()=>{const owner=open();owners.push(owner);result.opened_client_owners++;return owner;};
    const options={timeoutMs:3000};
    try{
        const owner=createOwner();
        assert.equal((await owner.capabilities.refresh(options)).selectedTransport,'quic');
        result.selected_transport='quic';
        result.mutation_submissions++;
        const original=await owner.conversations.create({policy:'persistent',participants:[]},{...options,operationId});
        reference=original.referenceJson();await saveReference(reference);
        const completed=await original.wait(options);
        assert.equal(completed.snapshot.state,'completed');
        assert.ok(completed.snapshot.recordId);
        const restored=owner.operations.restore(reference,owner.operationKinds.conversation_create);
        assert.deepEqual(restored.referenceJson(),reference);
        const before=await restored.status(options);
        assert.equal(before.recordId,completed.snapshot.recordId);assert.equal(before.state,'completed');
        result.current_original_read_before_boundary=true;
        await observeHelpers();
        if(kind==='expiry'){
            const delay=expiresAt-now();assert.ok(delay>0&&delay<=13,'Original expiry window must remain bounded');
            // A monotonic timer can finish before the original UTC boundary.
            // Recheck the same expiry within the original maximum wait.
            const cutoff=monotonicNow()+13100;
            await sleep(delay*1000+100);
            while(now()<expiresAt){
                const remaining=cutoff-monotonicNow();
                if(remaining<=0)break;
                await sleep(Math.min(remaining,Math.max(1,Math.ceil((expiresAt-now())*1000))));
            }
            result.original_expiry_elapsed=now()>=expiresAt;assert.equal(result.original_expiry_elapsed,true);
        }else{
            const observation=await boundary();assert.equal(observation.revoked,true);
            result.original_credential_revoked=true;
        }
        try{await restored.status(options);}catch(error){result.live_original_refusal=refusal(error);}
        assert.ok(result.live_original_refusal,'Live original operation remained readable');
        assert.ok(['authentication','cancelled','transport-uncertain','application'].includes(result.live_original_refusal.kind));
        assert.ok(['read-only','refused'].includes(result.live_original_refusal.certainty));
        if(result.live_original_refusal.kind==='application')
            assert.ok(result.live_original_refusal.certainty==='refused'&&[401,403].includes(result.live_original_refusal.problem?.code));
        const fresh=createOwner(),same=fresh.operations.restore(reference,fresh.operationKinds.conversation_create);
        assert.deepEqual(same.referenceJson(),reference);
        try{await same.status(options);}catch(error){result.fresh_original_refusal=refusal(error);}
        // A closed live QUIC transport alone is insufficient. A fresh native
        // authentication must explicitly refuse the exact original credential.
        assert.equal(result.fresh_original_refusal?.kind,'authentication');
        assert.ok(['read-only','refused','not-submitted'].includes(result.fresh_original_refusal.certainty));
        assert.deepEqual(original.referenceJson(),reference);
        result.original_reference_preserved=true;result.passed=true;
    }catch(error){result.failure={name:error.name,kind:error.kind??null,certainty:error.certainty??null};}
    finally{
        for(const owner of owners){
            try{await owner.close({timeoutMs:5000});result.closed_client_owners++;}
            catch(error){result.close_failure={name:error.name,kind:error.kind??null};}
        }
        try{result.original_helpers_joined=await helpersJoined();}
        catch(error){result.original_helpers_joined=false;result.join_failure=error.name;}
        result.original_owners_joined=result.opened_client_owners===result.closed_client_owners&&result.original_helpers_joined;
        result.passed&&=result.original_owners_joined;
    }
    return result;
}
