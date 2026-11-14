// Fixture-only bounded delivery barrier. Never verifies or grants authority.
// Actual bytes/events enter the unchanged canonical reader after release.
export function installOriginalCompletionGate({holdMs,source=0,ordinal=0}) {
    if(!Number.isInteger(holdMs)||holdMs<0||holdMs>500||source!==0||ordinal!==0)
        throw Error('Explicit bounded first original completion counter required');
    const Native=globalThis.WebSocket,now=()=>performance.now();
    if(typeof Native!=='function'||globalThis.connectOriginalCompletionGate)
        throw Error('One original WebSocket completion gate required');
    const nativeAdd=Native.prototype.addEventListener,nativeRemove=Native.prototype.removeEventListener;
    let armed=null,timer=null,closed=false,holding=false,released=false,heldBytes=0;
    let expired=false,overflow=false,callbackFailure=false,opened=0,ended=0,groupCount=0;
    const held=[],blocked=[],events=[],pcm=[],counts=new Map(),peers=new Map(),detach=[];
    const originalTag=globalThis.connectPeerTag;
    const tag=function(peer,id){const result=originalTag?.apply(this,arguments);
        if(!closed&&typeof id==='string'){if(peers.has(id)||peers.size<16)peers.set(id,peer);else overflow=true;}
        return result;};
    globalThis.connectPeerTag=tag;
    const event=(kind,fields={})=>{if(events.length<48)events.push({kind,at_ms:now(),...fields});else overflow=true;};
    const invoke=(listener,socket,e)=>typeof listener==='function'?listener.call(socket,e):listener.handleEvent(e);
    function release(reason) {
        if(timer!==null){clearTimeout(timer);timer=null;}
        holding=false;released=true;event('release',{reason,held_events:held.length,held_bytes:heldBytes});
        const originals=held.splice(0);heldBytes=0;
        // Detach custody before invoking callbacks; every original is attempted
        // even if a diagnostic listener unexpectedly throws.
        for(const row of originals)try{if(row.active())invoke(row.listener,row.socket,row.original);}
            catch{callbackFailure=true;}finally{row.done();}
    }
    function selected(socket,e) {
        if(closed||!armed||released||typeof e.data!=='string'||e.data.length>1048576)return false;
        let f;try{f=JSON.parse(e.data);}catch{return false;}
        const bounded=x=>typeof x==='string'&&x.length>0&&x.length<=128;
        if(!bounded(f.in_reply_to))return false;
        const sameGroup=blocked.some(row=>row.socket===socket&&row.reply===f.in_reply_to);
        // A reply-ID collision never delays auth, ICE, retirement or other
        // control traffic. Only this operation.get batch's ACK/error settles it.
        if(sameGroup&&['ack','error'].includes(f.type))return true;
        const p=f.payload,r=p?.result;
        if(!bounded(f.id)||!bounded(f.connid)||!bounded(p?.record_id)||!bounded(r?.connection_id)
            ||f.type!=='operation.updated'||p?.state!=='completed'||p.command_type!=='connection.replace'
            ||p.operation_id!==armed.operationId||f.cid!==armed.cid||f.sid!==armed.sid
            ||r?.replaces!==armed.old||r?.activation_state!=='ready'
            ||!Array.isArray(r.retained_connection_ids)||r.retained_connection_ids.length!==1
            ||r.retained_connection_ids[0]!==armed.retained)return false;
        // Classification is untrusted. Only later unchanged SDK verification
        // plus both exact public completion results can qualify this counter.
        if(sameGroup)return true;
        if(blocked.length>=4){overflow=true;release('request-bound');return false;}
        blocked.push({socket,reply:f.in_reply_to});groupCount++;
        event('completed-envelope-arrived-unverified',{id:f.id,reply:f.in_reply_to,
            connid:f.connid??null,record_id:p.record_id??null,target:r.connection_id??null});
        if(!holding){
            holding=true;
            const remaining=armed.until-now();
            if(remaining<=0){expired=true;release('original-cutoff');return false;}
            timer=setTimeout(()=>{if(now()>=armed.until)expired=true;release(expired?'original-cutoff':'configured-hold');},Math.min(holdMs,remaining));
        }
        return true;
    }
    function watch(id) {
        const peer=peers.get(id);
        if(!peer){event('peer-lifecycle-unavailable',{owner:id});return;}
        const on=(target,kind,fn)=>{target.addEventListener(kind,fn);detach.push(()=>target.removeEventListener(kind,fn));};
        event('peer-lifecycle-snapshot',{owner:id,state:peer.connectionState});
        on(peer,'connectionstatechange',()=>event('peer-state',{owner:id,state:peer.connectionState}));
        for(const receiver of peer.getReceivers()){
            const track=receiver.track;if(track?.kind!=='audio')continue;
            event('track-lifecycle-snapshot',{owner:id,muted:track.muted,ready_state:track.readyState});
            for(const kind of ['mute','unmute','ended'])on(track,kind,()=>event('track-'+kind,
                {owner:id,muted:track.muted,ready_state:track.readyState}));
        }
    }
    const ProxySocket=new Proxy(Native,{construct(target,args,newTarget){
        const socket=Reflect.construct(target,args,newTarget===ProxySocket?target:newTarget);
        opened++;nativeAdd.call(socket,'close',()=>{ended++;},{once:true});
        const listeners=new Map();
        Object.defineProperties(socket,{
            addEventListener:{configurable:true,value(type,listener,options){
                if(type!=='message'||listener===null)return nativeAdd.call(this,type,listener,options);
                const capture=typeof options==='boolean'?options:options?.capture===true;
                const signal=typeof options==='object'?options?.signal:null;
                if(signal?.aborted)return;
                let byCapture=listeners.get(listener);if(!byCapture)listeners.set(listener,byCapture=new Map());
                if(byCapture.has(capture))return;
                const state={active:true,once:typeof options==='object'&&options?.once===true};
                let abort=null,entry;
                const retire=cancel=>{
                    if(cancel)state.active=false;
                    if(byCapture.get(capture)===entry)byCapture.delete(capture);
                    if(listeners.get(listener)===byCapture&&byCapture.size===0)listeners.delete(listener);
                    if(abort){signal.removeEventListener('abort',abort);abort=null;}
                };
                const wrapped=e=>{
                    if(!state.active||signal?.aborted)return;
                    // Native once registration is consumed before invocation.
                    // A subsequently re-added callback is a separate generation;
                    // the held original event still owns this one invocation.
                    if(state.once)retire(false);
                    const active=()=>state.active&&(!signal?.aborted||state.once);
                    const done=()=>{if(state.once)state.active=false;};
                    if(selected(this,e)){
                        const bytes=new TextEncoder().encode(e.data).byteLength;
                        if(held.length>=8||heldBytes+bytes>2097152){overflow=true;release('event-or-byte-bound');}
                        else {held.push({socket:this,listener,original:e,active,done});heldBytes+=bytes;return;}
                    }
                    try{invoke(listener,this,e);}finally{done();}
                };
                entry={wrapped,state,options,retire};byCapture.set(capture,entry);
                if(signal){abort=()=>retire(true);signal.addEventListener('abort',abort,{once:true});}
                return nativeAdd.call(this,type,wrapped,options);
            }},
            removeEventListener:{configurable:true,value(type,listener,options){
                const capture=typeof options==='boolean'?options:options?.capture===true;
                const entry=type==='message'?listeners.get(listener)?.get(capture):null;
                if(entry){entry.retire(true);
                    return nativeRemove.call(this,type,entry.wrapped,options);}
                return nativeRemove.call(this,type,listener,options);
            }}
        });
        return socket;
    }});
    globalThis.WebSocket=ProxySocket;
    const api={
        arm({intent,cid,sid,old,retained,until}) {
            if(closed||armed||intent.source!==source||intent.ordinal!==ordinal
                ||![intent.operation,cid,sid,old,retained].every(x=>typeof x==='string'&&x.length>0&&x.length<=128)
                ||!Number.isFinite(until)||until<=now())throw Error('Exact original move gate identity/cutoff');
            armed={operationId:intent.operation,cid,sid,old,retained,until,target:null};
            event('armed',{operation_id:intent.operation,old,retained,until_ms:until,hold_ms:holdMs});
            watch(old);
        },
        target(id){if(armed){if(typeof id!=='string'||(armed.target&&armed.target!==id))throw Error('Original target changed');armed.target=id;event('target-prepared',{target:id});watch(id);}},
        validated({operationId,old,target}) {
            if(!armed||operationId!==armed.operationId||old!==armed.old||target!==armed.target)
                throw Error('Validated original counter root changed');
            event('both-original-sdk-receipts-validated',{operation_id:operationId,old,target});
        },
        render(kind,id){if(armed&&id===armed.target)event(kind,{target:id});},
        audio({owner,timestamp,at,index,identity}) {
            if(!armed||closed||at<events[0].at_ms||at>armed.until+500||![armed.old,armed.target].includes(owner))return;
            let row=counts.get(owner);if(!row)counts.set(owner,row={owner,frames:0,bad:0,low_energy:0,first_at_ms:at,last_at_ms:at});
            row.frames++;row.bad+=!identity.identified;row.low_energy+=identity.rms**2<=1e-6;row.last_at_ms=at;
            if((!identity.identified||row.frames===1)&&pcm.length<24)pcm.push({owner,index,original_timestamp_us:timestamp,
                callback_performance_ms:at,rms:identity.rms,identified:identity.identified});
        },
        report(){return {format:'connect/original-completion-delivery-counter/1',hold_ms:holdMs,
            original_cutoff_ms:armed?.until??null,armed:Boolean(armed),matched_original_reply_observed:groupCount>0,held_request_groups:groupCount,retained_socket_group_references:blocked.length,
            held_events_remaining:held.length,held_bytes_remaining:heldBytes,expired,overflow,callback_failure:callbackFailure,
            sockets_created:opened,sockets_closed:ended,hook_closed:closed,retained_peer_references:peers.size,lifecycle_listeners:detach.length,events:events.map(r=>({...r})),
            pcm:pcm.map(r=>({...r})),counts:[...counts.values()].map(r=>({...r})),
            original_root_cut_timestamp_proved:false,original_last_RTP_timestamp_proved:false,
            causal_attribution:false,matrix_credit:false,
            scope:'Unmodified original WSS reply delivery; signed receipt is validated only by unchanged SDK after release. Server Root cut/lastRTP require independently correlated server evidence; caller time is not server cut.'};},
        close(){if(!closed){release('fixture-cleanup');closed=true;
            if(globalThis.WebSocket===ProxySocket)globalThis.WebSocket=Native;
            if(globalThis.connectPeerTag===tag)globalThis.connectPeerTag=originalTag;
            for(const remove of detach.splice(0))try{remove();}catch{callbackFailure=true;}peers.clear();blocked.length=0;
            if(globalThis.connectOriginalCompletionGate===api)delete globalThis.connectOriginalCompletionGate;
            event('hook-closed');}return api.report();}
    };
    globalThis.connectOriginalCompletionGate=api;return api;
}
