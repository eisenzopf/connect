/** Actual Chrome SDK codec selection/answer proof; generated SDP remains private. */
import fs from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {createHash} from 'node:crypto';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import {pathToFileURL,fileURLToPath} from 'node:url';
const [distArg,output]=process.argv.slice(2);
if(!distArg||!output)throw Error('Provide private compiled SDK dist and output path');
const outputPath=path.resolve(output);
await fs.mkdir(path.dirname(outputPath),{recursive:true,mode:0o700});
const dist=await fs.realpath(distArg);
if(!process.env.CONNECT_PLAYWRIGHT_MODULE||!process.env.CONNECT_CHROME_BINARY)
    throw Error('Set explicit CONNECT_PLAYWRIGHT_MODULE and CONNECT_CHROME_BINARY for qualification');
const playwrightEntry=await fs.realpath(process.env.CONNECT_PLAYWRIGHT_MODULE);
const chromeExecutable=await fs.realpath(process.env.CONNECT_CHROME_BINARY);
const nodeExecutable=await fs.realpath(process.execPath);
const fixture=await fs.realpath(fileURLToPath(import.meta.url));
if(outputPath.startsWith(dist+path.sep))throw Error('Write proof outside tested SDK assets');
async function digest(file){const hash=createHash('sha256');for await(const chunk of createReadStream(file))hash.update(chunk);return hash.digest('hex');}
async function inputHashes(){
    const names=[fixture,nodeExecutable,chromeExecutable,playwrightEntry];
    async function visit(folder){
        for(const entry of await fs.readdir(folder,{withFileTypes:true})){
            const candidate=path.join(folder,entry.name);
            if(entry.isSymbolicLink())throw Error('SDK asset symlinks are not qualified');
            if(entry.isDirectory())await visit(candidate);
            else if(entry.isFile())names.push(candidate);
            else throw Error('SDK assets must be ordinary files');
        }
    }
    await visit(dist);const hashes={};for(const file of names.sort())hashes[file]=await digest(file);return hashes;
}
const before=await inputHashes();
const {chromium}=await import(pathToFileURL(playwrightEntry).href);
const server=http.createServer(async(req,res)=>{
    try {
        const pathname=new URL(req.url,'http://127.0.0.1').pathname;
        if(pathname==='/'){res.writeHead(200,{'Content-Type':'text/html'});res.end('<!doctype html><title>Local SDP proof</title>');return;}
        if(!/^\/[a-z_]+\.js$/.test(pathname))throw Error('denied');
        const file=await fs.realpath(path.join(dist,pathname.slice(1)));
        if(!file.startsWith(dist+path.sep))throw Error('denied');
        res.writeHead(200,{'Content-Type':'text/javascript'});res.end(await fs.readFile(file));
    }catch {res.writeHead(404);res.end();}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let browser;
const result={scope:'Actual Chrome SDK-generated offer/answer, no server media or impairment qualification',status:'failed',selected_inputs:{dist,fixture,nodeExecutable,chromeExecutable,playwrightEntry},input_sha256:{before},tested_inputs_stable:false};
try {
    browser=await chromium.launch({headless:true,executablePath:chromeExecutable,args:['--autoplay-policy=no-user-gesture-required']});
    result.browser=browser.version();
    const page=await browser.newPage();await page.goto(`http://127.0.0.1:${server.address().port}/`);
    let evaluationTimer;
    try { result.observation=await Promise.race([page.evaluate(async()=>{
        const {startVideoTransform,stopVideoWorker}=await import('/browser_video_setup.js');
        const {validateVideoAnswer}=await import('/browser_capture.js');
        const peers=[],workers=[],streams=[],contexts=[];
        function videoSummary(sdp){
            const section=sdp.replace(/\r/g,'').split(/\n(?=m=)/).find(x=>x.startsWith('m=video '));
            if(!section)throw Error('No video section');
            const pts=section.split('\n')[0].split(/\s+/).slice(3).map(Number);
            const codecs=[...section.matchAll(/^a=rtpmap:(\d+) ([^\n]+)$/gm)].filter(x=>pts.includes(Number(x[1]))).map(x=>({pt:Number(x[1]),codec:x[2]}));
            const associations=[...section.matchAll(/^a=fmtp:(\d+) ([^\n]+)$/gm)].flatMap(x=>[...x[2].matchAll(/(?:^|;)\s*apt=(\d+)(?:;|$)/g)].map(y=>({repair:Number(x[1]),primary:Number(y[1])})));
            const fid=[...section.matchAll(/^a=ssrc-group:FID (\d+) (\d+)$/gm)].map(x=>({distinct:x[1]!==x[2]}));
            return {pts,codecs,associations,fid};
        }
        try {
            const capabilities=RTCRtpSender.getCapabilities('video').codecs.map(x=>({mimeType:x.mimeType,clockRate:x.clockRate,sdpFmtpLine:x.sdpFmtpLine}));
            for(let i=0;i<2;i++){
                const peer=new RTCPeerConnection({iceServers:[]});peers.push(peer);
                const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;canvas.getContext('2d').fillRect(0,0,640,360);
                const video=canvas.captureStream(30);streams.push(video);
                const context=new AudioContext();contexts.push(context);const destination=context.createMediaStreamDestination();context.createOscillator().connect(destination);streams.push(destination.stream);
                peer.addTrack(video.getVideoTracks()[0],video);peer.addTrack(destination.stream.getAudioTracks()[0],destination.stream);
                workers.push(startVideoTransform(peer));
            }
            const offer=await peers[0].createOffer();await peers[0].setLocalDescription(offer);
            await peers[1].setRemoteDescription(offer);const answer=await peers[1].createAnswer();
            let validationError;try{validateVideoAnswer(answer.sdp);}catch(error){validationError=error.code??error.name;}
            await peers[1].setLocalDescription(answer);await peers[0].setRemoteDescription(answer);
            return {capabilities,offer:videoSummary(offer.sdp),answer:videoSummary(answer.sdp),validationError};
        }finally {
            for(const worker of workers)stopVideoWorker(worker);
            for(const peer of peers)peer.close();
            for(const stream of streams)for(const track of stream.getTracks())track.stop();
            await Promise.allSettled(contexts.map(c=>c.close()));
        }
    }),new Promise((_,reject)=>{evaluationTimer=setTimeout(()=>reject(Error("Browser SDP proof timed out")),15000);})]);
    } finally {clearTimeout(evaluationTimer);}
    assert.equal(result.observation.validationError,undefined);
    for(const side of ['offer','answer']){
        const row=result.observation[side];const primary=row.codecs.filter(x=>x.codec.toLowerCase()==='vp8/90000');const repair=row.codecs.filter(x=>x.codec.toLowerCase()==='rtx/90000');
        assert.ok(primary.length>0&&repair.length>0,`${side} must retain VP8 and RTX`);
        assert.equal(row.codecs.length,primary.length+repair.length);
        assert.equal(row.pts.length,row.codecs.length);
        for(const codec of repair){const assoc=row.associations.filter(x=>x.repair===codec.pt);assert.equal(assoc.length,1);assert.ok(primary.some(x=>x.pt===assoc[0].primary));}
        assert.ok(row.fid.length>0&&row.fid.every(x=>x.distinct),`${side} needs separate repair SSRC`);
    }
    result.status='passed';
}catch(error){result.error={name:error.name,message:error.message};process.exitCode=1;}
finally{
    try {
        const cleanup=await Promise.allSettled([browser?.close(),new Promise(resolve=>server.close(resolve))]);
        if(cleanup.some(value=>value.status==='rejected'))throw Error('Browser fixture cleanup failed');
        result.input_sha256.after=await inputHashes();
        result.tested_inputs_stable=JSON.stringify(before)===JSON.stringify(result.input_sha256.after);
        if(!result.tested_inputs_stable)throw Error('Qualification inputs changed during execution');
    }catch(error){result.status='failed';result.error={name:error.name,message:error.message};process.exitCode=1;}
    await fs.writeFile(outputPath,JSON.stringify(result,null,2)+'\n',{mode:0o600});
}
