/** Original browser objects remain private; exported observations contain no
 * SDP, network addresses, credentials or raw track identifiers. */
import fs from 'node:fs/promises';
import path from 'node:path';

/** Executed in the receiver page after the captured restart completes. A
 * negotiation receipt and observations from before it cannot prove recovery. */
export async function observeFreshIceMedia({expectedHz}) {
    if(!Number.isFinite(expectedHz)||expectedHz<100||expectedHz>3000)
        throw Error("Explicit expected decoded tone required");
    const f=window.videoFixture,started=performance.now();
    const after=(performance.timeOrigin+started)/1000;
    while(performance.now()-started<2000){
        const now=(performance.timeOrigin+performance.now())/1000;
        const samples=f.observation.audioSamples.filter(row=>row.unix_seconds>=after).slice(-3);
        const gaps=samples.slice(1).map((row,index)=>row.unix_seconds-samples[index].unix_seconds);
        const changing=f.observation.changingTimes.filter(at=>at>=after&&at<=now);
        if(samples.length===3&&samples.every(row=>Number.isFinite(row.rms)&&row.rms>.001
            &&Number.isFinite(row.frequency_hz)&&Math.abs(row.frequency_hz-expectedHz)<35
            &&row.unix_seconds<=now)&&gaps.every(gap=>gap>0&&gap<=.2)
            &&now-samples.at(-1).unix_seconds<=.2&&changing.length>0&&now-changing.at(-1)<=.2)
            return {status:"passed",seconds:(performance.now()-started)/1000,limit_seconds:2,
                fresh_audio_samples:3,fresh_changing_frames:changing.length,
                maximum_audio_gap_seconds:Math.max(...gaps)};
        await new Promise(resolve=>setTimeout(resolve,25));
    }
    throw Error("Fresh duplex decoded media did not recover within two seconds after ICE completion");
}

export function installIceOwnership() {
    const NativePeer=globalThis.RTCPeerConnection, peers=[];
    const NativeWorker=globalThis.Worker,workers=[];
    if(typeof NativeWorker==="function")globalThis.Worker=class extends NativeWorker {
        constructor(...args){super(...args);workers.push(this);}
    };
    globalThis.RTCPeerConnection=class extends NativePeer {
        constructor(...args){super(...args);peers.push(this);}
        setRemoteDescription(...args){this.connectAppliedRemote=args[0]?.sdp;return super.setRemoteDescription(...args);}
    };
    // Explicit synthetic-fixture diagnostic: only the private catch path below
    // writes these bytes to ignored storage; public reports omit them.
    globalThis.connectIcePrivateDescriptions=()=>peers.map(peer=>({
        local:peer.localDescription?.sdp,remote:peer.remoteDescription?.sdp,applied_remote:peer.connectAppliedRemote,
    }));
    let original,originalWorkerCount;
    const sample=()=>peers.map(peer=>({peer,transceivers:peer.getTransceivers(),
        senders:peer.getSenders().map(sender=>({sender,track:sender.track})),
        receivers:peer.getReceivers().map(receiver=>({receiver,track:receiver.track}))}));
    globalThis.connectIceOwnership=()=>{
        original??=sample();
        originalWorkerCount??=workers.length;
        const current=sample(),same=(a,b,match)=>a.length===b.length&&a.every((v,i)=>match(v,b[i]));
        return {peer_count:peers.length,worker_count:workers.length,gathering:peers.map(p=>p.iceGatheringState),
            original_video_workers_unchanged:workers.length===originalWorkerCount,
            original_media_unchanged:same(original,current,(a,b)=>a.peer===b.peer
            &&same(a.transceivers,b.transceivers,(x,y)=>x===y)
            &&same(a.senders,b.senders,(x,y)=>x.sender===y.sender&&x.track===y.track)
            &&same(a.receivers,b.receivers,(x,y)=>x.receiver===y.receiver&&x.track===y.track))};
    };
}

/** Actual browser SDK and signed wire; discard one verified original reply,
 * then recover the SAME request on the same peer before measuring fresh AV. */
export async function restartOriginalBrowsers(pages,participants,privateDirectory,expectedTones) {
    const results=[];
    try {
    for(let generation=0;generation<2;generation++)for(let index=0;index<pages.length;index++){
        const result=await pages[index].evaluate(async({generation,index,selection})=>{
            const f=window.videoFixture,{client,options}=f.control;
            const ownership=()=>{
                const value=globalThis.connectIceOwnership();
                if(value.peer_count!==1||value.worker_count<1||!value.original_video_workers_unchanged||!value.original_media_unchanged)
                    throw Error("Original browser media ownership changed");
                return value;
            };
            ownership();
            const selected=await client.operationStatus(options.conversationId,selection.operationId,{timeoutMs:3000});
            const revision=selected.result?.stream_revision;
            if(selected.state!=="completed"||!Number.isSafeInteger(revision)||revision<1)throw Error("Original receive revision missing");
            const operationId=`browser-ice-${index}-${generation}`;
            const request=client.request.bind(client);
            let discarded=0,requests=0,firstPayload;
            client.request=async(...args)=>{
                if(args[0]==="connection.update"&&args[1].action==="ice-restart"){
                    (f.iceWireTrace??=[]).push({elapsed_ms:performance.now(),ownership:ownership()});
                    requests++;
                    const payload=JSON.stringify(args[1]);
                    if(firstPayload!==undefined&&firstPayload!==payload)throw Error("Original browser ICE intent changed on recovery");
                    firstPayload??=payload;
                    const frames=await request(...args);
                    if(generation===0&&!discarded){
                        discarded++;
                        const {ConnectError}=await import("/sdk/typescript/dist/errors.js");
                        throw new ConnectError("timeout","Synthetic original reply discarded after signed verification");
                    }
                    return frames;
                }
                return request(...args);
            };
            let receipt;
            try {
                if(generation===0){
                    try {await f.session.restartIce(operationId,revision,generation,{timeoutMs:10000});throw Error("Expected discarded original reply");}
                    catch(error){if(error?.code!=="timeout")throw error;}
                    const saved=f.session.savedIceRestart;
                    if(!saved?.submissionAttempted||saved.scope.operation_id!==operationId||JSON.stringify(saved.payload)!==firstPayload)
                        throw Error("Original browser ICE request was not retained");
                }
                receipt=await f.session.restartIce(operationId,revision,generation,{timeoutMs:10000});
            } finally {client.request=request;}
            ownership();
            if(receipt.operationId!==operationId||receipt.iceGeneration!==generation+1||receipt.streamRevision!==revision
                ||receipt.connectionGeneration!==1||discarded!==(generation===0?1:0)||requests!==(generation===0?2:1))
                throw Error("Original browser restart receipt/recovery mismatch");
            const record=await client.operationStatus(options.conversationId,operationId,{timeoutMs:3000});
            if(record.state!=="completed"||record.result?.resources_changed!==false
                ||record.result.ice_generation!==generation+1)
                throw Error("Original browser operation has no captured completion");
            return {participant:index,operation_id:operationId,ice_generation:receipt.iceGeneration,
                stream_revision:revision,connection_generation:receipt.connectionGeneration,
                discarded_answers:discarded,signed_requests:requests,ownership:ownership()};
        },{generation,index,selection:participants[index].subscribe});
        result.decoded_recovery=await Promise.all(pages.map((page,receiver)=>page.evaluate(observeFreshIceMedia,
            {expectedHz:expectedTones?.[receiver]??(receiver===0?660:440)})));
        results.push(result);
    }
    return {status:"passed",profile:"original-browser-ice/1",results,
        scope:"Two original peers, two generations each and discarded original answers. Each completion requires fresh decoded audio/video within two seconds, followed by the unchanged clean window."};
    } catch(error) {
        if(privateDirectory&&path.resolve(privateDirectory).split(path.sep).includes('.local')){
            await fs.writeFile(path.join(privateDirectory,'ice-error-private.json'),JSON.stringify({
                name:error?.name,message:error?.message,stack:error?.stack,
            }),{flag:'wx',mode:0o600});
            const descriptions=await Promise.all(pages.map(page=>page.evaluate(()=>({
                original:window.videoFixture?.session.savedOffer?.payload?.substrate_setup?.sdp,
                proposed:window.videoFixture?.session.savedIceRestart?.payload?.details?.setup?.sdp,
                physical:globalThis.connectIcePrivateDescriptions?.(),
                wire:window.videoFixture?.iceWireTrace??[],
            })).catch(()=>({unavailable:true}))));
            await fs.writeFile(path.join(privateDirectory,'ice-descriptions-private.json'),JSON.stringify(descriptions),{flag:'wx',mode:0o600});
        }
        throw error;
    }
}
