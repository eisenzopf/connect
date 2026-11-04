/** Actual owned Chrome/browser-SDK matrix admission. Private input stays on disk. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import https from 'node:https';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {createHash,X509Certificate} from 'node:crypto';
import {pathToFileURL} from 'node:url';
import {fixturePrivateJwk,fixtureServerJwk} from '../lib/browser_matrix_keys.mjs';
import {launchOwnedChromium} from './owned_chromium.mjs';
import {installPeerDiagnostics} from './browser_failure.mjs';
import {openLoopbackStun} from './loopback_stun.mjs';
import {matrixResourceBridge} from '../lib/matrix_resource_bridge.mjs';
import {matrixUnavailableTransports} from '../lib/matrix_unavailable_transports.mjs';

const [inputPath]=process.argv.slice(2);
assert.equal(process.argv.length,3);
const root=await fs.realpath(process.cwd()),privateRoot=await fs.realpath(path.join(root,'.local'));
assert.ok((await fs.realpath(inputPath)).startsWith(privateRoot+path.sep));
assert.equal((await fs.stat(inputPath)).mode&0o077,0);
const spec=JSON.parse(await fs.readFile(inputPath,'utf8'));
assert.equal(spec.format,'connect/browser-matrix-admission-input/1');
assert.ok(['admission','programme','quality'].includes(spec.stage));
assert.ok(spec.stage==='admission'||spec.transport==='chrome-webtransport');
assert.ok(['chrome-webtransport','chrome-wss-webrtc'].includes(spec.transport));
assert.ok(['EdDSA','ES256'].includes(spec.signingAlgorithm));
assert.ok(process.platform==='linux'&&process.arch==='arm64');
assert.equal(process.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER,'1');
const output=await fs.realpath(spec.output);
assert.ok(output.startsWith(privateRoot+path.sep));
const leaf=new X509Certificate(await fs.readFile(spec.tls.certificatePem));
const leafSha256=createHash('sha256').update(leaf.raw).digest('hex');
assert.equal(leafSha256,spec.tls.leafSha256);
assert.equal(leaf.publicKey.asymmetricKeyType,'ec');
assert.equal(leaf.publicKey.asymmetricKeyDetails.namedCurve,'prime256v1');
const notBefore=Date.parse(leaf.validFrom),notAfter=Date.parse(leaf.validTo);
assert.ok(notBefore<=Date.now()&&Date.now()<notAfter&&notAfter-notBefore<=14*86400000);
assert.equal(spec.tls.webtransportTrust,'certificate-sha256');
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
    transport:spec.transport,stage:spec.stage,signing_algorithm:spec.signingAlgorithm,browser:version.stdout.trim(),
    passed:false,cell_qualified:false,task_complete:false,full_media_programme_executed:false,
    scope:'Actual isolated Chrome/public browser-SDK admission only. HTTPS/WSS use configured CA trust; WT uses its explicitly supplied fresh short-lived certificate SHA256 through the standard API. No full media/four-move/expiry/idle/fallback or complete-cell qualification.',
    tls_trust:{https_wss:'isolated-profile-CA',webtransport:spec.transport==='chrome-webtransport'?'explicit-fresh-certificate-sha256':null,
        leaf_sha256:leafSha256,leaf_key:'ECDSA-P256',leaf_validity_seconds:(notAfter-notBefore)/1000,certificate_errors_ignored:false}};
let owner,server,page,journal,stun,unavailable,stage='asset-listener',events=0,serial=Promise.resolve(),diagnostics=Promise.resolve(),diagnosticBytes=0;
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
            else if(url.pathname==='/_matrix/selection.mjs')file=path.join(root,'scripts/lib/browser_matrix_selection.mjs');
            else if(url.pathname==='/_matrix/sources.mjs')file=path.join(root,'scripts/lib/browser_matrix_sources.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_pipeline.mjs')file=path.join(root,'scripts/lib/browser_matrix_pipeline.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_audio_input.mjs')file=path.join(root,'scripts/lib/browser_matrix_audio_input.mjs');
            else if(url.pathname==='/_matrix/observation.mjs')file=path.join(root,'scripts/lib/browser_matrix_observation.mjs');
            else if(url.pathname==='/video/audio_envelope.mjs')file=path.join(root,'scripts/video/audio_envelope.mjs');
            else if(url.pathname==='/_matrix/programme.mjs')file=path.join(root,'scripts/lib/browser_matrix_programme.mjs');
            else if(url.pathname==='/video/marker_calibration.mjs')file=path.join(root,'scripts/video/marker_calibration.mjs');
            else if(url.pathname==='/video/source_identity.mjs')file=path.join(root,'scripts/video/source_identity.mjs');
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
    // Local scope uses an owned Binding reflector so the browser can supply
    // actual IP candidates while retaining its ordinary mDNS/privacy policy.
    stun=await openLoopbackStun();spec.rtcConfiguration={iceServers:[{urls:stun.url}]};
    unavailable=await matrixUnavailableTransports();spec.unavailableEndpoints=unavailable.endpoints;
    page=await owner.browser.newPage();
    await page.addInitScript(installPeerDiagnostics);
    page.on('console',message=>diagnostic(message.text()));
    page.on('pageerror',error=>diagnostic(error.stack??error));
    await page.goto(spec.origin,{timeout:5000});
    if(spec.stage==='quality')await page.exposeFunction('matrixResources',matrixResourceBridge(output));
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
        const {BrowserSdkClient,BrowserClient}=await import('/sdk/browser.js');
        // Diagnostic-only observer: retain the original low-level rejection in
        // the private browser log before the public SDK classifies it. No wire
        // bodies, credentials, retries or altered promise outcomes.
        const {BrowserMediaPeer}=await import('/sdk/browser_media.js');
        const retiredPeers=new Set(),originalShutdown=BrowserMediaPeer.prototype.shutdown;
        BrowserMediaPeer.prototype.shutdown=function(...args){
            if(!retiredPeers.has(this)&&retiredPeers.size>=7)throw Error('Bounded original media owners');
            retiredPeers.add(this);return originalShutdown.apply(this,args);
        };
        const beginReplacement=BrowserMediaPeer.prototype.beginReplacementConnection;
        BrowserMediaPeer.prototype.beginReplacementConnection=function(...args){
            const original=beginReplacement.apply(this,args);
            void original.completed.catch(error=>console.error('Original replacement rejection',
                String(error.code??error.name),String(error.message),String(error.stack)));
            return original;
        };
        const {browserMatrixPlan,browserMatrixAdmission,retireBrowserMatrixAdmission}=await import('/_matrix/admission.mjs');
        const {browserMatrixSources}=await import('/_matrix/sources.mjs');
        const {browserMatrixProgramme,browserMatrixKeepalive}=await import('/_matrix/programme.mjs');
        const pin=Uint8Array.from(spec.tls.leafSha256.match(/../g),value=>parseInt(value,16));
        let pinRefusal;
        if(selected==='webtransport'){
            const wrong=pin.slice();wrong[0]^=1;
            const physical=new WebTransport(spec.endpoints.webtransport,{allowPooling:false,
                serverCertificateHashes:[{algorithm:'sha-256',value:wrong}]});
            let timer,refused=false,closed=false;
            void physical.closed.catch(()=>{});
            try{
                await Promise.race([physical.ready.catch(error=>{refused=true;throw error;}),
                    new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('original-wrong-pin-deadline')),3000);})]);
            }catch(error){if(!refused)throw error;}
            finally{clearTimeout(timer);physical.close();await Promise.race([
                physical.closed.then(()=>{closed=true;},()=>{closed=true;}),
                new Promise(resolve=>{timer=setTimeout(resolve,3000);})]);clearTimeout(timer);}
            if(!refused||!closed)throw Error('Wrong WT certificate pin was not refused and joined');
            pinRefusal={mismatched_leaf_refused:true,original_transport_closed:true,application_streams_created:0,application_mutations_submitted:0};
        }
        const parameters=name=>name==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
        const actors={};window.matrixActors=actors;let state,sources,keepalive,observer,selectionEnrollment;
        const selection={};
        const programmeReport={};let resourceClock;
        try{
            if(spec.stage==='quality'){
                const baseline=await window.matrixResources('baseline');
                resourceClock={host:baseline.monotonic_seconds,browser:performance.now()};
            }
            for(const [name,configuration] of Object.entries(spec.actors)){
                window.matrixPhase='authenticate-'+name;
                const key=await crypto.subtle.importKey('jwk',configuration.signing.jwk,parameters(configuration.signing.algorithm),false,['sign']);
                const trust=await crypto.subtle.importKey('jwk',configuration.server.jwk,parameters(configuration.server.algorithm),false,['verify']);
                const enrollment={identityId:configuration.identityId,device:configuration.device,credential:configuration.credential,
                    signing:{key,keyId:configuration.signing.keyId,algorithm:configuration.signing.algorithm},
                    server:{key:trust,keyId:configuration.server.keyId,algorithm:configuration.server.algorithm,
                        notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt},
                    serverCertificateHashes:[{algorithm:'sha-256',value:pin.slice()}]};
                if(name==='participant-1')selectionEnrollment=enrollment;
                const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:configuration.identityId,
                    deviceId:configuration.device.id,endpoints:spec.endpoints,preferredTransport:selected,allowFallback:false,
                    timeoutMs:8000,attemptTimeoutMs:3000,nativeVideoFeedback:true,media:spec.rtcConfiguration,resolveEnrollment:async()=>enrollment});
                actors[name]={identityId:configuration.identityId,deviceId:configuration.device.id,endpointId:configuration.endpointId,client};
            }
            if(spec.stage!=='admission')keepalive=browserMatrixKeepalive(actors);
            window.matrixPhase='admission';const plan=browserMatrixPlan();
            sources=await browserMatrixSources();
            if(spec.stage==='quality'){
                const {browserMatrixObservation}=await import('/_matrix/observation.mjs');
                const {BrowserMediaRenderer}=await import('/sdk/browser_media_renderer.js');
                observer=browserMatrixObservation(BrowserMediaRenderer,plan,BrowserMediaPeer);
            }
            state=await browserMatrixAdmission({actors,plan,selectedTransport:selected,journal:event=>window.matrixJournal(event),
                startMedia:input=>sources.start(input)});
            window.matrixPhase='transport-selection';
            const {browserMatrixSelection}=await import('/_matrix/selection.mjs');
            await browserMatrixSelection({Sdk:BrowserSdkClient,Client:BrowserClient,enrollment:selectionEnrollment,
                endpoints:spec.endpoints,unavailable:spec.unavailableEndpoints,reference:state.createReference,
                conversationId:state.conversation.id,journal:event=>window.matrixJournal(event),report:selection});
            const sourceStartup=sources.observation();
            if(spec.stage!=='admission'){
                window.matrixPhase='four-move-programme';
                await browserMatrixProgramme({state,actors,plan,sources,journal:event=>window.matrixJournal(event),
                    keepalive,report:programmeReport,observer});
                await keepalive.close();
            }
            window.matrixPhase='original-retirement';
            if(spec.stage==='quality')await window.matrixResources('terminal',
                resourceClock.host+(programmeReport.terminal_ms-resourceClock.browser)/1000);
            const retirement=await retireBrowserMatrixAdmission({state,actors,plan,journal:event=>window.matrixJournal(event),
                ...(spec.stage!=='admission'?{terminalMs:programmeReport.terminal_ms,reason:'matrix-programme-complete'}:{})});
            let idleResources;
            if(spec.stage==='quality'){
                const renderer=await observer.close();await sources.close();
                const source=sources.observation();
                const peers=[...retiredPeers].map(peer=>peer.diagnostics);
                const localOwnersJoined=renderer.joined&&source.source_terminal&&source.real_time_audio_state==='closed'
                    &&source.sources.every(s=>!s.original_master_tracks_live&&s.attachments.every(a=>a.retired))
                    &&peers.length>=4&&peers.every(p=>p.activeBindings===0&&p.queuedPackets===0&&p.queuedBytes===0);
                const completedMs=performance.now()-programmeReport.terminal_ms;
                retirement.passed&&=completedMs<=5000&&localOwnersJoined;
                retirement.allLocalOwnersElapsedMs=completedMs;
                window.matrixPhase='ten-second-idle-observation';
                const measured=await window.matrixResources('retired');
                idleResources={passed:measured.passed&&localOwnersJoined,localOwnersJoined,renderer,
                    media_peers:peers.length,active_bindings:peers.reduce((n,p)=>n+p.activeBindings,0),
                    queued_packets:peers.reduce((n,p)=>n+p.queuedPackets,0),queued_bytes:peers.reduce((n,p)=>n+p.queuedBytes,0)};
            }
            return {passed:retirement.passed&&(!observer||programmeReport.decoded_media.passed)&&(!idleResources||idleResources.passed),
                ...(observer?{decoded_media:programmeReport.decoded_media,
                    ...(programmeReport.decoded_media.passed?{}:{failure:{stage:'decoded-media-quality',name:'ProfileBudgetsFailed'}})}:{}),wrong_pin_refusal:pinRefusal,authenticated_owners:Object.keys(actors).length,admission:state.report,
                actual_connections:state.connections.length,source_startup:sourceStartup,retirement,idle_resources:idleResources,selection,
                ...(spec.stage!=='admission'?{programme:programmeReport,keepalive:keepalive.observation()}:{}),
                ...(spec.stage!=='admission'?{scope:'Actual admission and four-move programme; decoded quality and idle resources are explicit when requested. Capture/expiry/fallback require separate evidence.'}:{})};
        }catch(error){
            console.error('Matrix failure',String(error.stack??error),JSON.stringify({kind:error.kind,certainty:error.certainty,problem:error.problem}));
            const closes=await Promise.allSettled(Object.values(actors).map(actor=>actor.client.close({timeoutMs:5000})));
            return {passed:false,wrong_pin_refusal:pinRefusal,authenticated_owners:Object.keys(actors).length,failure:{stage:window.matrixPhase,
                name:error.name,kind:error.kind??null,certainty:error.certainty??null,
                problem:typeof error.problem?.reason==='string'&&/^[a-zA-Z0-9_-]{1,128}$/.test(error.problem.reason)
                    ?{code:error.problem.code,reason:error.problem.reason}:null},source_startup:sources?.observation(),
                programme:programmeReport,keepalive:keepalive?.observation(),selection,original_client_closes:closes.map(value=>value.status)};
        } finally {await keepalive?.close().catch(()=>{});await observer?.close();await sources?.close();BrowserMediaPeer.prototype.shutdown=originalShutdown;}
    },{spec,selected});
    Object.assign(report,result);report.rtc_diagnostics=await page.evaluate(()=>globalThis.connectPeerDiagnostics?.()??{unavailable:true});
    stage='browser-retirement';
    report.trusted_browser=await owner.close();owner=undefined;
    report.passed=Boolean(report.passed&&report.trusted_browser.original_process_joined&&!report.trusted_browser.forced_kill);
}catch(error){
    report.passed=false;report.failure??={stage,name:error.name};
    await fs.writeFile(path.join(output,'failure-private.log'),String(error.stack??error),{mode:0o600});
}finally{
    if(owner)report.failure_browser=await owner.close();
    if(stun)report.owned_stun=await stun.close();
    if(unavailable){report.unavailable_transports=await unavailable.close();
        const sockets=report.unavailable_transports;
        report.passed&&=sockets.original_sockets_joined&&sockets.udp_datagrams>0&&sockets.tcp_connections>0&&!sockets.errors&&!sockets.overflow;}
    await serial.catch(()=>{});await journal?.close();await diagnostics;
    if(server)await new Promise(resolve=>server.close(resolve));
    report.journal_events=events;
    await fs.writeFile(path.join(output,'browser-observation.json'),JSON.stringify(report,null,2)+'\n',{mode:0o600});
}
process.stdout.write(JSON.stringify({browser_admission_passed:report.passed,cell_qualified:false})+'\n');
if(!report.passed)process.exitCode=1;
