import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';

const monotonicMs=()=>Number(process.hrtime.bigint())/1e6;
const hash=value=>createHash('sha256').update(value).digest('hex');
const terminal=new Set(['ended','cancelled','rejected','failed']);

/** Custody of the programme's original public Session handles and end intents.
 * No Session, client, credential or replacement operation is created here. */
export function originalSessionRetirement({now=monotonicMs,globalDeadlineMs=Infinity}={}){
    const entries=[];
    const remainingCloseMs=deadline=>Math.max(1,Math.floor(Math.min(deadline,globalDeadlineMs)-now()));
    const failure=value=>{
        const text=field=>typeof field==='string'&&/^[A-Za-z0-9_-]{1,64}$/.test(field)?field:null;
        return {name:text(value?.name),kind:text(value?.kind),certainty:text(value?.certainty)};
    };
    const remaining=deadline=>{
        const timeoutMs=Math.floor(Math.min(deadline,globalDeadlineMs)-now());
        if(timeoutMs<1)throw Object.assign(Error('Original Session cleanup deadline'),{kind:'deadline',certainty:'read-only'});
        return {timeoutMs};
    };
    const finish=(entry,deadline)=>{
        // This is the same original promise even when its effect was uncertain.
        // Never rebuild an end intent with a later topology revision on retry.
        if(entry.attempt)return entry.attempt;
        entry.attempt=(async()=>{
            try{
                assert.equal(entry.session.id,entry.sessionId);
                const current=await entry.session.refresh(remaining(deadline));
                assert.equal(current.id,entry.sessionId);
                if(terminal.has(current.state)){
                    entry.observation.status='already-terminal';
                    entry.observation.observed_terminal_state=current.state;
                    return current;
                }
                const intent=Object.freeze({expectedTopologyRevision:current.topologyRevision,
                    reasonCode:200,reason:'Synthetic fixture complete'});
                const options={...remaining(deadline),operationId:entry.operationId};
                entry.observation.submission_attempted=true;
                const operation=await entry.session.end(intent,options);
                entry.observation.original_reference_sha256=hash(JSON.stringify(operation.referenceJson()));
                const result=await operation.wait(remaining(deadline));
                assert.equal(result.snapshot.state,'completed');
                const ended=await entry.session.refresh(remaining(deadline));
                assert.equal(ended.id,entry.sessionId);assert.equal(ended.state,'ended');
                entry.observation.status='ended';entry.observation.observed_terminal_state='ended';
                return ended;
            }catch(error){
                entry.observation.status=error.certainty==='uncertain'?'uncertain':'failed';
                entry.observation.failure={kind:error.kind??error.name,certainty:error.certainty??null};
                throw error;
            }
        })();
        return entry.attempt;
    };
    return {
        own(session,operationId){
            assert.equal(typeof session.id,'string');assert.equal(typeof operationId,'string');
            assert.equal(typeof session.refresh,'function');assert.equal(typeof session.end,'function');
            assert.ok(!entries.some(e=>e.sessionId===session.id||e.operationId===operationId));
            entries.push({session,sessionId:session.id,operationId,attempt:null,observation:{
                session_index:entries.length,original_operation_sha256:hash(operationId),
                submission_attempted:false,status:'not-attempted'}});
        },
        async end(sessionId,{timeoutMs}){
            const entry=entries.find(e=>e.sessionId===sessionId);assert.ok(entry,'Original owned Session required');
            return finish(entry,Math.min(globalDeadlineMs,now()+timeoutMs));
        },
        cleanupDeadline(){return Math.min(globalDeadlineMs,now()+5000);},
        remainingCloseMs,
        async closeClients(clients,deadline){
            const original=Array.from(clients),cutoff=Math.min(deadline,globalDeadlineMs);
            const observations=original.map((_,index)=>({index,status:'not-started',deadline_met:false}));
            // Each independent original owner starts before awaiting any sibling.
            // Even an already expired cleanup still joins the SDK close with its
            // existing minimum1ms cleanup attempt, and can never qualify.
            const results=await Promise.allSettled(original.map(async(client,index)=>{
                const observation=observations[index],started=now();
                try{
                    await client.close({timeoutMs:remainingCloseMs(cutoff)});
                    observation.status='fulfilled';
                    observation.deadline_met=started<cutoff&&now()<=cutoff;
                }catch(error){
                    observation.status='rejected';
                    observation.failure=failure(error);
                    throw error;
                }
            }));
            return {format:'connect/original-client-retirement/1',
                passed:results.every(result=>result.status==='fulfilled')
                    &&observations.every(observation=>observation.deadline_met),
                original_cutoff_monotonic_ms:cutoff,
                clients:observations.slice(0,16),omitted_clients:Math.max(0,observations.length-16)};
        },
        async cleanup(deadline){
            // Independent original Sessions get the SAME existing absolute
            // cutoff. Promise.allSettled joins every original SDK call; no
            // timeout race detaches a request or grants a second cleanup budget.
            const results=await Promise.allSettled(entries.map(entry=>finish(entry,deadline)));
            return {format:'connect/original-session-retirement/1',passed:results.every(r=>r.status==='fulfilled'),
                sessions:entries.map(entry=>structuredClone(entry.observation)),
                scope:'Original public Session handles and planned end IDs only; no replay after uncertainty or renewed cleanup deadline.'};
        },
    };
}
