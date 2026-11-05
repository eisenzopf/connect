/** Native, pinned browser owners for the complete matrix. No TLS bypass. */
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import {spawn} from 'node:child_process';
import {pathToFileURL} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';
import {launchOwnedChromium} from './owned_chromium.mjs';

export async function launchMatrixBrowser(spec,profile,initializeProfile){
    if(spec.browserName==='chrome'){
        const {chromium}=await import(pathToFileURL(spec.playwrightModule).href);
        return launchOwnedChromium(chromium,spec.browserBinary,profile,process.env,initializeProfile);
    }
    if(spec.browserName!=='firefox')throw Error('Unknown matrix browser');
    const {default:puppeteer}=await import(pathToFileURL(spec.puppeteerModule).href);
    if(initializeProfile)await initializeProfile(profile);else await fs.mkdir(profile,{mode:0o700});
    await fs.writeFile(path.join(profile,'user.js'),[
        'user_pref("media.autoplay.default", 0);',
        'user_pref("media.autoplay.blocking_policy", 0);',
        'user_pref("browser.shell.checkDefaultBrowser", false);',
        'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
        'user_pref("toolkit.telemetry.enabled", false);',
    ].join('\n')+'\n',{mode:0o600});
    const reservation=net.createServer();
    await new Promise((yes,no)=>{reservation.once('error',no);reservation.listen(0,'127.0.0.1',yes);});
    const port=reservation.address().port;
    await new Promise(resolve=>reservation.close(resolve));
    const child=spawn(spec.browserBinary,['--headless','--no-remote','--profile',profile,'--remote-debugging-port',String(port)],
        {env:process.env,stdio:'ignore'});
    const exited=new Promise(resolve=>{child.once('exit',(code,signal)=>resolve({code,signal}));
        child.once('error',()=>resolve({launch_error:true}));});
    const bounded=async(promise,ms)=>{let timer;try{return await Promise.race([promise,
        new Promise(resolve=>{timer=setTimeout(()=>resolve(null),ms);})]);}finally{clearTimeout(timer);}};
    let browser,closing;
    const close=()=>closing??=(async()=>{
        if(browser)await bounded(browser.close().catch(()=>null),5000);
        let status=await bounded(exited,1000),forced=false;
        if(!status){child.kill('SIGTERM');status=await bounded(exited,5000);}
        if(!status){forced=true;child.kill('SIGKILL');status=await bounded(exited,5000);}
        return {original_process_joined:status!==null,forced_kill:forced,exit:status};
    })();
    try{
        const cutoff=performance.now()+10000;
        while(!browser){
            if(child.exitCode!==null||child.signalCode!==null)throw Error('Original Firefox exited before readiness');
            try{browser=await puppeteer.connect({protocol:'webDriverBiDi',
                browserWSEndpoint:`ws://127.0.0.1:${port}/session`,acceptInsecureCerts:false,protocolTimeout:10000});}
            catch(error){if(performance.now()>=cutoff)throw error;await delay(50);}
        }
        if(await browser.version()!=='firefox/158.0.1')throw Error('Frozen Firefox version differs');
        const facade={async newPage(){
            const page=await browser.newPage();
            return {goto:(url,options)=>page.goto(url,options),evaluate:(fn,arg)=>page.evaluate(fn,arg),
                addInitScript:fn=>page.evaluateOnNewDocument(fn),
                exposeFunction:(name,fn)=>page.exposeFunction(name,fn),
                on:(event,fn)=>page.on(event,fn)};
        }};
        return {browser:facade,close,port};
    }catch(error){await close();throw error;}
}
