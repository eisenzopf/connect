/** Independent canvas/capture/video calibration. No RTC, server or account. */
import fs from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {markerFrames} from './marker_calibration.mjs';
import {markerCells} from './source_identity.mjs';
import {launchOwnedChromium} from './owned_chromium.mjs';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const [directory]=process.argv.slice(2);
if(process.argv.length!==3||!directory)throw Error('One fresh .local output directory required');
const output=path.resolve(directory),binary=process.env.CONNECT_CHROME_BINARY;
const modulePath=process.env.CONNECT_PLAYWRIGHT_MODULE;
if(!output.startsWith(root+path.sep+'.local'+path.sep)||!path.isAbsolute(binary??'')||!path.isAbsolute(modulePath??''))
    throw Error('Explicit installed browser, framework module and ignored output required');
await fs.mkdir(path.dirname(output),{recursive:true});await fs.mkdir(output,{mode:0o700});
const hash=async file=>{const h=createHash('sha256');for await(const bytes of createReadStream(file))h.update(bytes);return h.digest('hex');};
const files=[fileURLToPath(import.meta.url),path.join(root,'scripts/video/marker_calibration.mjs'),
    path.join(root,'scripts/video/source_identity.mjs'),binary,
    path.join(root,'scripts/video/observer-tools/package.json'),path.join(root,'scripts/video/observer-tools/package-lock.json'),
    path.join(root,'scripts/video/owned_chromium.mjs'),path.join(root,'.github/workflows/gate6-browser-observer.yml')];
async function frameworkFiles(directory){
    for(const entry of await fs.readdir(directory,{withFileTypes:true})){
        const file=path.join(directory,entry.name);
        if(entry.isDirectory())await frameworkFiles(file);
        else if(entry.isFile()&&/\.(?:js|mjs|json)$/.test(entry.name))files.push(file);
    }
}
await frameworkFiles(path.dirname(modulePath));
await frameworkFiles(path.join(path.dirname(path.dirname(modulePath)),'playwright-core'));
if(process.env.CONNECT_CHROME_FRAMEWORK)files.push(process.env.CONNECT_CHROME_FRAMEWORK);
const inputs=async()=>{const result={};for(const file of [...new Set(files)].sort())result[file]=await hash(file);return result;};
const before=await inputs();await fs.writeFile(path.join(output,'source-inputs.json'),JSON.stringify(before,null,2)+'\n');
const {chromium}=await import(pathToFileURL(modulePath).href);
let browser,owner,ownership;const contexts=[];let rows=[],failure,joined=false,version;
try{
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'profile'));browser=owner.browser;
    version=browser.version();
    const pages=await Promise.all([0,1].map(async index=>{
        const context=await browser.newContext();contexts.push(context);const page=await context.newPage();
        await page.setContent('<!doctype html><body></body>');
        await page.evaluate(async({index,cells,pulses})=>{
            const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
            const paint=canvas.getContext('2d');let n=0,ordinal=0,draws=0;
            const began=performance.now();
            const draw=()=>{
                const elapsed=(performance.now()-began)/1000;
                while(ordinal+1<pulses.length&&pulses[ordinal+1]<=elapsed)ordinal++;
                paint.fillStyle=`hsl(${n%360} 90% 40%)`;paint.fillRect(0,0,640,360);
                paint.fillStyle='white';paint.fillRect((n*7)%580,70,60,150);n++;draws++;
                cells.forEach((value,i)=>{paint.fillStyle=`rgb(${value},${value},${value})`;
                    paint.fillRect((i%2)*80,Math.floor(i/2)*45,80,45);});
                if(elapsed>=pulses[ordinal]&&elapsed<pulses[ordinal]+.1){paint.fillStyle='white';paint.fillRect(0,0,640,360);}
            };
            draw();const timer=setInterval(draw,1000/30),stream=canvas.captureStream(30);
            const video=document.createElement('video');video.muted=true;video.autoplay=true;
            video.width=640;video.height=360;document.body.append(video);video.srcObject=stream;await video.play();
            const sample=document.createElement('canvas');sample.width=8;sample.height=8;
            const pixels=sample.getContext('2d',{willReadFrequently:true});pixels.imageSmoothingEnabled=false;
            let callbacks=0,changing=0,raf=0,last,closed=false;
            const decoded=()=>{
                if(closed)return;callbacks++;pixels.drawImage(video,0,0,8,8);
                const key=Array.from(pixels.getImageData(0,0,8,8).data).join(',');
                if(last!==undefined&&key!==last)changing++;last=key;video.requestVideoFrameCallback(decoded);
            };
            const animation=()=>{if(!closed){raf++;requestAnimationFrame(animation);}};
            video.requestVideoFrameCallback(decoded);requestAnimationFrame(animation);
            window.observer={snapshot:()=>({index,draws,callbacks,changing,raf,time:performance.now(),visibility:document.visibilityState}),
                close:()=>{closed=true;clearInterval(timer);for(const track of stream.getTracks())track.stop();video.srcObject=null;}};
        },{index,cells:markerCells(index),pulses:markerFrames().map(frame=>frame/30)});
        return page;
    }));
    const initial=await Promise.all(pages.map(page=>page.evaluate(()=>window.observer.snapshot())));
    await new Promise(resolve=>setTimeout(resolve,8000));
    rows=await Promise.all(pages.map(async(page,index)=>{
        const end=await page.evaluate(()=>window.observer.snapshot()),start=initial[index],seconds=(end.time-start.time)/1000;
        await page.evaluate(()=>window.observer.close());
        return {index,seconds,draw_fps:(end.draws-start.draws)/seconds,callback_fps:(end.callbacks-start.callbacks)/seconds,
            changing_fps:(end.changing-start.changing)/seconds,raf_fps:(end.raf-start.raf)/seconds,
            visibility:end.visibility,passed:end.visibility==='visible'&&(end.callbacks-start.callbacks)/seconds>=24&&(end.changing-start.changing)/seconds>=24};
    }));
}catch(error){failure={name:error?.name??'Error'};}
finally{
    await Promise.allSettled(contexts.map(context=>context.close()));
    if(owner){ownership=await owner.close();joined=ownership.original_process_joined&&!ownership.forced_kill&&ownership.exit?.code===0;}
}
const after=await inputs(),changed=Object.keys(before).filter(file=>before[file]!==after[file]);
const report={scope:'Two owned local canvas/capture/video observers for8seconds. Same synthetic motion/identity/cadence and independent8x8pixel comparison as browser quality fixture. No RTC, transport, Core, audio, remote host or account qualification.',
    os:process.platform,architecture:process.arch,node:process.version,browser:version,source_count:Object.keys(before).length,
    source_manifest_sha256:await hash(path.join(output,'source-inputs.json')),sources_unchanged:changed.length===0,changed_inputs:changed,
    minimum_callback_and_changing_fps:24,rows,original_browser_joined:joined,ownership,error:failure??null,
    passed:!failure&&joined&&changed.length===0&&rows.length===2&&rows.every(row=>row.passed)};
await fs.writeFile(path.join(output,'summary.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
process.exitCode=report.passed?0:1;
