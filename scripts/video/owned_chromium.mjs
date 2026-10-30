/** Match the canonical Rust fixture's fresh-profile Chromium launch flags. */
import fs from 'node:fs/promises';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';

export async function launchOwnedChromium(chromium,binary,profile){
    await fs.mkdir(profile,{mode:0o700});
    const child=spawn(binary,['--headless=new','--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0','--no-first-run','--no-default-browser-check',
        '--disable-background-networking','--disable-sync','--disable-breakpad','--disable-crash-reporter',
        '--autoplay-policy=no-user-gesture-required','--disable-background-timer-throttling',
        '--disable-renderer-backgrounding',...(process.platform==='linux'&&process.getuid()===0?['--no-sandbox']:[]),
        `--user-data-dir=${profile}`],{stdio:'ignore'});
    const exited=new Promise(resolve=>{
        child.once('exit',(code,signal)=>resolve({code,signal}));
        child.once('error',()=>resolve({launch_error:true}));
    });
    let browser,retiring;
    const bounded=async(promise,ms)=>{let timer;try{return await Promise.race([promise,new Promise(resolve=>{timer=setTimeout(()=>resolve(null),ms);})]);}finally{clearTimeout(timer);}};
    const close=()=>retiring??=(async()=>{
        if(browser?.isConnected())await bounded(browser.close().catch(()=>null),5000);
        let result=await bounded(exited,1000),forced=false;
        if(!result){child.kill('SIGTERM');result=await bounded(exited,5000);}
        if(!result){forced=true;child.kill('SIGKILL');result=await bounded(exited,5000);}
        return {original_process_joined:result!==null,forced_kill:forced,exit:result};
    })();
    try{
        const until=Date.now()+10000;let port;
        while(Date.now()<until){
            if(child.exitCode!==null||child.signalCode!==null)throw Error('Owned browser exited before readiness');
            try{port=Number((await fs.readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]);}
            catch(error){if(error.code!=='ENOENT')throw error;}
            if(Number.isInteger(port)&&port>0&&port<65536)break;
            await delay(20);
        }
        if(!Number.isInteger(port)||port<1||port>65535)throw Error('Owned browser readiness deadline');
        browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`,{timeout:5000});
        return {browser,close};
    }catch(error){await close();throw error;}
}
