const check=(condition,message)=>{if(!condition)throw Error(message);};
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,Math.max(0,ms)));

/** Application-owned reachability cadence. No reconnect, credential refresh,
 * operation retry or overlapping renewal batch. Close joins the original batch. */
export function browserMatrixKeepalive(actors) {
    const owners=Object.values(actors).map(actor=>({client:actor.client,epoch:actor.client.transport.physicalEpoch}));
    let timer,wake,stopped=false,failure,rounds=0,requests=0;
    const work=(async()=>{
        try{
            while(!stopped){
                await new Promise(resolve=>{wake=resolve;timer=setTimeout(resolve,20000);});wake=undefined;
                if(stopped)break;
                const outcomes=await Promise.allSettled(owners.map(async({client,epoch})=>{
                    check(client.transport.physicalEpoch===epoch,'Keepalive may not reconnect the original Device');
                    requests++;await client.keepAlive({timeoutMs:3000});
                    check(client.transport.physicalEpoch===epoch,'Keepalive changed the physical owner');
                }));
                const rejected=outcomes.find(outcome=>outcome.status==='rejected');
                if(rejected)throw rejected.reason;
                rounds++;
            }
        }catch(error){failure=error;}
    })();
    return {check(){if(failure)throw failure;},observation:()=>({rounds,requests,failed:Boolean(failure),stopped}),
        async close(){stopped=true;clearTimeout(timer);wake?.();await work;if(failure)throw failure;}};
}

/** Real six-source, two-Session programme and four original device moves.
 * Independent decoded receive/capture/expiry/idle proof is still required by
 * the full cell verifier; source draws and completed moves cannot stand in. */
export async function browserMatrixProgramme({state,actors,plan,sources,journal,keepalive,report}) {
    check(plan.programmeMs===62000&&plan.moves.length===4,'Approved fixed programme and four moves required');
    const original=sources.beginProgramme();
    Object.assign(report,{format:'connect/browser-matrix-programme/1',passed:false,source_programme:original,
        decoded_media_verified:false,packet_capture_verified:false,moves:[],terminal_ms:original.deadlineMs});
    await journal({kind:'programme-origin',original});
    // Replacement maps the local negotiated receive port, not the selected
    // remote publication. Initial offers use one sendrecv video stream.
    const current=plan.routes.map(route=>({connection:state.connections[route.index],
        role:route.role,video:route.video,receiveVideo:route.video}));
    async function move(intent){
        const route=plan.routes[intent.source],before=current[intent.source],target=actors[intent.targetRole];
        const began=performance.now(),cutoff=Math.min(began+8000,original.deadlineMs);
        const options=()=>{keepalive.check();const timeoutMs=Math.floor(cutoff-performance.now());check(timeoutMs>0,'Original movement deadline');return {timeoutMs};};
        const sid=plan.sessions[route.session].id,cid=state.conversation.id;
        const sourceSession=actors[before.role].client.conversations.byId(cid).sessions.byId(sid);
        const targetSession=target.client.conversations.byId(cid).sessions.byId(sid);
        const peers=plan.routes.filter(other=>other.session===route.session),retained=peers.find(other=>other.index!==route.index);
        check(typeof target.endpointId==='string'&&target.endpointId.length,'Actual provisioned target Endpoint required');
        const snapshot=await sourceSession.refresh(options());
        const command={retain:current[retained.index].connection.id,target:{endpoint:target.endpointId},
            expectedTopologyRevision:snapshot.topologyRevision,video:{publications:[{previous:before.video,replacement:intent.video}],
                receivers:[{previous:before.receiveVideo,replacement:intent.receiveVideo}]}};
        await journal({kind:'move-intent',name:`move-${route.index}-${intent.ordinal}`,operationId:intent.operation,
            sourceConnectionId:before.connection.id,command,scheduledStartMs:intent.scheduledStartMs,actualStartMs:began-original.originMs});
        let operation,targetOwner;
        try{
            operation=await before.connection.replaceDevice(command,{...options(),operationId:intent.operation});
            const reference=operation.referenceJson();
            await journal({kind:'move-accepted',operationId:intent.operation,reference});
            targetOwner=await targetSession.connections.prepareReplacement({operation:operation.reference,audio:intent.audio,
                video:{send:intent.video,receive:intent.receiveVideo}},options());
            await journal({kind:'move-target-prepared',operationId:intent.operation,connectionId:targetOwner.connection.id});
            await sources.start({route,connection:targetOwner.connection,options:options()});
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
            const old=await before.connection.refresh(options());check(old.state==='retired','Original source must be durably retired');
            const source=sources.sources.find(source=>source.index===route.index),binding=source.attachments.find(binding=>binding.connectionId===before.connection.id);
            check(binding?.capture.closed===true,'Original SDK capture must close after movement receipt');
            sources.retireConnection(before.connection.id);
            current[route.index]={connection:targetOwner.connection,role:intent.targetRole,video:intent.video,receiveVideo:intent.receiveVideo};
            state.connections[route.index]=targetOwner.connection;
            const result={operation_id:intent.operation,source:before.connection.id,target:moved.connection.id,
                source_index:route.index,ordinal:intent.ordinal,scheduled_start_ms:intent.scheduledStartMs,
                actual_start_ms:began-original.originMs,completed_ms:performance.now()-original.originMs,
                original_capture_closed:true,source_master_continues:source.stream.getTracks().every(track=>track.readyState==='live')};
            check(result.source_master_continues,'Movement cannot restart or retire the original master');
            report.moves.push(result);await journal({kind:'move-completed',...result});
        }catch(error){
            await journal({kind:'move-failure',operationId:intent.operation,reference:operation?.referenceJson()??null,
                certainty:error.certainty??null,kindOfFailure:error.kind??error.name});throw error;
        }
    }
    for(const offset of [15000,40000]){
        await sleep(original.originMs+offset-performance.now());keepalive.check();
        const outcomes=await Promise.allSettled(plan.moves.filter(move=>move.scheduledStartMs===offset).map(move));
        const rejected=outcomes.find(outcome=>outcome.status==='rejected');if(rejected)throw rejected.reason;
    }
    await sleep(original.deadlineMs-performance.now());keepalive.check();
    report.finished_ms=performance.now()-original.originMs;
    report.sources=sources.observation();report.passed=report.moves.length===4;
    await journal({kind:'programme-terminal',elapsedMs:report.finished_ms,moves:report.moves.length});
    return report;
}
