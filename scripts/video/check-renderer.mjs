/** Independent canvas/capture/video calibration. No RTC, server or account. */
import fs from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import http from 'node:http';
import {markerFrames} from './marker_calibration.mjs';
import {markerCells} from './source_identity.mjs';
import {launchOwnedChromium} from './owned_chromium.mjs';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const [directory,mode='calibration']=process.argv.slice(2);
if(![3,4].includes(process.argv.length)||!directory||!['calibration','quality','rtc'].includes(mode))
    throw Error('One fresh .local output directory and optional quality mode required');
// Retain the original calibration. The separate quality control matches the
// actual fixture's 60-second window, native canvas capture and 128x72 observer.
const seconds=mode==='calibration'?8:60,pixelsPerSide=mode==='calibration'?[8,8]:[128,72];
const output=path.resolve(directory);let binary=process.env.CONNECT_CHROME_BINARY;
const modulePath=process.env.CONNECT_PLAYWRIGHT_MODULE;
if(!output.startsWith(root+path.sep+'.local'+path.sep)||!path.isAbsolute(modulePath??''))
    throw Error('Explicit installed browser, framework module and ignored output required');
await fs.mkdir(path.dirname(output),{recursive:true});await fs.mkdir(output,{mode:0o700});
if(process.env.CONNECT_PINNED_CHROMIUM_CACHE){
    // The pinned image has different executable subdirectories per CPU. Use
    // its ONE installed Chromium, never a download or alternate browser.
    if(binary||process.platform!=='linux'||process.arch!=='x64'||process.env.CONNECT_PINNED_CHROMIUM_CACHE!=='/ms-playwright')
        throw Error('Pinned image discovery requires the explicit Linux x86 cache');
    const cache='/ms-playwright',entries=(await fs.readdir(cache)).filter(name=>/^chromium-\d+$/.test(name)).sort(),candidates=[];
    for(const entry of entries)for(const folder of ['chrome-linux64','chrome-linux']){
        const file=path.join(cache,entry,folder,'chrome');
        try{if((await fs.stat(file)).isFile())candidates.push(file);}catch(error){if(error.code!=='ENOENT')throw error;}
    }
    await fs.writeFile(path.join(output,'image-browser.json'),JSON.stringify({cache,architecture:process.arch,entries,candidates},null,2)+'\n');
    if(candidates.length!==1)throw Error('Pinned image must contain exactly one Chromium executable');
    [binary]=candidates;
}
if(!path.isAbsolute(binary??''))throw Error('Explicit installed browser required');
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
let browser,owner,ownership,assets;const contexts=[];let rows=[],failure,joined=false,version;
try{
    if(mode==='rtc'){
        assets=http.createServer((_request,response)=>response.end('<!doctype html><body></body>'));
        await new Promise((resolve,reject)=>{assets.once('error',reject);assets.listen(0,'127.0.0.1',resolve);});
    }
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'profile'));browser=owner.browser;
    version=browser.version();
    const pages=await Promise.all([0,1].map(async index=>{
        const context=await browser.newContext();contexts.push(context);const page=await context.newPage();
        if(assets){
            const origin=`http://127.0.0.1:${assets.address().port}`;
            // Only this owned synthetic origin can expose loopback ICE in the
            // network-disabled container. No physical capture is requested.
            await context.grantPermissions(['microphone','camera'],{origin});
            await page.goto(origin);
        }else await page.setContent('<!doctype html><body></body>');
        await page.evaluate(async({index,cells,pulses,mode,pixelsPerSide})=>{
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
            draw();const timer=setInterval(draw,1000/30),stream=mode==='calibration'?canvas.captureStream(30):canvas.captureStream();
            let received=stream,sender,receiver;
            if(mode==='rtc'){
                sender=new RTCPeerConnection({iceServers:[]});receiver=new RTCPeerConnection({iceServers:[]});
                received=new MediaStream();receiver.ontrack=event=>received.addTrack(event.track);
                const tx=sender.addTransceiver(stream.getVideoTracks()[0],{direction:'sendrecv',streams:[stream],
                    sendEncodings:[{maxFramerate:30,maxBitrate:1_000_000,scaleResolutionDownBy:1}]});
                const vp8=RTCRtpReceiver.getCapabilities('video').codecs.filter(codec=>codec.mimeType.toLowerCase()==='video/vp8');
                if(!vp8.length)throw Error('Native VP8 unavailable');tx.setCodecPreferences(vp8);
                tx.receiver.jitterBufferTarget=80;
                if(tx.receiver.jitterBufferTarget!==80)throw Error('Native receiver target unavailable');
                const gathered=async peer=>{
                    const until=performance.now()+5000;
                    while(peer.iceGatheringState!=='complete'){
                        if(performance.now()>=until)throw Error('Owned RTC gathering deadline');
                        await new Promise(resolve=>setTimeout(resolve,10));
                    }
                };
                await sender.setLocalDescription(await sender.createOffer());await gathered(sender);
                await receiver.setRemoteDescription(sender.localDescription);
                for(const {receiver:rx} of receiver.getTransceivers()){
                    rx.jitterBufferTarget=80;
                    if(rx.jitterBufferTarget!==80)throw Error('Native receiver target unavailable');
                }
                await receiver.setLocalDescription(await receiver.createAnswer());await gathered(receiver);
                await sender.setRemoteDescription(receiver.localDescription);
            }
            const video=document.createElement('video');video.muted=true;video.autoplay=true;
            video.width=640;video.height=360;video.style.cssText='position:fixed;inset:0;width:640px;height:360px;z-index:10000';
            document.body.append(video);video.srcObject=received;
            let playTimer;
            try{await Promise.race([video.play(),new Promise((_,reject)=>{
                playTimer=setTimeout(()=>reject(Error('Owned video playback deadline')),10000);
            })]);}finally{clearTimeout(playTimer);}
            const [width,height]=pixelsPerSide;
            const sample=document.createElement('canvas');sample.width=width;sample.height=height;
            const pixels=sample.getContext('2d',{willReadFrequently:true});pixels.imageSmoothingEnabled=false;
            let callbacks=0,changing=0,raf=0,last,closed=false;
            const decoded=()=>{
                if(closed)return;callbacks++;pixels.drawImage(video,0,0,width,height);
                const bytes=pixels.getImageData(0,0,width,height).data;
                if(last!==undefined&&bytes.some((value,index)=>value!==last[index]))changing++;
                last=bytes;video.requestVideoFrameCallback(decoded);
            };
            const animation=()=>{if(!closed){raf++;requestAnimationFrame(animation);}};
            video.requestVideoFrameCallback(decoded);requestAnimationFrame(animation);
            window.observer={snapshot:async()=>{
                let decoded;
                if(receiver)(await receiver.getStats()).forEach(stat=>{
                    if(stat.type==='inbound-rtp'&&stat.kind==='video'&&Number.isFinite(stat.framesDecoded))decoded=stat.framesDecoded;
                });
                return {index,draws,callbacks,changing,raf,decoded,time:performance.now(),visibility:document.visibilityState};
            },close:()=>{closed=true;clearInterval(timer);sender?.close();receiver?.close();
                for(const track of [...stream.getTracks(),...received.getTracks()])track.stop();video.srcObject=null;}};
        },{index,cells:markerCells(index),pulses:markerFrames().map(frame=>frame/30),mode,pixelsPerSide});
        return page;
    }));
    const initial=await Promise.all(pages.map(page=>page.evaluate(()=>window.observer.snapshot())));
    await new Promise(resolve=>setTimeout(resolve,seconds*1000));
    rows=await Promise.all(pages.map(async(page,index)=>{
        const end=await page.evaluate(()=>window.observer.snapshot()),start=initial[index],seconds=(end.time-start.time)/1000;
        await page.evaluate(()=>window.observer.close());
        return {index,seconds,draw_fps:(end.draws-start.draws)/seconds,callback_fps:(end.callbacks-start.callbacks)/seconds,
            changing_fps:(end.changing-start.changing)/seconds,raf_fps:(end.raf-start.raf)/seconds,
            ...(mode==='rtc'?{decoded_fps:(end.decoded-start.decoded)/seconds}:{}),
            visibility:end.visibility,passed:end.visibility==='visible'&&(end.callbacks-start.callbacks)/seconds>=24
                &&(end.changing-start.changing)/seconds>=24&&(mode!=='rtc'||(end.decoded-start.decoded)/seconds>=24)};
    }));
}catch(error){
    const reasons=[['Owned RTC gathering deadline','rtc-gathering'],['Native VP8 unavailable','vp8-unavailable'],
        ['Native receiver target unavailable','receiver-target'],['Owned video playback deadline','video-playback']];
    failure={name:error?.name??'Error',reason:reasons.find(([message])=>String(error?.message).includes(message))?.[1]??'other'};
}
finally{
    await Promise.allSettled(contexts.map(context=>context.close()));
    if(owner){ownership=await owner.close();joined=ownership.original_process_joined&&!ownership.forced_kill&&ownership.exit?.code===0;}
    if(assets)await new Promise(resolve=>assets.close(resolve));
}
const after=await inputs(),changed=Object.keys(before).filter(file=>before[file]!==after[file]);
const report={scope:mode==='rtc'?'Two owned browser-native VP8-only RTC loopback controls with 80 ms receiver targets. No Connect, Core, SDK worker, audio, remote host or account qualification.':'Two owned local canvas/capture/video controls. No RTC, transport, Core, audio, remote host or account qualification.',
    mode,requested_seconds:seconds,motion_sample_pixels:pixelsPerSide,
    os:process.platform,architecture:process.arch,node:process.version,browser:version,source_count:Object.keys(before).length,
    source_manifest_sha256:await hash(path.join(output,'source-inputs.json')),sources_unchanged:changed.length===0,changed_inputs:changed,
    minimum_callback_and_changing_fps:24,rows,original_browser_joined:joined,ownership,error:failure??null,
    passed:!failure&&joined&&changed.length===0&&rows.length===2&&rows.every(row=>row.passed)};
await fs.writeFile(path.join(output,'summary.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
process.exitCode=report.passed?0:1;
