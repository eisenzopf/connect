import {browserMatrixRtcStages} from './browser_matrix_stages.mjs';

// Fixture-only observation of the original native sessions. The SDK does not
// expose its physical WebTransport factory/session; no stream lock is borrowed.
export function browserMatrixWebTransportStatistics(scope=globalThis) {
    const entries=[],fields=['droppedIncoming','expiredIncoming','expiredOutgoing','lostOutgoing'];
    let capturing=false,stopped=false,closing,frozenReport,omitted=0,restorationFailures=0,omittedUnavailable=0;
    const interceptionUnavailable=[];
    const unavailable=(actor,reason)=>{if(interceptionUnavailable.length<16)interceptionUnavailable.push({actor,reason});else omittedUnavailable++;};
    const numeric=value=>Number.isSafeInteger(value)&&value>=0?value:null;
    const retain=(owner,actor)=>{
        if(entries.length>=16){omitted++;return;}
        const entry={owner,physical_index:entries.length,actor,ready:'pending',closed:false};entries.push(entry);
        // Observers handle both outcomes without replacing the original promises.
        void owner.ready.then(()=>{entry.ready='fulfilled';},()=>{entry.ready='rejected';});
        void owner.closed.then(()=>{entry.closed=true;entry.owner=null;},()=>{entry.closed=true;entry.owner=null;});
    };
    const sample=async()=>Promise.all(entries.map(async entry=>{
        const row={physical_index:entry.physical_index,actor:entry.actor,ready:entry.ready,closed:entry.closed,
            stats_timestamp_ms:null,datagrams:Object.fromEntries(fields.map(field=>[field,null])),unavailable:null};
        let owner=entry.owner;
        if(!owner||entry.ready!=='fulfilled'){row.unavailable=entry.closed?'original-closed':'original-not-ready';return row;}
        try{
            if(typeof owner.getStats!=='function'){row.unavailable='getStats-unavailable';return row;}
            const value=await owner.getStats();
            row.stats_timestamp_ms=Number.isFinite(value?.timestamp)&&value.timestamp>=0?value.timestamp:null;
            for(const field of fields)row.datagrams[field]=numeric(value?.datagrams?.[field]);
            if(fields.some(field=>row.datagrams[field]===null))row.unavailable='datagram-fields-unavailable';
        }catch{row.unavailable='getStats-rejected-or-unreadable';}
        finally{owner=null;}
        return row;
    }));
    // Reuse the qualified bounded scheduler. Its internal RTC label is replaced
    // only in this diagnostic report, never interpreted as RTC or media proof.
    const scheduler=browserMatrixRtcStages(sample);
    const report=()=>{
        const value=scheduler.report();
        return {scope:'Original WebTransport session statistics; diagnostic only, no media-quality or delivery pass',
            owner_bound:16,omitted_owners:omitted,constructor_restoration_failures:restorationFailures,
            interception_unavailable:structuredClone(interceptionUnavailable),omitted_interception_unavailable:omittedUnavailable,
            sample_bound:value.sample_bound,attempts:value.attempts,pending:value.pending,
            owners:entries.map(({owner,...entry})=>({...entry,live_owner_retained:owner!==null})),
            samples:value.samples.map(({peers,...sample})=>({...sample,transports:peers})),
            failures:value.failures.map(row=>({...row,reason:'original-webtransport-statistics-unavailable'}))};
    };
    return {
        async capture(actor,work){
            if(capturing||stopped||typeof actor!=='string'||!actor.length||actor.length>64)throw Error('Original WT constructor scope');
            let descriptor,Native,wrapper,installed=false;
            capturing=true;
            try{
                try{descriptor=Object.getOwnPropertyDescriptor(scope,'WebTransport');}
                catch{unavailable(actor,'constructor-descriptor-unavailable');return await work();}
                // Reading an accessor would consume a possibly stateful original
                // getter before the SDK does. Unknown/inherited/accessor shapes
                // remain entirely unobserved; execute the original work once.
                if(!descriptor||!('value'in descriptor)){
                    unavailable(actor,'constructor-not-own-data-property');return await work();
                }
                Native=descriptor.value;
                if(typeof Native!=='function'){unavailable(actor,'constructor-unavailable');return await work();}
                wrapper=new Proxy(Native,{construct(target,args,newTarget){
                    const owner=Reflect.construct(target,args,newTarget===wrapper?target:newTarget);
                    retain(owner,actor);return owner;
                }});
                try{
                    Object.defineProperty(scope,'WebTransport',{...descriptor,value:wrapper});
                    installed=true;
                }catch{unavailable(actor,'constructor-interception-unavailable');return await work();}
                return await work();
            }finally{
                if(installed){
                    // Never overwrite a foreign concurrent writer or mask the
                    // original connect outcome with diagnostic restoration errors.
                    try{
                        const current=Object.getOwnPropertyDescriptor(scope,'WebTransport');
                        if(current&&'value'in current&&current.value===wrapper){
                            Object.defineProperty(scope,'WebTransport',descriptor);
                        }else restorationFailures++;
                    }catch{restorationFailures++;}
                }
                capturing=false;
            }
        },
        start(origin){scheduler.start(origin);},
        report(){return structuredClone(frozenReport??report());},
        close(){return closing??=(async()=>{
            stopped=true;const original=scheduler.close();
            // In-flight sample owns its own exact session until settlement.
            for(const entry of entries)entry.owner=null;
            const receipt=await original;
            frozenReport=report();
            return {...receipt,joined:receipt.joined&&restorationFailures===0&&!capturing,
                constructor_restored:restorationFailures===0&&!capturing};
        })();},
    };
}
