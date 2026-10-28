/** Owned Linux Chromium fixture process. No existing browser/account/profile. */
import fs from "node:fs/promises";
import {spawn,spawnSync} from "node:child_process";
import {setTimeout as delay} from "node:timers/promises";
import {pathToFileURL} from "node:url";
import path from "node:path";
import process from "node:process";

const [directory,mode]=process.argv.slice(2);
if(!directory||!["untrusted","media","clock"].includes(mode)||process.argv.length!==4)throw Error("Expected owned fixture directory and mode");
const spec=JSON.parse(await fs.readFile(path.join(directory,"input.json"),"utf8"));
if(spec.designatedRemoteHost!=="host.lima.internal")throw Error("Owned Linux gateway required");
const binary="/ms-playwright/chromium-1234/chrome-linux/chrome";
const version=spawnSync(binary,["--version"],{encoding:"utf8"});
if(version.status!==0||version.stdout.trim()!=="Chromium 151.0.7922.34")throw Error("Pinned image browser changed");
if(mode!=="untrusted"){
    const imported=spawnSync("python3",[path.join(spec.root,"scripts/video/linux_browser_trust.py"),path.join(directory,"root.der")],{encoding:"utf8"});
    await fs.writeFile(path.join(directory,"trust-import.json"),imported.stdout,{mode:0o600});
    await fs.writeFile(path.join(directory,"trust-import.log"),imported.stderr,{mode:0o600});
    if(imported.status!==0)throw Error("Normal container CA import failed");
}
const profile="/tmp/connect-browser-profile";
async function retire(child,joined){
    if(child.exitCode===null&&child.signalCode===null)child.kill("SIGTERM");
    let timer;
    let result=await Promise.race([joined,new Promise(resolve=>{timer=setTimeout(()=>resolve(null),5000);})]);
    clearTimeout(timer);
    const forced=!result;
    if(forced){child.kill("SIGKILL");result=await joined;}
    return {original_joined:true,forced_kill:forced,exit:result};
}
let pulse,pulseJoined,pulseLog="";
if(mode!=="untrusted"){
    await fs.mkdir("/tmp/connect-audio",{mode:0o700});
    await fs.writeFile("/tmp/connect-audio/fixture.pa",[
        "load-module module-native-protocol-unix socket=/tmp/connect-audio/native",
        "load-module module-null-sink sink_name=connect_fixture rate=48000 channels=2",
        "set-default-sink connect_fixture",""].join("\n"),{mode:0o600});
    pulse=spawn("pulseaudio",["-n","--daemonize=no","--exit-idle-time=-1","--disable-shm=yes","--use-pid-file=no",
        "--log-target=stderr","--log-level=debug","--file=/tmp/connect-audio/fixture.pa"],{stdio:["ignore","ignore","pipe"]});
    pulse.stderr.on("data",data=>{if(pulseLog.length<200000)pulseLog+=data;});
    pulseJoined=new Promise(resolve=>pulse.once("exit",(code,signal)=>resolve({code,signal})));
    const until=Date.now()+5000;
    for(;;){
        if(await fs.stat("/tmp/connect-audio/native").then(()=>true,()=>false))break;
        if(pulse.exitCode!==null||pulse.signalCode!==null||Date.now()>until){
            await retire(pulse,pulseJoined);
            await fs.writeFile(path.join(directory,"pulse.log"),pulseLog,{mode:0o600});
            throw Error("Paced software audio server did not become ready");
        }
        await delay(25);
    }
}
const args=["--headless=new","--no-sandbox","--remote-debugging-address=127.0.0.1","--remote-debugging-port=0",
    `--user-data-dir=${profile}`,"--no-first-run","--no-default-browser-check","--disable-background-networking",
    "--disable-component-update","--disable-sync","--disable-breakpad","--disable-crash-reporter",
    "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE host.lima.internal",
    "--autoplay-policy=no-user-gesture-required","--disable-background-timer-throttling","--disable-renderer-backgrounding","about:blank"];
const chrome=spawn(binary,args,{stdio:["ignore","ignore","pipe"],
    env:{...process.env,...(pulse?{PULSE_SERVER:"unix:/tmp/connect-audio/native"}:{})}});
let log="";chrome.stderr.on("data",data=>{if(log.length<1000000)log+=data;});
const joined=new Promise(resolve=>chrome.once("exit",(code,signal)=>resolve({code,signal})));
let passed=false,calibrationBrowser,sourceClock;
try{
    const until=Date.now()+15000;let port;
    while(!port){
        if(chrome.exitCode!==null||chrome.signalCode!==null)throw Error("Browser exited before readiness");
        try{port=Number((await fs.readFile(path.join(profile,"DevToolsActivePort"),"utf8")).split("\n")[0]);}catch{}
        if(!port&&Date.now()>until)throw Error("Browser readiness deadline");
        if(!port)await delay(50);
    }
    if(mode==="untrusted"){
        const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
        const browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
        try{
            const page=await browser.newPage();
            let rejected=false;
            try{await page.goto(spec.participants[0].url.replace(/^wss:/,"https:"),{timeout:10000});}
            catch(error){rejected=String(error).includes("net::ERR_CERT_AUTHORITY_INVALID");}
            if(!rejected)throw Error("Unconfigured CA was not specifically rejected");
        }finally{await browser.close();}
        console.log(JSON.stringify({mode,os:process.platform,architecture:process.arch,browser:version.stdout.trim(),
            unconfigured_ca_refused:true,normal_certificate_verification:true}));
    }else{
        const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
        calibrationBrowser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
        const context=await calibrationBrowser.newContext(),page=await context.newPage();
        try{
            sourceClock=await page.evaluate(async()=>{
                const audio=new AudioContext(),oscillator=audio.createOscillator(),gain=audio.createGain();
                gain.gain.value=.01;oscillator.connect(gain);gain.connect(audio.destination);oscillator.start();await audio.resume();
                try{
                    const settle=performance.now();
                    while(audio.currentTime<.25&&performance.now()-settle<5000)await new Promise(resolve=>setTimeout(resolve,25));
                    const wall=performance.now(),clock=audio.currentTime;
                    await new Promise(resolve=>setTimeout(resolve,1200));
                    const elapsed=(performance.now()-wall)/1000,seconds=audio.currentTime-clock,ratio=seconds/elapsed;
                    return {startup_audio_seconds:clock,wall_seconds:elapsed,audio_seconds:seconds,ratio,passed:clock>=.25&&ratio>=.8&&ratio<=1.2};
                }finally{oscillator.stop();await audio.close();}
            });
        }finally{await context.close();}
        await fs.writeFile(path.join(directory,"source-clock.json"),JSON.stringify(sourceClock),{mode:0o600});
        if(!sourceClock.passed)throw Error("Software audio clock is not paced to wall time");
        if(mode==="clock")console.log(JSON.stringify({mode,source_clock:sourceClock,os:process.platform,architecture:process.arch}));
        else{
        spec.debugPort=port;
        const child=spawn(process.execPath,[path.join(spec.root,"scripts/video/browser-qualify.mjs")],{stdio:["pipe","inherit","inherit"]});
        child.stdin.end(JSON.stringify(spec));
        const status=await new Promise(resolve=>child.once("exit",(code,signal)=>resolve({code,signal})));
        if(status.code!==0||status.signal)throw Error("Remote browser media rehearsal failed");
        }
    }
    passed=true;
}finally{
    await calibrationBrowser?.close().catch(()=>{});
    const browserOwnership=await retire(chrome,joined);
    const audioOwnership=pulse?await retire(pulse,pulseJoined):undefined;
    if(pulse)await fs.writeFile(path.join(directory,"pulse.log"),pulseLog,{mode:0o600});
    await fs.writeFile(path.join(directory,`browser-${mode}.log`),log,{mode:0o600});
    await fs.writeFile(path.join(directory,`browser-${mode}-ownership.json`),JSON.stringify({passed:passed&&!browserOwnership.forced_kill&&!audioOwnership?.forced_kill,
        original_browser_joined:true,forced_kill:browserOwnership.forced_kill,exit:browserOwnership.exit,audio_server:audioOwnership,
        source_clock:sourceClock,os:process.platform,architecture:process.arch,browser:version.stdout.trim()}),{mode:0o600});
    if(browserOwnership.forced_kill||audioOwnership?.forced_kill)throw Error("Owned browser/audio server failed to join after TERM");
}
