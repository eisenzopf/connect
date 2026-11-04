const check=(value,message)=>{if(!value)throw Error(message);};

/** Actual browser transport selection against owned unavailable endpoints.
 * Recover the admission's original mutation; never submit another mutation.
 * All failed physical owners are retained and joined before returning. */
export async function browserMatrixSelection({Sdk,Client,enrollment,endpoints,unavailable,reference,conversationId,journal,report}) {
    const deadline=performance.now()+30000,physicals=[],commands=[],attempts=[];
    const originalOpen=Client.open;
    let fallback,phase='fallback',failure;
    const options=()=>{const timeoutMs=Math.min(8000,Math.floor(deadline-performance.now()));
        check(timeoutMs>0,'Original selection deadline');return {timeoutMs};};
    Object.assign(report,{format:'connect/browser-matrix-selection/1',passed:false,attempts,commands,
        original_operation_id:reference.operation_id,original_reference_recovered:0,
        scope:'Control fallback/refusal and original-operation reads only; no fallback media qualification.'});
    Client.open=function(input,own){return originalOpen.call(this,input,physical=>{
        check(physicals.length<4,'Bounded selection physical owners');physicals.push(physical);
        const begin=physical.begin;
        physical.begin=function(type,payload,...rest){
            check(commands.length<32,'Bounded selection commands');
            commands.push({phase,type,operation_id:typeof payload.operation_id==='string'?payload.operation_id:null});
            return begin.call(this,type,payload,...rest);
        };
        own?.(physical);
    });};
    const connect=contacts=>Sdk.connect({tenantId:'tnt_default',identityId:enrollment.identityId,deviceId:enrollment.device.id,
        endpoints:contacts,preferredTransport:'webtransport',allowFallback:true,...options(),attemptTimeoutMs:3000,
        resolveEnrollment:async context=>{attempts.push({phase,transport:context.transport});return enrollment;}});
    const recover=async()=>{
        const result=await fallback.operations.restore(reference,fallback.operationKinds.conversationCreate).wait(options());
        check(result.snapshot.state==='completed'&&result.value.id===conversationId,'Fallback must recover the same original conversation');
        report.original_reference_recovered++;
        await journal({kind:'selection-original-recovered',phase,operationId:reference.operation_id,reference});
    };
    try{
        await journal({kind:'selection-start',operationId:reference.operation_id,reference});
        fallback=await connect({...endpoints,webtransport:unavailable.webtransport});
        const selected=fallback.transport,capabilities=await fallback.capabilities.refresh(options());
        check(selected.selectedTransport==='wss-webrtc'&&capabilities.selectedTransport==='wss-webrtc'
            &&['preferred-transport-timeout','preferred-transport-failed'].includes(selected.fallbackReason)
            &&capabilities.fallbackReason===selected.fallbackReason,'Actual WSS fallback and explicit reason required');
        report.selected_transport=selected.selectedTransport;report.fallback_reason=selected.fallbackReason;
        await recover();phase='neither';
        let unexpected;
        try{unexpected=await connect(unavailable);}
        catch(error){report.refusal={kind:error.kind,certainty:error.certainty};}
        finally{if(unexpected)await unexpected.close(options());}
        check(!unexpected&&report.refusal?.certainty==='not-submitted'
            &&['transport-uncertain','deadline'].includes(report.refusal?.kind),'Both unavailable paths must explicitly refuse before submission');
        check(attempts.length===4&&attempts.every((a,index)=>a.phase===(index<2?'fallback':'neither')
            &&a.transport===(index%2?'wss-webrtc':'webtransport')),'Both real paths must be attempted in each round');
        phase='recovery-after-refusal';await recover();
        check(commands.length>=2&&commands.every(command=>command.type==='operation.get'
            &&command.operation_id===reference.operation_id&&command.phase!=='neither'),'Selection must only read the original operation, without mutation resubmission');
        report.application_mutations_submitted=0;
    }catch(error){failure=error;report.failure={phase,name:error.name};}
    finally{
        const cleanupDeadline=performance.now()+5000;
        try{
            // Closing is unconditional even if the probe's original request
            // budget expired. All joins share this one teardown deadline.
            for(const physical of physicals)physical.close();
            await fallback?.close({timeoutMs:5000});
            // Failed opens also own real transports. Do not replace a missing
            // disposal with a timeout interpreted as success.
            let timer;
            try{await Promise.race([Promise.all(physicals.map(physical=>physical.disposed)),
                new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Original selection owners did not join')),
                    Math.max(0,cleanupDeadline-performance.now()));})]);}
            finally{clearTimeout(timer);}
            report.cleanup_elapsed_ms=performance.now()-(cleanupDeadline-5000);
            report.original_physical_owners=physicals.length;
            report.original_physical_owners_joined=physicals.length===4&&report.cleanup_elapsed_ms<=5000;
        }catch(error){failure??=error;report.cleanup_failure=error.name;}
        Client.open=originalOpen;
    }
    report.passed=!failure&&report.original_physical_owners_joined===true;
    await journal({kind:'selection-terminal',passed:report.passed});
    if(failure)throw failure;
    check(report.passed,'Four original selection physical owners required');
    return report;
}
