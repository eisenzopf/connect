const check=(ok,message)=>{if(!ok)throw Error(message);};
const number=value=>{check(Number.isFinite(value)&&value>=0,'Original numeric diagnostic required');return value;};
const nativeFields=['sent','received','rejected','decodedAudio','decodedVideo','queuedPackets','queuedBytes','droppedPackets','activeBindings'];
const rtcStates=new Set(['new','connecting','connected','disconnected','failed','closed']);

/** Bounded diagnostic ownership of the exact original SDK shutdown calls.
 * Release only diagnostic references after their actual joins; never close,
 * retry, reconnect or force collection of the SDK owners here. */
export function browserMatrixRetirementHooks(Peer,Rtc,{log=(...args)=>console.error(...args)}={}){
    const peers=new Map(),rtcs=new Map(),originalNative=Peer.prototype.shutdown,originalRtc=Rtc.prototype.shutdown,
        originalBegin=Peer.prototype.beginReplacementConnection;
    let revision=0,closing,foreignOwner=false,registrationFailure=false;
    const text=value=>typeof value==='string'?value.slice(0,64):null;
    const failureDetails=error=>Object.freeze({name:text(error?.name),code:text(error?.code),kind:text(error?.kind),certainty:text(error?.certainty)});
    const track=(owners,bound,original,receiver,args,Type)=>{
        if(!(receiver instanceof Type)){foreignOwner=true;throw Error('Foreign original shutdown owner');}
        let entry=owners.get(receiver);
        if(!entry){if(owners.size>=bound){registrationFailure=true;throw Error('Bounded original diagnostic owners');}
            entry={owner:receiver,promises:new Set(),pending:0,failed:false,failure:null};owners.set(receiver,entry);}
        revision++;let work;
        try{work=original.apply(receiver,args);}catch(error){entry.failed=true;entry.failure=failureDetails(error);throw error;}
        if(!work||typeof work.then!=='function'){entry.failed=true;return work;}
        if(!entry.promises.has(work)){
            entry.promises.add(work);entry.pending++;
            void work.then(()=>{entry.pending--;},error=>{entry.pending--;entry.failed=true;entry.failure??=failureDetails(error);});
        }
        return work;
    };
    const nativeHook=function(...args){return track(peers,7,originalNative,this,args,Peer);};
    const rtcHook=function(...args){return track(rtcs,10,originalRtc,this,args,Rtc);};
    const beginHook=function(...args){
        const original=originalBegin.apply(this,args);
        void original.completed.catch(error=>log('Original replacement rejection',String(error.code??error.name),String(error.message),String(error.stack)));
        return original;
    };
    Peer.prototype.shutdown=nativeHook;Rtc.prototype.shutdown=rtcHook;Peer.prototype.beginReplacementConnection=beginHook;
    const owned=()=>Peer.prototype.shutdown===nativeHook&&Rtc.prototype.shutdown===rtcHook
        &&Peer.prototype.beginReplacementConnection===beginHook;
    return {finish(){return closing??=(async()=>{
        const startedRevision=revision,entries=[...peers.values(),...rtcs.values()];
        await Promise.allSettled(entries.flatMap(entry=>[...entry.promises]));
        const failures=[];
        if(foreignOwner)failures.push('foreign-original-shutdown-owner');
        if(registrationFailure)failures.push('original-diagnostic-owner-bound');
        if(!owned())failures.push('foreign-diagnostic-hook');
        if(startedRevision!==revision)failures.push('shutdown-started-during-diagnostic-release');
        if(entries.some(entry=>entry.pending!==0||entry.failed))failures.push('original-shutdown-unjoined-or-failed');
        let native=[],rtc=[];
        try{
            native=[...peers.values()].map(entry=>{
                const d=entry.owner.diagnostics;
                return Object.freeze({original_shutdown_joined:entry.pending===0&&!entry.failed,
                    ...Object.fromEntries(nativeFields.map(key=>[key,number(d[key])]))});
            });
            rtc=[...rtcs.values()].map(entry=>{
                const d=entry.owner.diagnostics;
                check(typeof d.closed==='boolean'&&rtcStates.has(d.peerConnectionState)
                    &&typeof d.captureActive==='boolean'&&typeof d.renderActive==='boolean','Original RTC diagnostic shape');
                return Object.freeze({original_shutdown_joined:entry.pending===0&&!entry.failed,closed:d.closed,
                    peerConnectionState:d.peerConnectionState,remoteTracks:number(d.remoteTracks),liveRemoteTracks:number(d.liveRemoteTracks),
                    captureActive:d.captureActive,renderActive:d.renderActive});
            });
            if(native.some(d=>d.activeBindings!==0||d.queuedPackets!==0||d.queuedBytes!==0)
                ||rtc.some(d=>!d.closed||d.peerConnectionState!=='closed'||d.remoteTracks!==0||d.liveRemoteTracks!==0
                    ||d.captureActive||d.renderActive))failures.push('original-media-owner-still-active');
        }catch{failures.push('original-numeric-diagnostics-unavailable');}
        const shutdownFailures=Object.freeze([...peers.values()].map((entry,index)=>({entry,index,transport:'native'}))
            .concat([...rtcs.values()].map((entry,index)=>({entry,index,transport:'rtc'})))
            .filter(({entry})=>entry.failure).map(({entry,index,transport})=>Object.freeze({transport,owner_index:index,...entry.failure})));
        const joined=failures.length===0;
        if(joined){
            Peer.prototype.shutdown=originalNative;Rtc.prototype.shutdown=originalRtc;Peer.prototype.beginReplacementConnection=originalBegin;
            for(const entry of entries){entry.owner=undefined;entry.promises.clear();}
            peers.clear();rtcs.clear();
        }
        return Object.freeze({format:'connect/original-media-diagnostic-retirement/1',joined,
            original_hooks_restored:joined,diagnostic_owner_references_released:joined,
            retained_diagnostic_owners:peers.size+rtcs.size,peers:Object.freeze(native),rtc:Object.freeze(rtc),
            shutdown_failures:shutdownFailures,failures:Object.freeze(failures)});
    })();}};
}


/** Always attempt the original diagnostic joins, then preserve an earlier
 * original local-close failure ahead of any diagnostic failure. */
export async function finishBrowserMatrixRetirement({hooks,closed,reported=false,previousFailure=false}){
    let diagnostic,failure;
    try{diagnostic=await hooks.finish();}catch(error){failure=error;}
    if('failure' in closed)throw closed.failure;
    if(failure)throw failure;
    if(!reported&&!previousFailure&&!diagnostic.joined)throw Error('Original diagnostic hooks did not retire');
    return diagnostic;
}
