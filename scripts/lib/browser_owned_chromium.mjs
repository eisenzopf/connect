/** Close one already-owned private CDP Chromium and its local fixture assets.
 * A rejected Browser.close reply can mean the browser has already exited. The
 * original Rust owner independently proves its PID's natural exit; no reply or
 * exception here substitutes for that proof. No browser is discovered/launched. */
export async function closeOwnedChromium({browser,context,assets,timeoutMs=3000}) {
    if(!Number.isFinite(timeoutMs)||timeoutMs<=0||timeoutMs>3000)throw new RangeError('Bounded original cleanup');
    const cutoff=performance.now()+timeoutMs,failures=[];
    const browserClose={requested:false,acknowledged:false,error:null};
    const phases=[];
    async function attempt(phase,action,{closingBrowser=false}={}) {
        let timer;
        try {
            const task=Promise.resolve().then(action);
            await Promise.race([task,new Promise((_,reject)=>{
                timer=setTimeout(()=>{const error=Error('Original cleanup deadline');error.name='CleanupTimeoutError';reject(error);},Math.max(0,cutoff-performance.now()));
            })]);
            phases.push({phase,passed:true});return true;
        } catch(error) {
            const name=error instanceof Error?error.name:'UnknownCleanupError';
            phases.push({phase,passed:false,error:name});
            if(closingBrowser)browserClose.error={name};
            else {
                const message=error instanceof Error?error.message.replace(/^(?:browserContext\.close|browser\.close): /,''):'';
                const closedByRequest=browserClose.requested&&error instanceof Error&&
                    (error.name==='TargetClosedError'||message==='Target page, context or browser has been closed');
                if(!closedByRequest)failures.push({phase,name});
            }
            return false;
        } finally {clearTimeout(timer);}
    }
    let session;
    if(browser) {
        await attempt('browser-close-session',async()=>{session=await browser.newBrowserCDPSession();},{closingBrowser:true});
        if(session) {
            browserClose.requested=true;
            browserClose.acknowledged=await attempt('browser-close',()=>session.send('Browser.close'),{closingBrowser:true});
        }
    }
    // Every resource is attempted, even when another action rejects or consumes
    // the one shared cutoff. None starts a new timeout budget.
    if(context)await attempt('context-close',()=>context.close());
    if(browser)await attempt('browser-disconnect',()=>browser.close());
    if(assets)await attempt('asset-server-close',()=>new Promise((resolve,reject)=>assets.close(error=>error?reject(error):resolve())));
    return {browserClose,phases,failures,timeoutMs};
}
