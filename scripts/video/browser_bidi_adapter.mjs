/** Public WebDriver BiDi adapter for the pinned installed Firefox fixture.
 * It translates automation calls only; media APIs and observers stay native.
 */
import {pathToFileURL} from 'node:url';
export async function connectFirefoxBidi(modulePath, port) {
    if(!modulePath?.startsWith('/')||!Number.isInteger(port)||port<1||port>65535)
        throw Error('Explicit private Puppeteer module and bounded loopback port required');
    const {default:puppeteer}=await import(pathToFileURL(modulePath).href);
    const browser=await puppeteer.connect({protocol:'webDriverBiDi',
        browserWSEndpoint:`ws://127.0.0.1:${port}/session`,acceptInsecureCerts:false,protocolTimeout:10000});
    try {
        const version=await browser.version();
        if(version!=='firefox/158.0.1')throw Error('Installed Firefox fixture requires version158.0.1');
        return {
            version:()=>version,
            close:()=>browser.close(),
            async newContext(){
                const context=await browser.createBrowserContext();
                return {close:()=>context.close(),async newPage(){
                    const page=await context.newPage();
                    return {mouse:page.mouse,addInitScript:fn=>page.evaluateOnNewDocument(fn),
                        goto:url=>page.goto(url),evaluate:(fn,arg)=>page.evaluate(fn,arg)};
                }};
            },
        };
    } catch(error){await browser.close();throw error;}
}
