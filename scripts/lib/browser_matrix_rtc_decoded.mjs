import {observeDecodedVideoFrames} from '../video/decoded_video_frames.mjs';
const check=(ok,message)=>{if(!ok)throw Error(message);};
export const RTC_DECODED_VIDEO_PROFILE='native-decoded-frames/1';

/** Fixture-only decoded-pixel owners. No source, sender, codec or authority
 * changes. Each clone belongs to one exact original received RTC track. */
export function rtcDecodedVideoOwners({observe,invalid}) {
    const owners=new Map();let closed=false,closing,lastReport;
    const fail=(entry,message)=>{entry.failed=true;invalid(entry.connection,message);};
    const closeEntry=entry=>entry.closing??=(async()=>{
        const result=await entry.reader.close();
        entry.retirement=result;
        if(!result.joined||result.failed)fail(entry,'Original native decoded-video reader did not retire cleanly');
        return result;
    })();
    return {
        begin(rtc,input){
            check(!closed&&owners.size<10,'Bounded original native decoded-video owners');
            const connection=rtc?.input?.connectionId;
            check(typeof connection==='string'&&connection.length>0&&!owners.has(connection),
                'One original native decoded-video owner per Connection');
            const streams=Object.keys(input?.video??{});
            check(streams.length===1&&streams[0]===(rtc.input.receiveVideo??rtc.input.video??rtc.input.screen),
                'Exact original RTC receive Stream required');
            const track=rtc.remote?.get(streams[0]);
            check(track?.kind==='video'&&track.readyState==='live'
                &&rtc.peer?.getReceivers?.().some(receiver=>receiver.track===track),
                'Exact live original peer-owned received video track required');
            const entry={connection,stream:streams[0],failed:false,reader:null,retirement:null,closing:null};
            // Insert ownership before a borrowed output can be delivered. The
            // reused helper owns only a clone and closes every borrowed frame.
            owners.set(connection,entry);
            try{
                entry.reader=observeDecodedVideoFrames(track,frame=>{
                    if(closed||entry.closing)return;
                    check(Number.isSafeInteger(frame.timestamp),'Original native decoded-video timestamp required');
                    observe(connection,frame.timestamp,performance.now(),frame,frame.displayWidth,frame.displayHeight);
                });
            }catch(error){owners.delete(connection);throw error;}
            return entry;
        },
        bind(entry,result){
            // These are observers of the original SDK futures, not replacement
            // render owners. Old physical clones retire as their original
            // render owner retires; they do not survive until programme end.
            void Promise.resolve(result).then(handle=>{
                if(!handle?.finished||typeof handle.finished.then!=='function'
                    ||!handle.retirement||typeof handle.retirement.then!=='function'){
                    fail(entry,'Original native decoded-video render futures unavailable');
                    return closeEntry(entry);
                }
                return Promise.allSettled([handle.finished,handle.retirement]).then(results=>{
                    if(results.some(result=>result.status==='rejected'))
                        fail(entry,'Original native decoded-video render owner failed');
                    return closeEntry(entry);
                });
            },()=>{fail(entry,'Original native decoded-video render startup failed');return closeEntry(entry);})
                .catch(()=>fail(entry,'Original native decoded-video retirement failed'));
        },
        abandon(entry){const result=closeEntry(entry);void result.catch(()=>fail(entry,'Original native decoded-video retirement failed'));return result;},
        report(){
            if(closed&&lastReport)return lastReport;
            return {profile:RTC_DECODED_VIDEO_PROFILE,owner_bound:10,max_queued_frames_per_owner:2,
                implicit_processor_queue_drops:'not-exposed-by-browser; no zero-drop claim',
                owners:[...owners.values()].map(entry=>({connection:entry.connection,stream:entry.stream,
                    ...entry.reader.snapshot(),failed:entry.failed||entry.reader.snapshot().failed})),
                scope:'Actual borrowed received-track VideoFrame pixels only. No RTP/getStats/source counter becomes frame credit'};
        },
        close(){return closing??=(async()=>{
            // Take the actual original owner list once. Even one refusal cannot
            // skip another clone/read task. No timeout fabricates a join.
            closed=true;
            const entries=[...owners.values()];
            const results=await Promise.allSettled(entries.map(closeEntry));
            lastReport={profile:RTC_DECODED_VIDEO_PROFILE,owner_bound:10,max_queued_frames_per_owner:2,
                implicit_processor_queue_drops:'not-exposed-by-browser; no zero-drop claim',
                owners:entries.map(entry=>({connection:entry.connection,stream:entry.stream,
                    ...entry.reader.snapshot(),failed:entry.failed||entry.reader.snapshot().failed})),
                scope:'Actual borrowed received-track VideoFrame pixels only. No RTP/getStats/source counter becomes frame credit'};
            const joined=results.every(result=>result.status==='fulfilled'&&result.value.joined===true),
                passed=joined&&lastReport.owners.length>=6&&lastReport.owners.every(owner=>owner.frames>0&&!owner.failed);
            if(joined)owners.clear();
            return {...lastReport,joined,passed,retained_owners:owners.size};
        })();},
    };
}
