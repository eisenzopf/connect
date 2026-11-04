/** Actual owned Chrome/browser-SDK matrix admission. Private input stays on disk. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import https from 'node:https';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';
import {fixturePrivateJwk,fixtureServerJwk} from '../lib/browser_matrix_keys.mjs';
import {launchOwnedChromium} from './owned_chromium.mjs';

const [inputPath]=process.argv.slice(2);
assert.equal(process.argv.length,3);
const root=await fs.realpath(process.cwd()),privateRoot=await fs.realpath(path.join(root,'.local'));
assert.ok((await fs.realpath(inputPath)).startsWith(privateRoot+path.sep));
assert.equal((await fs.stat(inputPath)).mode&0o077,0);
const spec=JSON.parse(await fs.readFile(inputPath,'utf8'));
assert.equal(spec.format,'connect/browser-matrix-admission-input/1');
assert.ok(['chrome-webtransport','chrome-wss-webrtc'].includes(spec.transport));
assert.ok(['EdDSA','ES256'].includes(spec.signingAlgorithm));
assert.ok(process.platform==='linux'&&process.arch==='arm64');
assert.equal(process.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER,'1');
const output=await fs.realpath(spec.output);
assert.ok(output.startsWith(privateRoot+path.sep));
const binary='/ms-playwright/chromium-1248/chrome-linux-arm64/chrome';
const version=spawnSync(binary,['--version'],{encoding:'utf8',timeout:10000});
assert.equal(version.status,0);assert.equal(version.stdout.trim(),'Google Chrome for Testing 156.0.8078.4');
const binaryFile=await fs.open(binary,'r'),header=Buffer.alloc(64);
try{assert.equal((await binaryFile.read(header,0,64,0)).bytesRead,64);}finally{await binaryFile.close();}
assert.equal(header.subarray(0,6).toString('hex'),'7f454c460201');assert.equal(header.readUInt16LE(18),183);
for(const value of [spec.origin,spec.endpoints.webtransport,spec.endpoints['wss-webrtc']]){
    const url=new URL(value);
    assert.ok(['https:','wss:'].includes(url.protocol)&&url.hostname==='127.0.0.1'&&url.port&&!url.username&&!url.password&&!url.search&&!url.hash);
}
for(const actor of Object.values(spec.actors)){
    assert.equal(actor.signing.algorithm,spec.signingAlgorithm);assert.equal(actor.server.algorithm,spec.signingAlgorithm);
    actor.signing.jwk=fixturePrivateJwk(Buffer.from(actor.signing.privatePkcs8,'base64url'),actor.signing.algorithm,
        Buffer.from(actor.signing.publicRaw,'base64url'));
    actor.server.jwk=await fixtureServerJwk(Buffer.from(actor.server.publicRaw,'base64url'),actor.server.algorithm);
    delete actor.signing.privatePkcs8;delete actor.signing.publicRaw;delete actor.server.publicRaw;
}
const {chromium}=await import(pathToFileURL('/opt/connect-browser-tools/node_modules/playwright/index.mjs').href);
const report={format:'connect/browser-matrix-admission-observation/1',source_revision:spec.sourceRevision,
    transport:spec.transport,signing_algorithm:spec.signingAlgorithm,browser:version.stdout.trim(),
    passed:false,cell_qualified:false,task_complete:false,full_media_programme_executed:false,
    scope:'Actual isolated Chrome normal-CA/public browser-SDK control and six-connection admission only. No full media/four-move/expiry/idle/fallback or complete-cell qualification.'};
let owner,server,page,journal,stage='asset-listener',events=0,serial=Promise.resolve(),diagnostics=Promise.resolve(),diagnosticBytes=0;
const diagnostic=text=>{
    const body=String(text).slice(0,8192)+'\n';diagnosticBytes+=Buffer.byteLength(body);
    if(diagnosticBytes<=1024*1024)diagnostics=diagnostics.then(()=>fs.appendFile(path.join(output,'browser-private.log'),body,{mode:0o600})).catch(()=>{});
};
const assets=path.join(root,'sdk/typescript/dist');
try{
    server=https.createServer({cert:await fs.readFile(spec.tls.certificatePem),key:await fs.readFile(spec.tls.privateKeyPem)},async(request,response)=>{
        try{
            assert.equal(request.method,'GET');
            const url=new URL(request.url,spec.origin);assert.equal(url.origin,spec.origin);assert.ok(!url.search&&!url.hash);
            let file;
            if(url.pathname==='/'){response.writeHead(200,{'Content-Type':'text/html','Cache-Control':'no-store'});response.end('<!doctype html><meta charset=utf-8><title>Owned matrix fixture</title>');return;}
            if(url.pathname==='/_matrix/admission.mjs')file=path.join(root,'scripts/lib/browser_matrix_admission.mjs');
            else{
                assert.ok(url.pathname.startsWith('/sdk/')&&url.pathname.endsWith('.js'));
                file=await fs.realpath(path.resolve(assets,'.'+url.pathname.slice(4)));
                assert.ok(file.startsWith(assets+path.sep));
            }
            response.writeHead(200,{'Content-Type':'text/javascript','Cache-Control':'no-store'});response.end(await fs.readFile(file));
        }catch{response.writeHead(404);response.end('fixture asset unavailable');}
    });
    await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(Number(new URL(spec.origin).port),'127.0.0.1',resolve);});
    stage='untrusted-ca';
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'untrusted-profile'));
    let refused=false;
    try{await (await owner.browser.newPage()).goto(spec.origin,{timeout:5000});}
    catch(error){refused=String(error.message).includes('ERR_CERT_AUTHORITY_INVALID');}
    report.untrusted_browser=await owner.close();owner=undefined;
    assert.ok(refused&&report.untrusted_browser.original_process_joined&&!report.untrusted_browser.forced_kill);
    report.unconfigured_ca_refused=true;
    stage='normal-ca-import';
    owner=await launchOwnedChromium(chromium,binary,path.join(output,'trusted-profile'),process.env,async profile=>{
        const imported=spawnSync('python3',['scripts/video/browser_profile.py','chrome',binary,profile,spec.tls.rootDer],
            {encoding:'utf8',timeout:10000});
        await fs.writeFile(path.join(output,'profile-import.log'),imported.stdout+imported.stderr,{mode:0o600});
        assert.equal(imported.status,0);const result=JSON.parse(imported.stdout);
        report.trust={normal_profile_ca_import:true,root_sha256:result.root_sha256,browser_version:result.version};
    });
    stage='current-browser-authentication';
    page=await owner.browser.newPage();
    page.on('console',message=>diagnostic(message.text()));
    page.on('pageerror',error=>diagnostic(error.stack??error));
    await page.goto(spec.origin,{timeout:5000});
    journal=await fs.open(path.join(output,'original-operations.jsonl'),'wx',0o600);
    await page.exposeFunction('matrixJournal',event=>{
        serial=serial.then(async()=>{
            const body=JSON.stringify(event)+'\n';assert.ok(Buffer.byteLength(body)<=65536&&++events<=256);
            await journal.write(body);await journal.sync();
            if(event.name)report.last_original_stage=event.name;
        });return serial;
    });
    const selected=spec.transport==='chrome-webtransport'?'webtransport':'wss-webrtc';
    const result=await page.evaluate(async({spec,selected})=>{
        const {BrowserSdkClient}=await import('/sdk/browser.js');
        const {browserMatrixPlan,browserMatrixAdmission,retireBrowserMatrixAdmission}=await import('/_matrix/admission.mjs');
        const parameters=name=>name==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
        const actors={};window.matrixActors=actors;let state;
        try{
            for(const [name,configuration] of Object.entries(spec.actors)){
                window.matrixPhase='authenticate-'+name;
                const key=await crypto.subtle.importKey('jwk',configuration.signing.jwk,parameters(configuration.signing.algorithm),false,['sign']);
                const trust=await crypto.subtle.importKey('jwk',configuration.server.jwk,parameters(configuration.server.algorithm),false,['verify']);
                const enrollment={identityId:configuration.identityId,device:configuration.device,credential:configuration.credential,
                    signing:{key,keyId:configuration.signing.keyId,algorithm:configuration.signing.algorithm},
                    server:{key:trust,keyId:configuration.server.keyId,algorithm:configuration.server.algorithm,
                        notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt}};
                const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:configuration.identityId,
                    deviceId:configuration.device.id,endpoints:spec.endpoints,preferredTransport:selected,allowFallback:false,
                    timeoutMs:8000,attemptTimeoutMs:3000,nativeVideoFeedback:true,resolveEnrollment:async()=>enrollment});
                actors[name]={identityId:configuration.identityId,deviceId:configuration.device.id,client};
            }
            window.matrixPhase='admission';const plan=browserMatrixPlan();
            state=await browserMatrixAdmission({actors,plan,selectedTransport:selected,journal:event=>window.matrixJournal(event)});
            window.matrixPhase='original-retirement';
            const retirement=await retireBrowserMatrixAdmission({state,actors,plan,journal:event=>window.matrixJournal(event)});
            return {passed:retirement.passed,authenticated_owners:Object.keys(actors).length,admission:state.report,
                actual_connections:state.connections.length,retirement};
        }catch(error){
            const closes=await Promise.allSettled(Object.values(actors).map(actor=>actor.client.close({timeoutMs:5000})));
            return {passed:false,authenticated_owners:Object.keys(actors).length,failure:{stage:window.matrixPhase,
                name:error.name,kind:error.kind??null,certainty:error.certainty??null,
                problem:typeof error.problem?.reason==='string'&&/^[a-zA-Z0-9_-]{1,128}$/.test(error.problem.reason)
                    ?{code:error.problem.code,reason:error.problem.reason}:null},
                original_client_closes:closes.map(value=>value.status)};
        }
    },{spec,selected});
    Object.assign(report,result);stage='browser-retirement';
    report.trusted_browser=await owner.close();owner=undefined;
    report.passed=Boolean(report.passed&&report.trusted_browser.original_process_joined&&!report.trusted_browser.forced_kill);
}catch(error){
    report.passed=false;report.failure??={stage,name:error.name};
    await fs.writeFile(path.join(output,'failure-private.log'),String(error.stack??error),{mode:0o600});
}finally{
    if(owner)report.failure_browser=await owner.close();
    await serial.catch(()=>{});await journal?.close();await diagnostics;
    if(server)await new Promise(resolve=>server.close(resolve));
    report.journal_events=events;
    await fs.writeFile(path.join(output,'browser-observation.json'),JSON.stringify(report,null,2)+'\n',{mode:0o600});
}
process.stdout.write(JSON.stringify({browser_admission_passed:report.passed,cell_qualified:false})+'\n');
if(!report.passed)process.exitCode=1;
