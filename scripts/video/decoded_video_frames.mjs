/** Observe borrowed native decoded frames synchronously, without a compositor
 * callback clock. Own only a cloned track and at most two queued frames. */
export function observeDecodedVideoFrames(track, observe) {
    if (typeof globalThis.MediaStreamTrackProcessor !== "function")
        throw new TypeError("Native decoded-frame observation unavailable");
    const owned = track.clone();
    let reader;
    try {
        reader = new globalThis.MediaStreamTrackProcessor({track:owned,maxBufferSize:2}).readable.getReader();
    } catch (error) {
        owned.stop();
        throw error;
    }
    let stopped=false,joined=false,frames=0,failed=false,closing;
    const task=(async()=>{
        try {
            for (;;) {
                const {value,done}=await reader.read();
                if(done)break;
                try {
                    if(stopped)break;
                    observe(value);
                    frames=Math.min(Number.MAX_SAFE_INTEGER,frames+1);
                } finally { value.close(); }
            }
        } catch { failed=true; }
        finally { reader.releaseLock();owned.stop();joined=true; }
    })();
    return {
        snapshot:()=>({frames,failed,joined}),
        close:()=>closing??=(async()=>{
            stopped=true;
            if(!joined)await reader.cancel().catch(()=>{failed=true;});
            await task;
            return {frames,failed,joined};
        })(),
    };
}
