/** Owned Linux browser software clock proof before actual media cases. */
import fs from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {launchOwnedChromium} from './owned_chromium.mjs';
const [directory]=process.argv.slice(2),output=path.resolve(directory??'');
if(process.argv.length!==3||!output.startsWith(process.cwd()+path.sep+'.local'+path.sep)
    ||process.platform!=='linux'||process.arch!=='x64'||!process.env.PULSE_SERVER)
    throw Error('Owned Linux audio clock output and software server required');
await fs.mkdir(output,{mode:0o700});
const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
let owner,context,result,ownership,failure,stage='browser-launch';
try {
    owner=await launchOwnedChromium(chromium,process.env.CONNECT_GATE6_CHROME,path.join(output,'profile'));
    stage='clock-observation';
    context=await owner.browser.newContext();const page=await context.newPage();
    result=await page.evaluate(async()=>{
        const audio=new AudioContext(),tone=audio.createOscillator(),gain=audio.createGain();
        gain.gain.value=.01;tone.connect(gain);gain.connect(audio.destination);tone.start();await audio.resume();
        try {
            const until=performance.now()+5000;
            while(audio.currentTime<.25&&performance.now()<until)await new Promise(r=>setTimeout(r,25));
            const began=performance.now(),clock=audio.currentTime;
            await new Promise(r=>setTimeout(r,1200));
            const elapsed=(performance.now()-began)/1000,seconds=audio.currentTime-clock,ratio=seconds/elapsed;
            return {startup_audio_seconds:clock,wall_seconds:elapsed,audio_seconds:seconds,ratio,
                passed:clock>=.25&&ratio>=.8&&ratio<=1.2};
        } finally {tone.stop();await audio.close();}
    });
} catch(error) {
    failure={stage,error_class:['Error','TimeoutError','TargetClosedError'].includes(error?.constructor?.name)?error.constructor.name:'other'};
} finally {
    try {await context?.close();}
    catch {failure??={stage:'context-close',error_class:'other'};}
    finally {ownership=await owner?.close();}
    await fs.writeFile(path.join(output,'summary.json'),JSON.stringify({scope:'Software clock pacing only; no RTC or decoded media.',
        result,ownership,failure,passed:Boolean(!failure&&result?.passed&&ownership?.original_process_joined&&!ownership?.forced_kill)},null,2)+'\n');
}
if(!result?.passed||!ownership?.original_process_joined||ownership.forced_kill)throw Error('Owned audio clock or browser join failed');
