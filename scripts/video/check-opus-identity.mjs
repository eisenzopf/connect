/** Original installed browser/SDK codec characterization, not media grading. */
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {launchOwnedChromium} from './owned_chromium.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const [directory]=process.argv.slice(2),binary=process.env.CONNECT_CHROME_BINARY,module=process.env.CONNECT_PLAYWRIGHT_MODULE;
if(process.argv.length!==3||!directory||!path.isAbsolute(binary??'')||!path.isAbsolute(module??''))
    throw Error('Fresh ignored output and explicit original installed browser/framework required');
const output=path.resolve(directory);if(!output.startsWith(root+path.sep+'.local'+path.sep))throw Error('Ignored output required');
await fs.mkdir(output,{mode:0o700});
const hash=bytes=>createHash('sha256').update(bytes).digest('hex'),assets=new Map(),sources={};
// Closed recursive static asset graph. Paths/bytes are fixed before browser
// launch. No whole repository server, route traversal or dependency download.
async function asset(name){
    name=path.normalize(name);if(assets.has('/'+name))return;
    if(assets.size>=64||name.startsWith('..')||!['scripts/lib/','scripts/video/','sdk/typescript/dist/'].some(p=>name.startsWith(p)))throw Error('Closed component asset graph exceeded');
    const absolute=path.join(root,name),actual=await fs.realpath(absolute);
    if(actual!==absolute)throw Error('Original component regular path required');
    const bytes=await fs.readFile(absolute);assets.set('/'+name,bytes);sources[name]=hash(bytes);
    for(const match of bytes.toString('utf8').matchAll(/\b(?:from\s*|import\s*)["'](\.[^"']+)["']/g))
        await asset(path.posix.normalize(path.posix.join(path.posix.dirname(name),match[1])));
}
await asset('scripts/lib/browser_opus_identity_control.mjs');
for(const name of ['scripts/video/owned_chromium.mjs',path.relative(root,fileURLToPath(import.meta.url))])
    sources[name]=hash(await fs.readFile(path.join(root,name)));
sources.original_browser_binary=hash(await fs.readFile(binary));sources.original_framework_entry=hash(await fs.readFile(module));
await fs.writeFile(path.join(output,'source-inputs.json'),JSON.stringify(sources,null,2)+'\n');
let server,owner,browser,context,ownership,report,error,version,evaluation;
let evaluationOutcome,contextOutcome,serverJoined=false;
const hostUntil=performance.now()+45000;
const failure=cause=>{error??={name:cause?.name??'Error',message:String(cause?.message??cause).slice(0,160)};};
async function observedUntil(original){
    let timer;
    try{return await Promise.race([original,new Promise((_,reject)=>{
        timer=setTimeout(()=>reject(Error('Original host component deadline exhausted')),Math.max(0,hostUntil-performance.now()));
    })]);}finally{clearTimeout(timer);}
}
try{
    server=http.createServer((request,response)=>{
        if(request.method==='GET'&&request.url==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><meta charset="utf-8"><title>Original Opus identity component</title>');return;}
        if(request.method==='GET'&&assets.has(request.url)){response.setHeader('Content-Type','text/javascript');response.end(assets.get(request.url));return;}
        response.statusCode=404;response.end();
    });
    await new Promise((yes,no)=>{server.once('error',no);server.listen(0,'127.0.0.1',yes);});
    const {chromium}=await import(pathToFileURL(module).href);
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'profile'));browser=owner.browser;
    version=browser.version();context=await browser.newContext();const page=await context.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}/`,{waitUntil:'load',timeout:5000});
    evaluation=page.evaluate(async remaining=>{
        const {browserOpusIdentityControl}=await import('/scripts/lib/browser_opus_identity_control.mjs');
        return await browserOpusIdentityControl({deadlineMs:performance.now()+Math.min(30000,remaining)});
    },Math.max(0,hostUntil-performance.now()));
    report=await observedUntil(evaluation);
}catch(cause){failure(cause);}
finally{
    // Begin all original cleanup before observing it. A deadline records failure
    // only; it never supplies a synthetic context/evaluation/browser join.
    const contextWork=Promise.allSettled(context?[context.close()]:[]);
    const serverWork=server?new Promise(resolve=>server.close(cause=>{
        if(cause)failure(cause);else serverJoined=true;resolve();
    })):Promise.resolve();
    try{
        try{await observedUntil(contextWork);}catch(cause){failure(cause);}
        try{if(owner)ownership=await owner.close();}catch(cause){failure(cause);}
    }finally{
        // HTTP closure is attempted even when original browser retirement fails.
        await serverWork;
    }
    contextOutcome=await contextWork;
    if(contextOutcome.some(result=>result.status==='rejected'))failure(Error('Original context close failed'));
    if(evaluation){
        evaluationOutcome=(await Promise.allSettled([evaluation]))[0];
        if(evaluationOutcome.status==='rejected')failure(Error('Original evaluation did not complete'));
    }
}
const changed=[];
for(const [name,digest]of Object.entries(sources)){
    const absolute=name==='original_browser_binary'?binary:name==='original_framework_entry'?module:path.join(root,name);
    if(hash(await fs.readFile(absolute))!==digest)changed.push(name);
}
const joined=ownership?.original_process_joined===true&&!ownership.forced_kill&&ownership.exit?.code===0;
const summary={format:'connect/original-opus-identity-component/1',browser:version??null,os:process.platform,architecture:process.arch,
    source_manifest_sha256:hash(await fs.readFile(path.join(output,'source-inputs.json'))),sources_unchanged:changed.length===0,changed,
    ownership:ownership??null,original_browser_joined:joined,original_server_joined:serverJoined,
    original_contexts_joined:contextOutcome?.every(row=>row.status==='fulfilled')??false,
    original_evaluation_joined:Boolean(evaluationOutcome),error:error??null,component:report??null,
    component_complete:!error&&joined&&serverJoined&&changed.length===0&&report?.complete===true,
    quality_qualified:false,matrix_qualified:false,
    scope:'Original SDK encoder/decoder and unchanged classifier characterization only. Ambiguous output remains a real quality failure. No Connect transport, PLC, RTC or actual historical-cell attribution.'};
await fs.writeFile(path.join(output,'summary.json'),JSON.stringify(summary,null,2)+'\n');
console.log(JSON.stringify(summary));process.exitCode=summary.component_complete?0:1;
