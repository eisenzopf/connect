const check=(condition,message)=>{if(!condition)throw Error(message);};
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,Math.max(0,ms)));

/** Application-owned reachability cadence. No reconnect, credential refresh,
 * operation retry or overlapping renewal batch. Close joins the original batch. */
export function browserMatrixKeepalive(actors) {
    const owners=Object.entries(actors).map(([role,actor])=>({role,client:actor.client,
        epoch:actor.client.transport.physicalEpoch,failed:false}));
    check(owners.length>0&&owners.length<=7,'Bounded original reachability owners required');
    const failures=[];let timer,wake,stopped=false,failed=false,failure,rounds=0,requests=0,closing,originalBatchJoined=false;
    const refuse=(owner,error)=>{
        owner.failed=true;
        if(!failed){failed=true;failure=error;}
        failures.push({owner:owner.role,...failureDetails(error)});
    };
    const work=(async()=>{
        try{
            while(!stopped){
                const current=owners.filter(owner=>!owner.failed);
                if(!current.length)break;
                await new Promise(resolve=>{wake=resolve;timer=setTimeout(resolve,20000);});wake=undefined;
                if(stopped)break;
                const outcomes=await Promise.allSettled(current.map(async({client,epoch})=>{
                    check(client.transport.physicalEpoch===epoch,'Keepalive may not reconnect the original Device');
                    requests++;await client.keepAlive({timeoutMs:3000});
                    check(client.transport.physicalEpoch===epoch,'Keepalive changed the physical owner');
                }));
                // Never retry the failed original owner or change its epoch.
                // Its rejection must not stop the other original owners' cadence.
                outcomes.forEach((outcome,index)=>{if(outcome.status==='rejected')refuse(current[index],outcome.reason);});
                rounds++;
            }
        }catch(error){if(!failed){failed=true;failure=error;}}
    })();
    return {check(){if(failed)throw failure;},observation:()=>({rounds,requests,failed,stopped,original_batch_joined:originalBatchJoined,retained_client_references:owners.length,
            failures:failures.map(value=>({...value}))}),
        close(){return closing??=(async()=>{stopped=true;clearTimeout(timer);wake?.();await work;originalBatchJoined=true;owners.length=0;if(failed)throw failure;})();}};
}

function failureDetails(error){return {name:error?.name??'Error',kind:error?.kind??null,certainty:error?.certainty??null,
    problem:error?.problem?{code:error.problem.code,reason:error.problem.reason}:null};}
function retainKeepaliveFailure(report,error){report.keepalive_failure??=failureDetails(error);report.passed=false;}

/** Join the original reachability batch, retain its failure, then attempt the
 * original retirement exactly once. A failure grants no new owner or deadline. */
export async function retireBrowserMatrixProgramme({keepalive,report,retire}){
    try{await keepalive?.close();}catch(error){retainKeepaliveFailure(report,error);}
    return retire();
}

/** Join every original local cleanup even if another close rejects. A repeated
 * keepalive close is the same joined promise already recorded by retirement;
 * any unrecorded reachability, renderer or source failure remains an error. */
export async function closeBrowserMatrixProgramme({keepalive,observer,sources,report,previousFailure=false}){
    const outcomes=await Promise.allSettled([keepalive,observer,sources].map(owner=>
        Promise.resolve().then(()=>owner?.close())));
    const unrecorded=previousFailure?undefined:outcomes.find((value,index)=>value.status==='rejected'
        &&(index!==0||!report.keepalive_failure));
    return {outcomes,...(unrecorded?{failure:unrecorded.reason}:{})};
}

/** Read-only cleanup recovery for at most four already-submitted original moves.
 * This never changes programme results or submits/repeats a replacement. */
export function browserMatrixCleanupMoves(input){
    let {state,actors,plan,journal,operationKind}=input;
    const entries=[],observations=[];let released;
    const sameKey=(key,ref)=>key?.operationId===ref.operation_id&&key.command===ref.command
        &&key.scope?.conversationId===ref.scope.conversation_id&&key.scope?.sessionId===ref.scope.session_id
        &&key.scope?.connectionId===ref.scope.connection_id;
    const reference=operation=>JSON.parse(JSON.stringify(operation.referenceJson()));
    const current=(entry)=>{
        check(!released&&entries.includes(entry),'Original cleanup registry owner');
        check(JSON.stringify(reference(entry.operation))===JSON.stringify(entry.reference),'Original move reference changed');
        check(actors[entry.targetRole]===entry.actor&&entry.actor.deviceId===entry.deviceId
            &&entry.actor.endpointId===entry.endpointId&&entry.actor.client===entry.targetClient
            &&entry.targetClient.transport.physicalEpoch===entry.targetEpoch
            &&actors[entry.sourceRole]?.client===entry.sourceClient
            &&entry.sourceClient.transport.physicalEpoch===entry.sourceEpoch,'Original target or source actor changed');
    };
    const api={
        track({intent,route,before,targetSession,retain,sourceSnapshot}){
            check(!released&&entries.length<4&&plan.moves.includes(intent)&&plan.routes[route.index]===route,
                'Bounded original cleanup move');
            const actor=actors[intent.targetRole],sourceClient=actors[before.role].client;
            const original={schema:'connect-operation-reference/1',operation_id:intent.operation,command:'connection.replace',
                scope:{conversation_id:state.conversation.id,session_id:plan.sessions[route.session].id,connection_id:before.connection.id},
                continuation:{kind:'replacement',retained_connection_id:retain,target:{kind:'endpoint',id:actor.endpointId}}};
            // A public read-only reference exists before submission returns.
            // Unknown acceptance still queries this one original key, never a
            // replacement offer or another logical mutation.
            const operation=sourceClient.operations.restore(original,operationKind),ref=reference(operation);
            check(!entries.some(e=>e.reference.operation_id===intent.operation),'Duplicate original cleanup move');
            check(ref.schema==='connect-operation-reference/1'&&ref.command==='connection.replace'
                &&ref.operation_id===intent.operation&&ref.scope?.conversation_id===state.conversation.id
                &&ref.scope?.session_id===plan.sessions[route.session].id&&ref.scope?.connection_id===before.connection.id
                &&ref.continuation?.kind==='replacement'&&ref.continuation.retained_connection_id===retain
                &&ref.continuation.target?.kind==='endpoint'&&ref.continuation.target.id===actor.endpointId,
                'Original cleanup move scope');
            check(sourceSnapshot?.id===before.connection.id&&sourceSnapshot.conversationId===state.conversation.id
                &&sourceSnapshot.sessionId===plan.sessions[route.session].id&&sourceSnapshot.state==='ready'
                &&Number.isSafeInteger(sourceSnapshot.generation)&&sourceSnapshot.generation>0
                &&sourceSnapshot.generation<Number.MAX_SAFE_INTEGER-1,'Original cleanup source generation');
            const entry={routeIndex:route.index,ordinal:intent.ordinal,source:before.connection,operation,reference:ref,
                targetSession,actor,targetRole:intent.targetRole,deviceId:actor.deviceId,endpointId:actor.endpointId,
                targetClient:actor.client,targetEpoch:actor.client.transport.physicalEpoch,sourceRole:before.role,
                sourceClient:actors[before.role].client,sourceEpoch:actors[before.role].client.transport.physicalEpoch,
                generation:sourceSnapshot.generation+1,participantId:sourceSnapshot.participantId,retain,candidate:undefined};
            entries.push(entry);return entry;
        },
        accepted(entry,operation){current(entry);check(JSON.stringify(reference(operation))===JSON.stringify(entry.reference),
            'Returned original move reference changed');entry.operation=operation;},
        prepared(entry,connection){current(entry);check(connection.conversationId===entry.reference.scope.conversation_id
            &&connection.sessionId===entry.reference.scope.session_id&&typeof connection.id==='string'
            &&connection.id.startsWith('replacement_'),'Original prepared cleanup target');entry.candidate=connection;},
        async resolve(connection,index,options){
            check(!released,'Original cleanup registry released');
            const found=entries.filter(e=>e.routeIndex===index&&e.source===connection);
            check(found.length<=1,'Ambiguous original cleanup lineage');
            if(!found.length)return {connection,snapshot:await connection.refresh(options())};
            const entry=found[0];current(entry);
            const status=await entry.operation.status(options());current(entry);
            check(sameKey(status.key,entry.reference),'Original cleanup status key');
            if(status.state!=='completed'){
                check(['unknown','accepted','preparing','ready','committing','failed','cancelled'].includes(status.state),
                    'Unknown original cleanup state');
                const snapshot=await connection.refresh(options());current(entry);
                check(snapshot.state==='ready'&&snapshot.generation===entry.generation-1,
                    'Unresolved original move cannot supply a cleanup target');
                observations.push({route_index:index,operation_id:entry.reference.operation_id,record_id:status.recordId,
                    state:status.state,recovered:false,connection_id:connection.id});
                return {connection,snapshot};
            }
            // wait observes the same SDK operation and uses its existing typed
            // completion/provenance parser. No new offer or mutation is issued.
            check(typeof status.recordId==='string'&&status.recordId.length,'Original cleanup status record');
            const complete=await entry.operation.wait(options());current(entry);
            const moved=complete.value,id='replacement_'+status.recordId;
            check(complete.snapshot?.state==='completed'&&complete.snapshot.recordId===status.recordId
                &&sameKey(complete.snapshot.key,entry.reference)&&complete.snapshot.revision>=status.revision,
                'Original cleanup completion record');
            check(moved?.activationState==='ready'&&moved.connection?.id===id
                &&moved.connection.conversationId===entry.reference.scope.conversation_id
                &&moved.connection.sessionId===entry.reference.scope.session_id
                &&moved.binding?.generation===entry.generation&&moved.binding.participantId===entry.participantId
                &&Number.isSafeInteger(moved.binding.membershipEpoch)&&moved.binding.membershipEpoch>0
                &&moved.binding.deviceId===entry.deviceId&&moved.binding.endpointId===entry.endpointId
                &&moved.retired?.length===1&&moved.retired[0]===connection.id
                &&moved.retained?.length===1&&moved.retained[0].id===entry.retain
                &&moved.admitted?.length===1&&moved.admitted[0].id===id,
                'Original completed move cleanup provenance');
            check(!entry.candidate||entry.candidate.id===id,'Original prepared candidate changed');
            // The receipt wrapper belongs to the submitter. Use the target's
            // original public handle/client so current native custody is exact.
            const target=entry.candidate??entry.targetSession.connections.byId(id);
            const snapshot=await target.refresh(options());current(entry);
            check(snapshot.id===id&&snapshot.conversationId===entry.reference.scope.conversation_id
                &&snapshot.sessionId===entry.reference.scope.session_id&&snapshot.state==='ready'
                &&snapshot.generation===entry.generation&&snapshot.participantId===entry.participantId,
                'Original cleanup target is not current');
            const source=await connection.refresh(options());current(entry);
            check(source.id===connection.id&&source.state==='retired','Original move did not retire this source');
            const proof={route_index:index,operation_id:entry.reference.operation_id,record_id:status.recordId,
                state:'completed',recovered:true,source_connection_id:connection.id,connection_id:id,generation:entry.generation};
            await journal({kind:'cleanup-move-recovered',...proof});current(entry);
            check(state.connections[index]===connection,'Original cleanup source handle changed');
            options();state.connections[index]=target;observations.push(proof);
            return {connection:target,snapshot,recovery:proof};
        },
        release(){
            if(released)return released;
            const count=entries.length;
            for(const entry of entries){entry.source=entry.operation=entry.targetSession=entry.actor=entry.candidate=entry.targetClient=entry.sourceClient=undefined;}
            const retained=entries.reduce((n,entry)=>n+['source','operation','targetSession','actor','candidate','targetClient','sourceClient']
                .filter(key=>entry[key]!==undefined).length,0);check(retained===0,'Original cleanup references retained');
            entries.length=0;state=actors=plan=journal=operationKind=undefined;
            released=Object.freeze({released:true,original_move_records:count,retained_move_records:entries.length,
                retained_sdk_references:retained,observations:observations.map(value=>({...value}))});
            return released;
        },
    };
    return Object.freeze(api);
}

/** Real six-source, two-Session programme and four original device moves.
 * Independent decoded receive/capture/expiry/idle proof is still required by
 * the full cell verifier; source draws and completed moves cannot stand in. */
export async function browserMatrixProgramme({state,actors,plan,sources,journal,keepalive,report,observer,selectedTransport,cleanupMoves}) {
    check(plan.programmeMs===62000&&plan.moves.length===4,'Approved fixed programme and four moves required');
    let decoderStartupFailure;
    if(observer){try{await observer.initial(state,{timeoutMs:8000});}
        catch(error){decoderStartupFailure={name:error.name,kind:error.kind??null};}}
    const original=sources.beginProgramme();
    observer?.start(original);
    Object.assign(report,{format:'connect/browser-matrix-programme/1',passed:false,source_programme:original,
        decoded_media_verified:false,packet_capture_verified:false,moves:[],failures:[],blocked_moves:[],terminal_ms:original.deadlineMs});
    if(decoderStartupFailure)report.decoder_startup_failure=decoderStartupFailure;
    await journal({kind:'programme-origin',original});
    // stream.subscribe creates a dedicated local receiver named after the
    // selected publication. Transfer that port's existing selection; the
    // original sendrecv publication port has no direct subscription to move.
    const current=plan.routes.map(route=>{
        const peers=plan.routes.filter(other=>other.session===route.session),index=peers.indexOf(route);
        return {connection:state.connections[route.index],role:route.role,video:route.video,
            receiveVideo:selectedTransport==='wss-webrtc'?route.video:peers[(index+peers.length-1)%peers.length].video};
    });
    async function move(intent){
        const route=plan.routes[intent.source],before=current[intent.source],target=actors[intent.targetRole];
        if(intent.ordinal===1&&!report.moves.some(value=>value.source_index===route.index&&value.ordinal===0)){
            report.blocked_moves.push({operation_id:intent.operation,source_index:route.index,ordinal:intent.ordinal,
                reason:'outbound-move-failed; no return mutation submitted'});return;
        }
        const began=performance.now(),cutoff=Math.min(began+8000,original.deadlineMs);
        const options=()=>{const timeoutMs=Math.floor(cutoff-performance.now());check(timeoutMs>0,'Original movement deadline');return {timeoutMs};};
        let operation,targetOwner,cleanupEntry;
        try{
            const sid=plan.sessions[route.session].id,cid=state.conversation.id;
            const sourceSession=actors[before.role].client.conversations.byId(cid).sessions.byId(sid);
            const targetSession=target.client.conversations.byId(cid).sessions.byId(sid);
            const peers=plan.routes.filter(other=>other.session===route.session),retained=peers.find(other=>other.index!==route.index);
            check(typeof target.endpointId==='string'&&target.endpointId.length,'Actual provisioned target Endpoint required');
            const snapshot=await sourceSession.refresh(options());
            const sourceSnapshot=cleanupMoves?await before.connection.refresh(options()):undefined;
            const command={retain:current[retained.index].connection.id,target:{endpoint:target.endpointId},
                expectedTopologyRevision:snapshot.topologyRevision,video:{publications:[{previous:before.video,replacement:intent.video}],
                    receivers:[{previous:before.receiveVideo,replacement:intent.receiveVideo}]}};
            await journal({kind:'move-intent',name:`move-${route.index}-${intent.ordinal}`,operationId:intent.operation,
                sourceConnectionId:before.connection.id,command,scheduledStartMs:intent.scheduledStartMs,actualStartMs:began-original.originMs});
            cleanupEntry=cleanupMoves?.track({intent,route,before,targetSession,retain:command.retain,sourceSnapshot});
            operation=await before.connection.replaceDevice(command,{...options(),operationId:intent.operation});
            if(cleanupEntry)cleanupMoves.accepted(cleanupEntry,operation);
            const reference=operation.referenceJson();
            await journal({kind:'move-accepted',operationId:intent.operation,reference});
            targetOwner=await targetSession.connections.prepareReplacement({operation:operation.reference,audio:intent.audio,
                video:{send:intent.video,receive:intent.receiveVideo}},options());
            if(cleanupEntry)cleanupMoves.prepared(cleanupEntry,targetOwner.connection);
            await journal({kind:'move-target-prepared',operationId:intent.operation,connectionId:targetOwner.connection.id});
            await sources.start({route,connection:targetOwner.connection,options:options()});
            if(observer)await observer.attach(route.index,targetOwner.connection,{audio:intent.audio,video:intent.receiveVideo,sendVideo:intent.video},options());
            // Both receipts retain their original owners; do not cancel or
            // resubmit a mutation when one response is uncertain.
            const outcomes=await Promise.allSettled([operation.wait(options()),targetOwner.completion]);
            const rejected=outcomes.find(outcome=>outcome.status==='rejected');if(rejected)throw rejected.reason;
            const moved=outcomes[0].value.value,prepared=outcomes[1].value.value;
            check(moved.connection.id===targetOwner.connection.id&&prepared.connection.id===moved.connection.id,
                'Both original owners must identify the same admitted target');
            check(moved.activationState==='ready'&&moved.binding.deviceId===target.deviceId&&moved.binding.endpointId===target.endpointId,
                'Original current target Device and Endpoint must be ready');
            check(moved.retired.length===1&&moved.retired[0]===before.connection.id&&moved.retained.length===1&&
                moved.retained[0].id===command.retain,'Original source retirement and retained peer');
            const source=sources.sources.find(source=>source.index===route.index),binding=source.attachments.find(binding=>binding.connectionId===before.connection.id);
            check(binding?.capture.closed===true,'Original SDK capture must close after movement receipt');
            // Both original receipts already prove the ready target and exact
            // source retirement. Observe its subsequent genuine decoder output
            // now; an additional verification read must not manufacture a gap.
            // Prepared outputs remain excluded and are never replayed.
            observer?.activate(route.index,targetOwner.connection.id);
            const old=await before.connection.refresh(options());check(old.state==='retired','Original source must be durably retired');
            sources.retireConnection(before.connection.id);
            current[route.index]={connection:targetOwner.connection,role:intent.targetRole,video:intent.video,receiveVideo:intent.receiveVideo};
            state.connections[route.index]=targetOwner.connection;
            const result={operation_id:intent.operation,source:before.connection.id,target:moved.connection.id,
                source_index:route.index,ordinal:intent.ordinal,scheduled_start_ms:intent.scheduledStartMs,
                actual_start_ms:began-original.originMs,completed_ms:performance.now()-original.originMs,
                original_capture_closed:true,source_master_kind:'caller-owned-audio-node+caller-owned-canvas',
                source_master_continues:source.ownerLive()};
            check(result.source_master_continues,'Movement cannot restart or retire the original master');
            report.moves.push(result);await journal({kind:'move-completed',...result});
        }catch(error){
            report.failures.push({operation_id:intent.operation,source_index:route.index,ordinal:intent.ordinal,
                name:error.name,kind:error.kind??null,certainty:error.certainty??null,
                problem:error.problem?{code:error.problem.code,reason:error.problem.reason}:null});
            await journal({kind:'move-failure',operationId:intent.operation,reference:operation?.referenceJson()??cleanupEntry?.reference??null,
                certainty:error.certainty??null,kindOfFailure:error.kind??error.name});throw error;
        }
    }
    for(const offset of [15000,40000]){
        await sleep(original.originMs+offset-performance.now());
        const outcomes=await Promise.allSettled(plan.moves.filter(move=>move.scheduledStartMs===offset).map(move));
        // Keep the fixed original programme and every independent observer
        // running after a failed move. Its return depends on that move and is
        // recorded as blocked above, never retried or submitted against self.
        report.failed_move_attempts=(report.failed_move_attempts??0)+outcomes.filter(outcome=>outcome.status==='rejected').length;
    }
    // Timers may wake before the requested absolute instant. Retain the same
    // original deadline and observe the full window before recording terminal.
    while(performance.now()<original.deadlineMs)await sleep(original.deadlineMs-performance.now());
    report.finished_ms=performance.now()-original.originMs;
    report.sources=sources.observation();report.passed=report.moves.length===4;
    // Aggregate the original failure only after the fixed programme finishes.
    // Each actual SDK operation still performs its own current authorization.
    try{keepalive.check();}catch(error){retainKeepaliveFailure(report,error);}
    if(observer)report.decoded_media=observer.report();
    await journal({kind:'programme-terminal',elapsedMs:report.finished_ms,moves:report.moves.length});
    return report;
}
