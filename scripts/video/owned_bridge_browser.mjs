/** Current CFT browser in one owned internal network namespace. */
import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawn,spawnSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';
import {launchOwnedChromium} from './owned_chromium.mjs';

const [directory]=process.argv.slice(2),output=path.resolve(directory??'');
if(process.argv.length!==3||process.platform!=='linux'||process.arch!=='x64'
    ||process.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER!=='1'||!output.startsWith(process.cwd()+'/.local/'))
    throw Error('Owned Linux x86 bridge fixture required');
const spec=JSON.parse(await fs.readFile(path.join(output,'input.json'),'utf8'));
if(spec.designatedRemoteHost!=='connect.fixture.internal'||!spec.ownedContainerBridge)throw Error('Owned bridge spec required');
const binary=process.env.CONNECT_GATE6_CHROME;
const version=spawnSync(binary,['--version'],{encoding:'utf8'});
if(version.status!==0||version.stdout.trim()!=='Google Chrome for Testing 156.0.8078.4')throw Error('Current pinned CFT required');
const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
const summary={scope:'Owned internal Docker bridge, Linux x86 CFT156 normal CA and decoded clean60 media. No WAN, provider or full matrix qualification.',
    browser:version.stdout.trim(),network_namespace_sha256:createHash('sha256').update(await fs.readlink('/proc/self/ns/net')).digest('hex'),passed:false};
let stage='untrusted-ca',owner;
try{
    const httpsUrl=spec.participants[0].url.replace(/^wss:/,'https:');
    const untrustedHome=path.join(output,'untrusted-home');await fs.mkdir(untrustedHome,{mode:0o700});
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'untrusted-profile'),{...process.env,HOME:untrustedHome});
    let refused=false;
    try{await (await owner.browser.newPage()).goto(httpsUrl,{timeout:5000});}
    catch(error){refused=String(error.message).includes('ERR_CERT_AUTHORITY_INVALID');}
    summary.untrusted_browser=await owner.close();owner=undefined;
    if(!refused||!summary.untrusted_browser.original_process_joined||summary.untrusted_browser.forced_kill)throw Error('Unconfigured CA refusal required');
    summary.unconfigured_ca_refused=true;
    stage='normal-ca-import';
    const trustedProfile=path.join(output,'trusted-profile');
    const prepareProfile=async profile=>{
        const trust=spawnSync('python3',['scripts/video/browser_profile.py','chrome',binary,profile,path.join(output,'root.der')],{encoding:'utf8'});
        if(trust.status!==0)throw Error('Normal private Chrome profile import required');
        const value=JSON.parse(trust.stdout);
        summary.trust={normal_profile_ca_import:true,browser:value.browser,version:value.version,root_sha256:value.root_sha256};
    };
    owner=await launchOwnedChromium(chromium,binary,trustedProfile,process.env,prepareProfile);
    stage='trusted-ca';
    const response=await (await owner.browser.newPage()).goto(httpsUrl,{timeout:5000});
    if(!response)throw Error('Normal HTTPS response required');
    summary.trusted_https_status=response.status();summary.trusted_browser=await owner.close();owner=undefined;
    if(!summary.trusted_browser.original_process_joined||summary.trusted_browser.forced_kill)throw Error('Trusted browser join required');
    stage='audio-clock';
    const clock=spawn(process.execPath,['scripts/video/check-audio-clock.mjs',path.join(output,'clock')],{stdio:'inherit'});
    const clockExit=await new Promise(resolve=>clock.once('exit',(code,signal)=>resolve({code,signal})));
    summary.clock=JSON.parse(await fs.readFile(path.join(output,'clock/summary.json'),'utf8'));
    if(clockExit.code!==0||clockExit.signal||!summary.clock.passed)throw Error('Paced audio required');
    stage='decoded-media';
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'media-profile'),process.env,prepareProfile);
    spec.debugPort=owner.port;
    const child=spawn(process.execPath,['scripts/video/browser-qualify.mjs'],{stdio:['pipe','pipe','inherit']});
    let text='';child.stdout.on('data',chunk=>{text+=chunk;if(text.length>4*1024*1024)child.kill('SIGTERM');});
    child.stdin.end(JSON.stringify(spec));
    const exit=await new Promise(resolve=>child.once('exit',(code,signal)=>resolve({code,signal})));
    await fs.writeFile(path.join(output,'qualification-private.log'),text,{mode:0o600});
    summary.media=text.split('\n').filter(line=>line.startsWith('{')).map(line=>JSON.parse(line)).at(-1);
    summary.media_browser=await owner.close();owner=undefined;
    summary.passed=exit.code===0&&!exit.signal&&summary.media?.status==='passed'
        &&summary.media.owned_container_bridge?.nominated_udp_duplex===true
        &&summary.media_browser.original_process_joined&&!summary.media_browser.forced_kill;
}catch(error){summary.failure={stage,error_class:error?.name==='Error'?'Error':'other',
    certificate_error:['ERR_CERT_AUTHORITY_INVALID','ERR_CERT_COMMON_NAME_INVALID','ERR_CERT_DATE_INVALID']
        .find(code=>String(error?.message).includes(code))??null};}
finally{
    if(owner)summary.failure_browser=await owner.close();
    await fs.writeFile(path.join(output,'browser-summary.json'),JSON.stringify(summary,null,2)+'\n',{mode:0o600});
}
if(!summary.passed)process.exitCode=1;
