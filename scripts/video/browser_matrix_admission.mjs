/** Actual owned Chrome/browser-SDK matrix admission. Private input stays on disk. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import https from 'node:https';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {createHash,X509Certificate} from 'node:crypto';
import {pathToFileURL} from 'node:url';
import {fixturePrivateJwk,fixtureServerJwk} from '../lib/browser_matrix_keys.mjs';
import {launchMatrixBrowser} from './owned_matrix_browser.mjs';
import {installPeerDiagnostics} from './browser_failure.mjs';
import {openLoopbackStun} from './loopback_stun.mjs';
import {matrixResourceBridge} from '../lib/matrix_resource_bridge.mjs';
import {matrixUnavailableTransports} from '../lib/matrix_unavailable_transports.mjs';
import {matrixAuthorityBridge} from '../lib/matrix_authority_bridge.mjs';
import {originalBrowserMatrixResult} from '../lib/browser_matrix_result.mjs';
import {matrixSdkAssetName} from '../lib/browser_matrix_sdk_asset.mjs';

const [inputPath]=process.argv.slice(2);
assert.equal(process.argv.length,3);
const root=await fs.realpath(process.cwd()),privateRoot=await fs.realpath(path.join(root,'.local'));
assert.ok((await fs.realpath(inputPath)).startsWith(privateRoot+path.sep));
assert.equal((await fs.stat(inputPath)).mode&0o077,0);
const spec=JSON.parse(await fs.readFile(inputPath,'utf8'));
assert.equal(spec.format,'connect/browser-matrix-admission-input/1');
assert.ok(['admission','programme','quality'].includes(spec.stage));
assert.ok(['chrome-webtransport','chrome-wss-webrtc','firefox-wss-webrtc'].includes(spec.transport));
assert.ok(['EdDSA','ES256'].includes(spec.signingAlgorithm));
const cycleCount=spec.resourceCycles??1;
assert.ok([1,3].includes(cycleCount)&& (cycleCount===1||spec.stage==='quality'));
assert.equal(spec.cycleWorkloads.length,cycleCount);
assert.ok(spec.cycleWorkloads.every((cycle,index)=>cycle.index===index+1
    &&typeof cycle.workload_id==='string'&&/^[a-zA-Z0-9_-]{1,48}$/.test(cycle.workload_id)));
assert.equal(new Set(spec.cycleWorkloads.map(cycle=>cycle.workload_id)).size,cycleCount);
assert.ok(['linux','darwin'].includes(process.platform)&&process.arch==='arm64');
if(process.platform==='linux')assert.equal(process.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER,'1');
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
spec.browserName=spec.transport.startsWith('firefox')?'firefox':'chrome';
const binary=spec.browserBinary;
const version=spawnSync(binary,['--version'],{encoding:'utf8',timeout:10000});
assert.equal(version.status,0);assert.equal(version.stdout.trim(),spec.expectedBrowserVersion);
const binaryFile=await fs.open(binary,'r'),header=Buffer.alloc(64);
try{assert.equal((await binaryFile.read(header,0,64,0)).bytesRead,64);}finally{await binaryFile.close();}
if(process.platform==='linux'){assert.equal(header.subarray(0,6).toString('hex'),'7f454c460201');assert.equal(header.readUInt16LE(18),183);}
const fixtureAddresses=spec.fixtureAddresses;
assert.ok(Array.isArray(fixtureAddresses)&&fixtureAddresses.length>=1&&fixtureAddresses.length<=2);
for(const address of fixtureAddresses){
    const parts=address.split('.').map(Number);assert.equal(parts.length,4);assert.ok(parts.every(v=>Number.isInteger(v)&&v>=0&&v<=255));
    assert.ok(address==='127.0.0.1'||parts[0]===10||parts[0]===192&&parts[1]===168||parts[0]===172&&parts[1]>=16&&parts[1]<=31);
    assert.ok(leaf.checkIP(address),'Fixture TLS SAN must authenticate each actual host');
}
for(const value of [spec.origin,spec.endpoints.webtransport,spec.endpoints['wss-webrtc']]){
    const url=new URL(value);
    assert.ok(['https:','wss:'].includes(url.protocol)&&fixtureAddresses.includes(url.hostname)&&url.port&&!url.username&&!url.password&&!url.search&&!url.hash);
}
for(const actor of Object.values(spec.actors)){
    assert.equal(actor.signing.algorithm,spec.signingAlgorithm);assert.equal(actor.server.algorithm,spec.signingAlgorithm);
    actor.signing.jwk=fixturePrivateJwk(Buffer.from(actor.signing.privatePkcs8,'base64url'),actor.signing.algorithm,
        Buffer.from(actor.signing.publicRaw,'base64url'));
    actor.server.jwk=await fixtureServerJwk(Buffer.from(actor.server.publicRaw,'base64url'),actor.server.algorithm);
    delete actor.signing.privatePkcs8;delete actor.signing.publicRaw;delete actor.server.publicRaw;
}
const report={format:'connect/browser-matrix-admission-observation/1',source_revision:spec.sourceRevision,
    transport:spec.transport,stage:spec.stage,signing_algorithm:spec.signingAlgorithm,browser:version.stdout.trim(),
    client_runtime:{platform:process.platform==='linux'?'ubuntu24-arm64':'macos-arm64',architecture:process.arch,node:process.version},
    passed:false,cell_qualified:false,task_complete:false,full_media_programme_executed:false,
    scope:'Actual isolated Chrome/public browser-SDK admission only. HTTPS/WSS use configured CA trust; WT uses its explicitly supplied fresh short-lived certificate SHA256 through the standard API. No full media/four-move/expiry/idle/fallback or complete-cell qualification.',
    tls_trust:{https_wss:'isolated-profile-CA',webtransport:spec.transport==='chrome-webtransport'?'explicit-fresh-certificate-sha256':null,
        leaf_sha256:leafSha256,leaf_key:'ECDSA-P256',leaf_validity_seconds:(notAfter-notBefore)/1000,certificate_errors_ignored:false}};
// The parent owns a 240-second process deadline. Leave room here for the
// original browser and listener joins, while preserving every inner deadline.
const programmeDeadline=performance.now()+210000*cycleCount;
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
            else if(url.pathname==='/_matrix/authority.mjs')file=path.join(root,'scripts/lib/browser_matrix_authority.mjs');
            else if(url.pathname==='/_matrix/sources.mjs')file=path.join(root,'scripts/lib/browser_matrix_sources.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_pipeline.mjs')file=path.join(root,'scripts/lib/browser_matrix_pipeline.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_stages.mjs')file=path.join(root,'scripts/lib/browser_matrix_stages.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_pcm.mjs')file=path.join(root,'scripts/lib/browser_matrix_pcm.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_audio_input.mjs')file=path.join(root,'scripts/lib/browser_matrix_audio_input.mjs');
            else if(url.pathname==='/_matrix/browser_matrix_rtc_observation.mjs')file=path.join(root,'scripts/lib/browser_matrix_rtc_observation.mjs');
            else if(url.pathname==='/_matrix/media_transition_coverage.mjs')file=path.join(root,'scripts/lib/media_transition_coverage.mjs');
            else if(url.pathname==='/_matrix/observation.mjs')file=path.join(root,'scripts/lib/browser_matrix_observation.mjs');
            else if(url.pathname==='/video/audio_envelope.mjs')file=path.join(root,'scripts/video/audio_envelope.mjs');
            else if(url.pathname==='/_matrix/retired-references.mjs')file=path.join(root,'scripts/lib/browser_matrix_retired_references.mjs');
            else if(url.pathname==='/_matrix/retirement-hooks.mjs')file=path.join(root,'scripts/lib/browser_matrix_retirement_hooks.mjs');
            else if(url.pathname==='/_matrix/programme.mjs')file=path.join(root,'scripts/lib/browser_matrix_programme.mjs');
            else if(url.pathname==='/video/marker_calibration.mjs')file=path.join(root,'scripts/video/marker_calibration.mjs');
            else if(url.pathname==='/video/source_identity.mjs')file=path.join(root,'scripts/video/source_identity.mjs');
            else{
                // The observation module retains its valid Node-relative SDK
                // import, which resolves beneath this second browser URL prefix.
                file=await fs.realpath(path.join(assets,matrixSdkAssetName(url.pathname)));
                assert.ok(file.startsWith(assets+path.sep));
            }
            response.writeHead(200,{'Content-Type':'text/javascript','Cache-Control':'no-store'});response.end(await fs.readFile(file));
        }catch{response.writeHead(404);response.end('fixture asset unavailable');}
    });
    await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(Number(new URL(spec.origin).port),new URL(spec.origin).hostname,resolve);});
    stage='untrusted-ca';
    owner=await launchMatrixBrowser(spec,path.join(output,'untrusted-profile'));
    let refused=false;
    try{await (await owner.browser.newPage()).goto(spec.origin,{timeout:5000});}
    catch(error){refused=/ERR_CERT_AUTHORITY_INVALID|SEC_ERROR_UNKNOWN_ISSUER|MOZILLA_PKIX_ERROR|insecure certificate/i.test(String(error.message));}
    report.untrusted_browser=await owner.close();owner=undefined;
    assert.ok(refused&&report.untrusted_browser.original_process_joined&&!report.untrusted_browser.forced_kill);
    report.unconfigured_ca_refused=true;
    stage='normal-ca-import';
    owner=await launchMatrixBrowser(spec,path.join(output,'trusted-profile'),async profile=>{
        const imported=spawnSync('python3',['scripts/video/browser_profile.py',spec.browserName,binary,profile,spec.tls.rootDer],
            {encoding:'utf8',timeout:10000});
        await fs.writeFile(path.join(output,'profile-import.log'),imported.stdout+imported.stderr,{mode:0o600});
        assert.equal(imported.status,0);const result=JSON.parse(imported.stdout);
        report.trust={normal_profile_ca_import:true,root_sha256:result.root_sha256,browser_version:result.version};
    });
    stage='current-browser-authentication';
    // Bind only the actual client interface already authenticated by the
    // fixture certificate, retaining ordinary browser mDNS/privacy policy.
    stun=await openLoopbackStun(new URL(spec.origin).hostname,fixtureAddresses);spec.rtcConfiguration={iceServers:[{urls:stun.url}]};
    unavailable=await matrixUnavailableTransports({cycles:cycleCount});spec.unavailableEndpoints=unavailable.endpoints;
    page=await owner.browser.newPage();
    await page.addInitScript(installPeerDiagnostics,{peerBound:16});
    page.on('console',message=>diagnostic(message.text()));
    page.on('pageerror',error=>diagnostic(error.stack??error));
    await page.goto(spec.origin,{timeout:5000});
    if(spec.stage==='quality')await page.exposeFunction('matrixResources',matrixResourceBridge(output,{cycles:cycleCount}));
    await page.exposeFunction('matrixAuthority',matrixAuthorityBridge(output,{cycles:cycleCount}));
    const cycleArchives=[],cycleSocketBaselines=[];
    await page.exposeFunction('matrixCycleStart',cycle=>{
        assert.equal(cycle.index,cycleSocketBaselines.length+1);
        assert.deepEqual(cycle,spec.cycleWorkloads[cycle.index-1]);
        cycleSocketBaselines.push(unavailable.observation());
    });
    await page.exposeFunction('matrixCycleArchive',async(cycle,valueJson)=>{
        assert.equal(typeof valueJson,'string');assert.ok(Buffer.byteLength(valueJson)<=16*1024*1024);
        const value=JSON.parse(valueJson);
        assert.equal(cycle.index,cycleArchives.length+1);
        assert.deepEqual(cycle,spec.cycleWorkloads[cycle.index-1]);
        const folder=path.join(output,'cycles',`cycle-${cycle.index}`);
        await fs.mkdir(folder,{recursive:true,mode:0o700});
        const save=async(name,data)=>{
            const body=JSON.stringify(data)+'\n';assert.ok(Buffer.byteLength(body)<=16*1024*1024);
            await fs.writeFile(path.join(folder,name),body,{flag:'wx',mode:0o600});
            return createHash('sha256').update(body).digest('hex');
        };
        const socketStart=cycleSocketBaselines[cycle.index-1],socketEnd=unavailable.observation();
        value.transport_probe_delta={udp_datagrams:socketEnd.udp_datagrams-socketStart.udp_datagrams,
            tcp_connections:socketEnd.tcp_connections-socketStart.tcp_connections,
            errors:socketEnd.errors-socketStart.errors,overflow:socketEnd.overflow};
        const joins={cycle,original_node_pid:process.pid,retirement:value.retirement,
            idle_resources:value.idle_resources,original_client_closes:value.original_client_closes,
            diagnostic_cycle_retirement:value.diagnostic_cycle_retirement};
        const evidence={...cycle,observation_sha256:await save('cycle-observation.json',value),
            workload_evidence_sha256:await save('workload.json',value.programme??{}),
            join_evidence_sha256:await save('original-joins.json',joins)};
        cycleArchives.push(evidence);return evidence;
    });
    const cycleEvents=Array(cycleCount).fill(0);
    journal=await fs.open(path.join(output,'original-operations.jsonl'),'wx',0o600);
    await page.exposeFunction('matrixJournal',event=>{
        serial=serial.then(async()=>{
            const body=JSON.stringify(event)+'\n';assert.ok(Buffer.byteLength(body)<=65536&&++events<=256*cycleCount);
            assert.ok(Number.isInteger(event.cycle_index)&&event.cycle_index>=1&&event.cycle_index<=cycleCount);
            assert.equal(event.workload_id,spec.cycleWorkloads[event.cycle_index-1].workload_id);
            assert.ok(++cycleEvents[event.cycle_index-1]<=256);
            await journal.write(body);await journal.sync();
            if(event.name)report.last_original_stage=event.name;
        });return serial;
    });
    const selected=spec.transport==='chrome-webtransport'?'webtransport':'wss-webrtc';
    await page.evaluate(({spec,selected,cycleCount})=>{
        if(globalThis.connectMatrixOriginalProgramme)throw Error('Original browser programme already started');
        const original=globalThis.connectMatrixOriginalProgramme={state:'pending'};
        // Start once, return immediately to automation, and retain the exact
        // completion. Polling below never submits another operation/workload.
        void (async()=>{
        const {BrowserSdkClient,BrowserClient,browserOperationKinds}=await import('/sdk/browser.js');
        // Diagnostic-only observer: retain the original low-level rejection in
        // the private browser log before the public SDK classifies it. No wire
        // bodies, credentials, retries or altered promise outcomes.
        const {BrowserMediaPeer}=await import('/sdk/browser_media.js');
        const {BrowserRtcConnection}=await import('/sdk/browser_sdk_webrtc.js');
        const {browserMatrixRetirementHooks,finishBrowserMatrixRetirement}=await import('/_matrix/retirement-hooks.mjs');
        const {browserMatrixPlan,browserMatrixAdmission,retireBrowserMatrixAdmission,browserMatrixCycleSummary,browserMatrixCycleOutcome}=await import('/_matrix/admission.mjs');
        const {browserMatrixSources}=await import('/_matrix/sources.mjs');
        const {browserMatrixProgramme,browserMatrixKeepalive,retireBrowserMatrixProgramme,closeBrowserMatrixProgramme,browserMatrixCleanupMoves}=await import('/_matrix/programme.mjs');
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
        const cycles=[];
        for(const cycle of spec.cycleWorkloads){
        const cycleStarted=performance.now(),cycleDeadline=cycleStarted+210000;
        await window.matrixCycleStart(cycle);
        let value=await (async()=>{
        const retirementHooks=browserMatrixRetirementHooks(BrowserMediaPeer,BrowserRtcConnection);
        const resources=(phase,terminal)=>window.matrixResources(phase,terminal,cycle.index);
        const journal=event=>{
            if(performance.now()>=cycleDeadline)throw Error('Original browser cycle deadline');
            return window.matrixJournal({...event,cycle_index:cycle.index,workload_id:cycle.workload_id});
        };
        const actors={};window.matrixActors=actors;let state={},sources,keepalive,observer,selectionEnrollment,cleanupMoves;
        const {browserMatrixRetiredReferences}=await import('/_matrix/retired-references.mjs');
        const retiredReferences=browserMatrixRetiredReferences({actors,state});
        const selection={},currentAuthority={};
        const programmeReport={};let resourceClock,resourceTerminalRequested=false,resourceRetiredRequested=false,failureHandled=false,diagnosticHooksReported=false;
        try{
            if(spec.stage==='quality'){
                const baseline=await resources('baseline');
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
            window.matrixPhase='admission';const plan=browserMatrixPlan(undefined,cycle);
            sources=await browserMatrixSources();
            if(spec.stage==='quality'){
                const {browserMatrixObservation}=await import('/_matrix/observation.mjs');
                const {BrowserMediaRenderer}=await import('/sdk/browser_media_renderer.js');
                observer=browserMatrixObservation(BrowserMediaRenderer,plan,selected==='webtransport'?BrowserMediaPeer:null,
                    selected==='wss-webrtc'?BrowserRtcConnection:null);
            }
            state=await browserMatrixAdmission({actors,plan,state,selectedTransport:selected,journal:event=>journal(event),
                startMedia:input=>sources.start(input),
                afterMediaOffer:selected==='wss-webrtc'&&observer
                    ?({route,connection,options})=>{observer.startInitialConnection(route.index,connection,options);}
                    :undefined,
                afterControl:async state=>{
                    window.matrixPhase='transport-selection';
                    const {browserMatrixSelection}=await import('/_matrix/selection.mjs');
                    try{await browserMatrixSelection({Sdk:BrowserSdkClient,Client:BrowserClient,enrollment:selectionEnrollment,
                        endpoints:spec.endpoints,unavailable:spec.unavailableEndpoints,reference:state.createReference,
                        conversationId:state.conversation.id,journal:event=>journal(event),report:selection});}
                    catch(error){selection.failure??={name:error.name,kind:error.kind??null};}
                    window.matrixPhase='current-authority-refusals';
                    const {browserMatrixAuthority}=await import('/_matrix/authority.mjs');
                    await browserMatrixAuthority({Sdk:BrowserSdkClient,provision:kind=>window.matrixAuthority(kind,cycle.index),
                        endpoints:spec.endpoints,selected,pin,state,report:currentAuthority});
                    window.matrixPhase='six-browser-media-offers';
                }});
            const sourceStartup=sources.observation();
            if(spec.stage!=='admission'){
                window.matrixPhase='four-move-programme';
                cleanupMoves=browserMatrixCleanupMoves({state,actors,plan,journal:event=>journal(event),operationKind:browserOperationKinds.connectionReplace});
                await browserMatrixProgramme({state,actors,plan,sources,journal:event=>journal(event),
                    keepalive,report:programmeReport,observer,selectedTransport:selected,cleanupMoves});
            }
            const retirement=await retireBrowserMatrixProgramme({keepalive,report:programmeReport,retire:async()=>{
                window.matrixPhase='original-retirement';
                if(spec.stage==='quality'){
                    resourceTerminalRequested=true;
                    await resources('terminal',resourceClock.host+(programmeReport.terminal_ms-resourceClock.browser)/1000);
                }
                return retireBrowserMatrixAdmission({state,actors,plan,journal:event=>journal(event),cleanupMoves,
                    closeOriginalClients:perform=>retiredReferences.closeClients(perform),
                    ...(spec.stage!=='admission'?{terminalMs:programmeReport.terminal_ms,reason:'matrix-programme-complete'}:{})});
            }});
            let idleResources;
            if(spec.stage==='quality'){
                const renderer=await observer.close();await sources.close();
                const sourceReferences=await sources.retireReferences(),source=sources.observation();
                const diagnosticHooks=await retirementHooks.finish();diagnosticHooksReported=true;
                const peers=diagnosticHooks.peers,rtc=diagnosticHooks.rtc;
                const transportOwnersJoined=diagnosticHooks.joined&&(selected==='webtransport'
                    ?peers.length>=4&&peers.every(p=>p.activeBindings===0&&p.queuedPackets===0&&p.queuedBytes===0)
                    :rtc.length>=6&&rtc.every(p=>p.original_shutdown_joined&&p.closed&&p.peerConnectionState==='closed'
                        &&p.remoteTracks===0&&p.liveRemoteTracks===0&&!p.captureActive&&!p.renderActive));
                const localOwnersJoined=renderer.joined&&source.source_terminal&&source.real_time_audio_state==='closed'
                    &&source.sources.every(s=>s.original_source_owner_live===false&&s.original_source_timer_active===false
                        &&s.original_source_retired===true&&s.attachments.every(a=>a.retired))
                    &&transportOwnersJoined;
                if(cleanupMoves&&!(retirement.cleanupRecovery?.released===true
                    &&retirement.cleanupRecovery.retained_move_records===0&&retirement.cleanupRecovery.retained_sdk_references===0))
                    throw Error('Original movement cleanup references must be released before idle');
                const fixtureReferences=retiredReferences.release({renderer,source:sourceReferences,transport:diagnosticHooks,keepalive:keepalive?.observation()});
                const completedMs=performance.now()-programmeReport.terminal_ms;
                retirement.passed&&=completedMs<=5000&&localOwnersJoined&&fixtureReferences.released;
                retirement.allLocalOwnersElapsedMs=completedMs;
                window.matrixPhase='ten-second-idle-observation';
                resourceRetiredRequested=true;
                const measured=await resources('retired');
                idleResources={passed:measured.passed&&localOwnersJoined&&fixtureReferences.released,localOwnersJoined,renderer,diagnostic_hooks:diagnosticHooks,fixture_reference_retirement:fixtureReferences,
                    ...(selected==='webtransport'?{media_peers:peers.length,active_bindings:peers.reduce((n,p)=>n+p.activeBindings,0),
                        queued_packets:peers.reduce((n,p)=>n+p.queuedPackets,0),queued_bytes:peers.reduce((n,p)=>n+p.queuedBytes,0)}:
                        {rtc_owners:rtc,media_peers:rtc.length,active_bindings:rtc.filter(p=>!p.closed).length,
                            queue_observation:'RTC has no SDK datagram queues; original RTCPeerConnections must be closed and shutdown joined'})};
            }
            return {workload_started:Boolean(programmeReport.source_programme),
                workload_complete:programmeReport.source_programme?.durationMs===62000
                    &&programmeReport.finished_ms>=62000&&programmeReport.finished_ms<67000,cycle:{...cycle},passed:retirement.passed&&selection.passed&&currentAuthority.passed&&(spec.stage==='admission'||programmeReport.passed)&&(!observer||(programmeReport.decoded_media.passed&&idleResources?.renderer.playback_passed===true))&&(!idleResources||idleResources.passed),
                ...(observer?{decoded_media:programmeReport.decoded_media,
                    ...(programmeReport.decoded_media.passed?{}:{failure:{stage:'decoded-media-quality',name:'ProfileBudgetsFailed'}})}:{}),wrong_pin_refusal:pinRefusal,authenticated_owners:retiredReferences.counts().authenticated_owners,admission:state.report,
                ...(programmeReport.keepalive_failure?{failure:{stage:'original-keepalive',...programmeReport.keepalive_failure}}:{}),
                actual_connections:retiredReferences.counts().actual_connections,source_startup:sourceStartup,retirement,idle_resources:idleResources,selection,current_authority:currentAuthority,
                ...(spec.stage!=='admission'?{programme:programmeReport,keepalive:keepalive.observation()}:{}),
                ...(spec.stage!=='admission'?{scope:'Actual admission and four-move programme; decoded quality and idle resources are explicit when requested. Capture/expiry/fallback require separate evidence.'}:{})};
        }catch(error){
            failureHandled=true;
            console.error('Matrix failure',String(error.stack??error),JSON.stringify({kind:error.kind,certainty:error.certainty,problem:error.problem}));
            const cleanupErrors=[];
            if(spec.stage==='quality'&&resourceClock&&!resourceTerminalRequested){
                // A failed original setup still owns real resources. Observe
                // its actual failure/cleanup boundary once; this does not claim
                // that the fixed media programme or its retirement completed.
                resourceTerminalRequested=true;
                try{await resources('terminal',resourceClock.host+(performance.now()-resourceClock.browser)/1000);}
                catch(failure){cleanupErrors.push({stage:'terminal',name:failure.name});}
            }
            const closes=await retiredReferences.closeClients(client=>client.close({timeoutMs:5000}));
            cleanupMoves?.release();
            const {outcomes:local}=await closeBrowserMatrixProgramme({keepalive,observer,sources,report:programmeReport,previousFailure:true});
            local.forEach((value,index)=>{if(value.status==='rejected')cleanupErrors.push({stage:['keepalive','renderer','source'][index],name:value.reason?.name??'Error'});});
            const diagnosticHooks=await retirementHooks.finish();diagnosticHooksReported=true;
            if(!diagnosticHooks.joined)cleanupErrors.push({stage:'diagnostic-hooks',name:'OriginalShutdownNotJoined'});
            let sourceReferences;
            try{sourceReferences=await sources?.retireReferences();}catch(error){cleanupErrors.push({stage:'source-references',name:error.name});}
            const fixtureReferences=retiredReferences.release({renderer:local[1].status==='fulfilled'?local[1].value:undefined,
                source:sourceReferences,transport:diagnosticHooks,keepalive:keepalive?.observation()});
            let failedIdle;
            if(spec.stage==='quality'&&resourceClock&&resourceTerminalRequested&&!resourceRetiredRequested){
                resourceRetiredRequested=true;
                try{
                    const measured=await resources('retired');
                    failedIdle={passed:false,localOwnersJoined:false,original_failure_cleanup_observed:true,
                        resource_restoration:measured.passed,
                        originalCleanupOwnersJoined:closes.every(value=>value.status==='fulfilled')
                            &&local[1].status==='fulfilled'&&local[1].value.joined===true
                            &&sourceReferences?.joined===true&&sourceReferences?.released===true
                            &&diagnosticHooks.joined===true&&keepalive?.observation().stopped===true
                            &&keepalive?.observation().original_batch_joined===true,
                        original_client_closes:closes.map(value=>value.status),
                        local_cleanup:local.map(value=>value.status),
                        renderer:local[1].status==='fulfilled'?local[1].value:{joined:false,playback_passed:false},
                        diagnostic_hooks:diagnosticHooks,fixture_reference_retirement:fixtureReferences,errors:cleanupErrors};
                }catch(failure){cleanupErrors.push({stage:'retired',name:failure.name});}
            }
            return {workload_started:Boolean(programmeReport.source_programme),
                workload_complete:programmeReport.source_programme?.durationMs===62000
                    &&programmeReport.finished_ms>=62000&&programmeReport.finished_ms<67000,cycle:{...cycle},passed:false,wrong_pin_refusal:pinRefusal,authenticated_owners:retiredReferences.counts().authenticated_owners,
                admission:state.report,actual_connections:retiredReferences.counts().actual_connections,failure:{stage:window.matrixPhase,
                name:error.name,kind:error.kind??null,certainty:error.certainty??null,
                problem:typeof error.problem?.reason==='string'&&/^[a-zA-Z0-9_-]{1,128}$/.test(error.problem.reason)
                    ?{code:error.problem.code,reason:error.problem.reason}:null},source_startup:sources?.observation(),
                programme:programmeReport,...(programmeReport.decoded_media?{decoded_media:programmeReport.decoded_media}:{}),
                keepalive:keepalive?.observation(),selection,current_authority:currentAuthority,
                idle_resources:failedIdle,original_cleanup_errors:cleanupErrors,original_client_closes:closes.map(value=>value.status)};
        } finally {
            const closed=await closeBrowserMatrixProgramme({keepalive,observer,sources,report:programmeReport,previousFailure:failureHandled});
            cleanupMoves?.release();
            await finishBrowserMatrixRetirement({hooks:retirementHooks,closed,reported:diagnosticHooksReported,previousFailure:failureHandled});}
        })();
        value.original_cycle_budget={started_ms:cycleStarted,deadline_ms:cycleDeadline,finished_ms:performance.now(),budget_ms:210000};
        value.passed&&=value.original_cycle_budget.finished_ms<cycleDeadline;
        const closedOwners=value.idle_resources?.localOwnersJoined===true
            ||value.idle_resources?.originalCleanupOwnersJoined===true;
        value.diagnostic_cycle_retirement=cycleCount===1?{released:true,scope:'single-cycle-no-diagnostic-reset'}:
            closedOwners?globalThis.connectPeerRetireCycle?.()??{released:false}:
                {released:false,reason:'original-render-source-client-joins-unproven'};
        value.passed&&=value.diagnostic_cycle_retirement.released===true;
        const evidence=await window.matrixCycleArchive(cycle,JSON.stringify(value));
        cycles.push(browserMatrixCycleSummary(cycle,value,evidence));
        // Full AV, diagnostics and closure graphs are held only by the host
        // archive after this point. Keep no previous workload owner in JS.
        const joined=value.diagnostic_cycle_retirement.released===true;
        value=undefined;
        if(!joined)break; // Never start more workload behind an unjoined original owner.
        }
        if(cycleCount===3&&cycles.length===3)return browserMatrixCycleOutcome(cycles);
        const first=cycles.find(value=>value.failure||!value.passed)??cycles[0];
        return {...first,passed:cycleCount===1&&first.passed,cycles,
            unstarted_cycles:spec.cycleWorkloads.slice(cycles.length),
            ...(cycleCount===1&&first.passed||first.failure?{}:{failure:{stage:'original-cycle-joins',name:'OriginalCycleIncomplete'}})};
        })().then(value=>{original.value=value;original.state='fulfilled';},error=>{
            original.error={name:error.name,message:String(error.message??error),stack:String(error.stack??error)};
            original.state='rejected';
        });
    },{spec,selected,cycleCount});
    const result=await originalBrowserMatrixResult(page,{deadline:programmeDeadline});
    assert.ok(cycleArchives.length>0&&cycleArchives.length<=cycleCount);
    const primaryIndex=result.first_failed_cycle??1;
    const primary=JSON.parse(await fs.readFile(path.join(output,'cycles',`cycle-${primaryIndex}`,'cycle-observation.json'),'utf8'));
    Object.assign(report,primary,result);report.resource_cycles=cycleArchives;
    report.rtc_diagnostics=await page.evaluate(()=>globalThis.connectPeerDiagnostics?.()??{unavailable:true});
    stage='browser-retirement';
    report.trusted_browser=await owner.close();owner=undefined;
    report.passed=Boolean(report.passed&&report.trusted_browser.original_process_joined&&!report.trusted_browser.forced_kill);
}catch(error){
    report.passed=false;report.failure??={stage:error.matrixPhase??stage,name:error.name};
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
