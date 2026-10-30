/** Real Chromium/WebRTC independent decoder proof. All addresses are loopback;
 * credentials arrive only through private stdin and never enter its report.
 */
import { pathToFileURL } from "node:url";
import process from "node:process";
import {connectFirefoxBidi} from "./browser_bidi_adapter.mjs";
import {createPrivateKey,webcrypto} from "node:crypto";
import http from "node:http";
import https from "node:https";
import fs from "node:fs/promises";
import path from "node:path";
import { ordinalSync } from "./marker_calibration.mjs";
import { audioObservation } from "./audio_observation.mjs";
import { combineAudioEnvelopeSync } from "./audio_envelope.mjs";
import { acceptancePolicy, audioVideoAcceptance } from "./acceptance_policy.mjs";
import { motionObservation } from "./motion_observation.mjs";
import { workerPipeline } from "./encoded_worker_observation.mjs";
import { installPeerDiagnostics, installControlDiagnostics, browserFailure } from "./browser_failure.mjs";
import { installIceOwnership, restartOriginalBrowsers } from "./browser_ice_restart.mjs";
import { refusedRelay } from "./browser_relay_expiry.mjs";
const moduleInput = process.env.CONNECT_PLAYWRIGHT_MODULE;

let input = "";
for await (const chunk of process.stdin) {
    input += chunk;
    if (input.length > 65536) throw new Error("Browser fixture input too large");
}
const spec = JSON.parse(input);
if (!Array.isArray(spec.participants) || ![1,2].includes(spec.participants.length) || !spec.terminal)
    throw Error("Canonical browser qualification requires explicit signed participant configurations, subscriptions and a stable Session end; bearer-only legacy fixture migration is unsupported");
// Protected local fixture key material is normalized inside this private
// process before WebCrypto imports it in the actual page. Never log this spec.
for(const participant of spec.participants){
    if(participant.signing?.privatePkcs8){
        participant.signing.jwk=createPrivateKey({key:Buffer.from(participant.signing.privatePkcs8,"base64url"),format:"der",type:"pkcs8"}).export({format:"jwk"});
        delete participant.signing.privatePkcs8;
    }
    if(participant.server?.publicRaw){
        const algorithm=participant.server.algorithm==="EdDSA"?{name:"Ed25519"}:
            participant.server.algorithm==="ES256"?{name:"ECDSA",namedCurve:"P-256"}:null;
        if(!algorithm)throw Error("Unknown fixture server signing algorithm");
        const key=await webcrypto.subtle.importKey("raw",Buffer.from(participant.server.publicRaw,"base64url"),algorithm,true,["verify"]);
        participant.server.jwk=await webcrypto.subtle.exportKey("jwk",key);
        delete participant.server.publicRaw;
    }
}
for(const port of [spec.assetPort,spec.debugPort])if(port!==undefined&&(!Number.isInteger(port)||port<1||port>65535))
    throw Error("Use an explicit bounded local fixture port");
spec.browser??="chrome";
if(!["chrome","firefox"].includes(spec.browser))throw Error("Unknown designated installed browser");
const chromium=spec.browser==="firefox"?undefined:(await import(moduleInput ? pathToFileURL(moduleInput).href : "playwright")).chromium;
spec.profileMode??="clean";
if(!["clean","impairment","mute"].includes(spec.profileMode))throw Error("Unknown browser measurement scenario");
if(Boolean(spec.muteRecovery)!==(spec.profileMode==="mute") || (spec.muteRecovery&&spec.seconds<20))
    throw Error("Mute measurement requires its separate scenario and at least 20 seconds");
if(spec.iceRestart&&(spec.profileMode!=="clean"||spec.participants.length!==2||spec.designatedRemoteHost))
    throw Error("ICE restart qualification requires two owned loopback clean-profile participants");
if(spec.requireRelay&&(!spec.iceRestart||spec.participants.some(p=>p.connection?.iceTransportPolicy!=="relay")))
    throw Error("Owned relay qualification requires explicit relay-only original-peer ICE recovery");
let assets;
if(spec.root) {
    const root=await fs.realpath(spec.root);
    const publicFolders=await Promise.all(["examples/conversation-client","sdk/typescript/dist"].map(name=>fs.realpath(path.join(root,name))));
    const serveAsset=async(request,response)=>{
        try {
            const url=new URL(request.url,"http://127.0.0.1");
            if(request.method==="GET"&&url.pathname==="/_qualification/source-identity.mjs"){
                response.writeHead(200,{"Content-Type":"text/javascript"});
                response.end(await fs.readFile(new URL("./source_identity.mjs",import.meta.url)));return;
            }
            if(request.method==="GET"&&url.pathname==="/_qualification/marker-calibration.mjs"){
                response.writeHead(200,{"Content-Type":"text/javascript"});
                response.end(await fs.readFile(new URL("./marker_calibration.mjs",import.meta.url)));return;
            }
            if(request.method==="GET"&&url.pathname==="/_qualification/audio-envelope.mjs"){
                response.writeHead(200,{"Content-Type":"text/javascript"});
                response.end(await fs.readFile(new URL("./audio_envelope.mjs",import.meta.url)));return;
            }
            if(request.method==="GET"&&url.pathname==="/_qualification/audio-spectrum.mjs"){
                response.writeHead(200,{"Content-Type":"text/javascript"});
                response.end(await fs.readFile(new URL("./audio_spectrum.mjs",import.meta.url)));return;
            }
            if(request.method==="GET"&&url.pathname==="/_qualification/audio-sample-clock.mjs"){
                response.writeHead(200,{"Content-Type":"text/javascript"});
                response.end(await fs.readFile(new URL("./audio_sample_clock.mjs",import.meta.url)));return;
            }
            if(request.method!=="GET" || !(url.pathname.startsWith("/examples/conversation-client/") || url.pathname.startsWith("/sdk/typescript/dist/"))) throw Error("asset denied");
            const name=path.resolve(root,"."+decodeURIComponent(url.pathname)+(url.pathname.endsWith("/")?"index.html":""));
            const actual=await fs.realpath(name);
            if(!publicFolders.some(folder=>actual.startsWith(folder+path.sep)))throw Error("asset denied");
            const bytes=await fs.readFile(actual);
            response.writeHead(200,{"Content-Type":actual.endsWith(".js")?"text/javascript":actual.endsWith(".css")?"text/css":"text/html"});response.end(bytes);
        }catch {response.writeHead(404);response.end("asset unavailable");}
    };
    assets=spec.assetTls?https.createServer({cert:await fs.readFile(spec.assetTls.certFile),
        key:await fs.readFile(spec.assetTls.keyFile)},serveAsset):http.createServer(serveAsset);
    await new Promise((resolve,reject)=>{assets.once("error",reject);assets.listen(spec.assetPort??0,"127.0.0.1",resolve)});
    spec.baseURL=spec.assetTls?`https://localhost:${assets.address().port}`:`http://127.0.0.1:${assets.address().port}`;
}
for (const address of [spec.baseURL, ...spec.participants.map(p=>p.url)]) {
    const url = new URL(address);
    const ownedRemote=spec.designatedRemoteHost==="host.lima.internal"&&url.hostname===spec.designatedRemoteHost
        &&url.protocol==="wss:"&&url.pathname==="/connect"&&url.port&&!url.hash;
    if ((!ownedRemote&&!["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)) || url.username || url.password || url.search)
        throw new Error("Use an isolated loopback browser fixture");
}
if (!Number.isInteger(spec.seconds) || spec.seconds < 1 || spec.seconds > 120 || ![1,2].includes(spec.participants.length))
    throw new Error("Invalid bounded browser rehearsal");
for(const tones of [spec.sourceToneHz,spec.expectedToneHz])if(tones!==undefined&&(!Array.isArray(tones)
    ||tones.length!==spec.participants.length||tones.some(value=>!Number.isFinite(value)||value<100||value>3000)))
    throw new Error("Generated/expected audio requires an identifiable 100–3000 Hz tone per participant");
let browser;
const contexts = [];
const observedPages = [];
let cleanupPromise;
const cleanup=()=>cleanupPromise??=(async()=>{
    await Promise.allSettled(contexts.map(context=>context.close()));
    await browser?.close();
    if(assets)await new Promise(resolve=>assets.close(resolve));
})();
process.once("SIGTERM",()=>{cleanup().finally(()=>process.exit(1));});
try {
    browser=spec.browser==="firefox"?
        await connectFirefoxBidi(process.env.CONNECT_PUPPETEER_MODULE,spec.debugPort):
        spec.debugPort!==undefined?await chromium.connectOverCDP(`http://127.0.0.1:${spec.debugPort}`):
        await chromium.launch({headless:true,executablePath:process.env.CONNECT_CHROME_BINARY,
            args:["--autoplay-policy=no-user-gesture-required","--disable-background-timer-throttling","--disable-renderer-backgrounding"]});
    if(spec.expiredRelayOnly||spec.unreachableRelayOnly) {
        if(spec.requireRelay||spec.iceRestart||spec.designatedRemoteHost
            ||(spec.expiredRelayOnly&&spec.unreachableRelayOnly)
            ||spec.participants.some(p=>p.connection?.iceTransportPolicy!=="relay"))
            throw Error('Relay refusal probe requires its separate owned loopback relay-only scene');
        const reason=spec.expiredRelayOnly?'expired':'unreachable';
        const refusal=await refusedRelay(browser,spec,reason);
        const version=browser.version();await cleanup();
        console.log(JSON.stringify({status:'passed',browser:version,[`${reason}_relay`]:refusal}));
    } else {
    const pages = await Promise.all(spec.participants.map(async (configuration,index) => {
        const context = await browser.newContext(); contexts.push(context);
        const page = await context.newPage();
        observedPages[index]=page;
        await page.addInitScript(installPeerDiagnostics);
        await page.addInitScript(installControlDiagnostics);
        if(spec.iceRestart)await page.addInitScript(installIceOwnership);
        await page.goto(spec.baseURL + "/examples/conversation-client/");
        // Exercise normal sticky user activation for AudioContext in both runtimes.
        await page.mouse.click(1,1);
        await page.evaluate(async ({ configuration, frequency, sourceId }) => {
            const { BrowserVideoSession, BrowserClient } = await import("/sdk/typescript/dist/browser.js");
            const {canonicalMediaClient}=await import("/examples/conversation-client/canonical-media.js");
            let fallback=null;
            if(configuration.fallbackProbe){
                const probe=configuration.fallbackProbe,url=new URL(probe.url);
                if(typeof WebTransport!=="function"||url.protocol!=="https:"||!["127.0.0.1","localhost","[::1]"].includes(url.hostname)
                    ||url.username||url.password||url.search||!Number.isInteger(probe.timeoutMs)||probe.timeoutMs<100||probe.timeoutMs>2000)
                    throw Error("Actual bounded loopback WebTransport probe required");
                const algorithm=name=>name==="EdDSA"?{name:"Ed25519"}:name==="ES256"?{name:"ECDSA",namedCurve:"P-256"}:null;
                const key=await crypto.subtle.importKey("jwk",configuration.signing.jwk,algorithm(configuration.signing.algorithm),false,["sign"]);
                const trust=await crypto.subtle.importKey("jwk",configuration.server.jwk,algorithm(configuration.server.algorithm),false,["verify"]);
                const started=performance.now();let opened,errorCode;
                try {
                    opened=await BrowserClient.open({transport:"webtransport",url:probe.url,timeoutMs:probe.timeoutMs,
                        credential:configuration.credential,identityId:configuration.identityId,device:configuration.device,
                        signing:{algorithm:configuration.signing.algorithm,keyId:configuration.signing.keyId,key},
                        server:{algorithm:configuration.server.algorithm,keyId:configuration.server.keyId,key:trust,
                            notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt}});
                }catch(error){errorCode=error?.code??"unclassified";}
                finally{opened?.close();}
                if(opened||!["timeout","transport"].includes(errorCode))throw Error("Designated UDP blackhole did not cause the expected physical WebTransport refusal");
                fallback={transport:"webtransport",status:"refused",error_code:errorCode,seconds:(performance.now()-started)/1000,
                    original_timeout_ms:probe.timeoutMs,application_mutations_attempted:0,replayed_originals:[]};
            }
            const control=await canonicalMediaClient(configuration).catch(error=>{throw Error(`Canonical WSS open failed: ${error?.code??"unclassified"}`)});
            for(const original of configuration.fallbackOriginals??[]){
                if(!fallback)throw Error("Original transport-switch replay requires its actual failed WebTransport probe");
                const terminalType={"message.send":"ack","session.invite":"operation.updated","session.accept":"operation.updated"}[original.type];
                if(!terminalType)throw Error("Unknown original fixture replay command");
                const frames=await control.client.request(original.type,original.payload,original.scope,{timeoutMs:5000,terminalType}).catch(error=>{throw Error(`Original ${original.type} replay failed: ${error?.code??"unclassified"}`)});
                const status=await control.client.operationStatus(original.scope.cid,original.scope.operation_id,{timeoutMs:5000}).catch(error=>{throw Error(`Original ${original.type} status failed: ${error?.code??"unclassified"}`)});
                if(status.state!=="completed")throw Error("Original preaccepted operation is not completed on WSS");
                if(original.type==="message.send"){
                    const details=frames.find(frame=>frame.type==="ack")?.payload.details;
                    if(details?.kind!=="accepted"||details.data?.message_id!==original.payload.msg_id
                        ||details.data?.request_id!==original.scope.operation_id||details.data?.conversation_id!==original.scope.cid)
                        throw Error("WSS did not preserve the original accepted message receipt");
                }
                fallback.replayed_originals.push({type:original.type,operation_id:original.scope.operation_id,state:status.state});
            }
            const {markerCells,classifySourceFrame,sourceIdentityReport}=await import("/_qualification/source-identity.mjs");
            const identityCells=markerCells(sourceId);
            const {spectralTone}=await import("/_qualification/audio-spectrum.mjs");
            const {markerFrames}=await import("/_qualification/marker-calibration.mjs");
            const pulseTimes=markerFrames().map(frame=>frame/30);
            const sourceBegan=performance.now();let pulseOrdinal=0;
            const source = document.createElement("canvas"); source.width = 640; source.height = 360;
            const paint = source.getContext("2d"); let n=0;
            // Source observations are numeric capture metadata, not decoded
            // content or transport receipt. Keep their scope explicit.
            const sourceObservation={draws:0,pulses:0,lastDrawUnix:null,lastPulseUnix:null,
                maximumDrawGapSeconds:0,pulseTimes:[],pulseOrdinals:[],wasPulse:null};
            const audio = new AudioContext(); await audio.resume();
            const oscillator = audio.createOscillator(); oscillator.frequency.value = frequency;
            const gain=audio.createGain();
            const destination = audio.createMediaStreamDestination(); destination.channelCount=1; oscillator.connect(gain);gain.connect(destination); oscillator.start();
            const draw = () => {
                const now=(performance.timeOrigin+performance.now())/1000;
                const elapsed=(performance.now()-sourceBegan)/1000;
                while(pulseOrdinal+1<pulseTimes.length&&pulseTimes[pulseOrdinal+1]<=elapsed)pulseOrdinal++;
                const pulse=elapsed>=pulseTimes[pulseOrdinal]&&elapsed<pulseTimes[pulseOrdinal]+.1;
                if(sourceObservation.lastDrawUnix!==null)sourceObservation.maximumDrawGapSeconds=Math.max(
                    sourceObservation.maximumDrawGapSeconds,now-sourceObservation.lastDrawUnix);
                sourceObservation.draws++;sourceObservation.lastDrawUnix=now;
                if(pulse&&sourceObservation.wasPulse===false){
                    sourceObservation.pulses++;sourceObservation.lastPulseUnix=now;
                    if(sourceObservation.pulseTimes.length<512){sourceObservation.pulseTimes.push(now);
                        sourceObservation.pulseOrdinals.push(pulseOrdinal);}
                }
                sourceObservation.wasPulse=pulse;
                gain.gain.setValueAtTime(pulse?0.7:0.05,audio.currentTime);
                paint.fillStyle=`hsl(${n % 360} 90% 40%)`; paint.fillRect(0,0,640,360);
                paint.fillStyle="white"; paint.fillRect((n*7)%580,70,60,150); n++;
                identityCells.forEach((value,index)=>{paint.fillStyle=`rgb(${value},${value},${value})`;
                    paint.fillRect((index%2)*80,Math.floor(index/2)*45,80,45);});
                if(pulse){paint.fillStyle="white";paint.fillRect(0,0,640,360);}
            };
            draw(); const timer=setInterval(draw,1000/30);
            const stream = new MediaStream(destination.stream.getAudioTracks());
            const session = await BrowserVideoSession.open(control.client,{...control.options,inputStream:stream,videoCanvas:source});
            const video=document.createElement("video"); video.muted=true; video.autoplay=true;
            video.style.cssText="position:fixed;inset:0;width:640px;height:360px;z-index:10000";
            document.body.append(video);

            const sample=document.createElement("canvas"); sample.width=8;sample.height=8;
            const pixels=sample.getContext("2d",{willReadFrequently:true});
            pixels.imageSmoothingEnabled=false; // Sample uniform marker centres, without neighboring background blend.
            const observation={rendered:0,changing:0,last:null,videoEdges:[],audioEdges:[],changingTimes:[],audioSamples:[],audioTicks:0,white:null,audioHigh:null,sourceIdentityCounts:{}};
            const decoded=()=>{
                observation.rendered++;
                pixels.drawImage(video,0,0,8,8);
                const bytes=pixels.getImageData(0,0,8,8).data;
                const key=Array.from(bytes).join(",");
                const gray=Array.from({length:64},(_,index)=>.299*bytes[index*4]+.587*bytes[index*4+1]+.114*bytes[index*4+2]);
                const sourceKind=classifySourceFrame(gray);
                if(observation.recordIdentity)observation.sourceIdentityCounts[sourceKind]=(observation.sourceIdentityCounts[sourceKind]??0)+1;
                const white=Array.from(bytes).filter((_,index)=>index%4!==3).every(value=>value>240);
                if(white&&observation.white===false)observation.videoEdges.push(performance.now());
                observation.white=white;
                if(observation.last!==null && key!==observation.last) {
                    observation.changing++;observation.changingTimes.push((performance.timeOrigin+performance.now())/1000);
                }
                observation.last=key; video.requestVideoFrameCallback(decoded);
            };
            video.requestVideoFrameCallback(decoded);
            window.videoFixture={session,control,video,fallback,observation,sourceObservation,sourceIdentityReport,spectralTone,timer,audio,oscillator,muteTimers:[],muteTransitions:[]};
        }, { configuration,frequency:spec.sourceToneHz?.[index]??440+index*220,
            sourceId:spec.sourceIds?.[index]??index });
        return page;
    }));
    const visibility=await Promise.all(pages.map(page=>page.evaluate(()=>document.visibilityState)));
    if(visibility.some(state=>state!=="visible"))throw Error("Original browser fixture receiver must be visible");
    // Both original publishers must be durably admitted before either
    // receiver selects its counterpart. An SDP answer alone is insufficient.
    await Promise.all(pages.map(page=>page.evaluate(async()=>{
        const {client,options}=window.videoFixture.control,until=performance.now()+10000;
        for(;;){
            if(performance.now()>=until)throw Error("Original publisher admission is not confirmed complete");
            const status=await client.operationStatus(options.conversationId,options.operationId,{timeoutMs:Math.max(1,until-performance.now())});
            if(status.state==="completed")return;
            if(["failed","cancelled"].includes(status.state))throw Error("Original publisher admission failed or was cancelled");
            await new Promise(resolve=>setTimeout(resolve,20));
        }
    })));
    // The canonical host grants no implicit receive selection. Apply only the
    // caller's original explicit intents after all physical peers have answered.
    await Promise.all(pages.map(page=>page.evaluate(()=>window.videoFixture.control.receive())));
    // The canonical route has explicit receive admission; playback can become
    // ready only after those original selections have completed.
    await Promise.all(pages.map(page=>page.evaluate(()=>{const f=window.videoFixture;return f.session.attach(f.video)})));
    // Initial negotiation does not count toward the clean 60-second window.
    await new Promise(resolve=>setTimeout(resolve,1500));
    await Promise.all(pages.map(page=>page.evaluate(async()=>{
        const f=window.videoFixture;
        const {AudioEnvelope}=await import("/_qualification/audio-envelope.mjs");
        const {audioSampleClock}=await import("/_qualification/audio-sample-clock.mjs");
        const sampleDue=audioSampleClock();
        f.audioEnvelope=new AudioEnvelope();
        if(f.session.remoteStream.getAudioTracks().length!==1)throw Error("No independent remote audio track");
        const source=f.audio.createMediaStreamSource(f.session.remoteStream);
        const analyser=f.audio.createAnalyser();analyser.fftSize=256;source.connect(analyser);f.analyser=analyser;
        // Keep marker observation fast, and use a separate longer window so
        // distinct 440/660 Hz sources cannot collide in coarse frequency bins.
        const frequencyAnalyser=f.audio.createAnalyser();frequencyAnalyser.fftSize=2048;
        source.connect(frequencyAnalyser);frequencyAnalyser.smoothingTimeConstant=0;f.frequencyAnalyser=frequencyAnalyser;
        f.audioTimer=setInterval(()=>{
            const values=new Float32Array(analyser.fftSize);analyser.getFloatTimeDomainData(values);
            const rms=Math.sqrt(values.reduce((s,v)=>s+v*v,0)/values.length);
            const observedAt=performance.now();
            if(f.audioEnvelope.sample(observedAt/1000,rms))f.observation.audioEdges.push(observedAt);
            if(sampleDue(observedAt) && f.observation.audioSamples.length<2000) {
                const frequencySamples=new Float32Array(f.frequencyAnalyser.fftSize);
                f.frequencyAnalyser.getFloatTimeDomainData(frequencySamples);
                const spectrum=new Float32Array(f.frequencyAnalyser.frequencyBinCount);f.frequencyAnalyser.getFloatFrequencyData(spectrum);
                const tone=f.spectralTone(frequencySamples,spectrum,f.audio.sampleRate,f.frequencyAnalyser.fftSize);
                f.observation.audioSamples.push({unix_seconds:(performance.timeOrigin+performance.now())/1000,...tone});
            }
        },10);
    })));
    // All receivers must establish the expected decoded tone before their
    // measured windows start. No microphone/device permission is requested.
    const audioReadiness=await Promise.all(pages.map((page,index)=>page.evaluate(async({expected})=>{
        const f=window.videoFixture,deadline=performance.now()+5000;
        while(performance.now()<deadline){
            const recent=f.observation.audioSamples.slice(-3);
            if(recent.length===3&&recent.every(row=>row.rms>.001&&Math.abs(row.frequency_hz-expected)<35))
                return{status:"identified",expected_frequency_hz:expected,consecutive_samples:3,
                    samples:recent,limit_seconds:5};
            await new Promise(resolve=>setTimeout(resolve,25));
        }
        throw Error("Identifiable duplex decoded audio did not become ready within five seconds");
    },{expected:spec.expectedToneHz?.[index]??(index===0?660:440)})));
    const relayPath=async()=>Promise.all(pages.map(page=>page.evaluate(async()=>{
        const stats=await window.videoFixture.session.diagnostics();
        const incoming=stats.find(row=>row.direction==="inbound");
        if(incoming?.selectedCandidateType!=="relay"||incoming.selectedCandidatePairSource!=="stats")
            throw Error("Actual nominated relay candidate pair is missing");
        return {candidate_type:incoming.selectedCandidateType,observation_source:incoming.selectedCandidatePairSource};
    })));
    const relayBefore=spec.requireRelay?await relayPath():undefined;
    const iceRestarts=spec.iceRestart?await restartOriginalBrowsers(pages,spec.participants,process.env.CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS):undefined;
    const relayAfter=spec.requireRelay?await relayPath():undefined;
    const initial = await Promise.all(pages.map(page=>page.evaluate(async()=>({
        stats:await window.videoFixture.session.diagnostics(),rendered:window.videoFixture.observation.rendered,
        changing:window.videoFixture.observation.changing,
        source:{...window.videoFixture.sourceObservation},markerStart:performance.now(),markerStartUnix:(performance.timeOrigin+performance.now())/1000 }))));
    await Promise.all(pages.map((page,index)=>page.evaluate(({initialStats})=>{
        const f=window.videoFixture;f.observation.recordIdentity=true;f.iceSamples=[];f.outboundSamples=[];f.iceErrors=0;f.iceBusy=false;
        const finite=value=>Number.isFinite(value)&&value>=0?value:undefined;
        f.outboundMetadata=stats=>{
            const outgoing=stats.find(row=>row.direction==="outbound");
            if(!outgoing)return undefined;
            return {codec:outgoing.codec,frames:finite(outgoing.frames),keyframes:finite(outgoing.keyframes),
                width:finite(outgoing.width),height:finite(outgoing.height),packets:finite(outgoing.packets),
                bytes:finite(outgoing.bytes),timestamp:finite(outgoing.timestamp),
                encode_seconds:finite(outgoing.encodeSeconds),packet_send_delay_seconds:finite(outgoing.packetSendDelaySeconds),
                frames_per_second:finite(outgoing.framesPerSecond),
                captured_frames:finite(outgoing.capturedFrames),captured_frames_per_second:finite(outgoing.capturedFramesPerSecond),
                target_bitrate_bits_per_second:finite(outgoing.targetBitrate),
                remote_packets_lost:Number.isSafeInteger(outgoing.remotePacketsLost)?outgoing.remotePacketsLost:undefined,
                remote_fraction_lost:finite(outgoing.remoteFractionLost),remote_round_trip_time_seconds:finite(outgoing.remoteRoundTripTimeSeconds),
                available_outgoing_bitrate_bits_per_second:finite(outgoing.availableOutgoingBitrate),
                // BiDi drops repeated object aliases. Copy the current numeric
                // snapshot so it survives beside the same getStats result.
                worker:outgoing.worker?{...outgoing.worker}:undefined,
                quality_limit:["none","cpu","bandwidth","other"].includes(outgoing.qualityLimit)?outgoing.qualityLimit:undefined};
        };
        f.recordIce=stats=>{
            const incoming=stats.find(row=>row.direction==="inbound");
            const at=(performance.timeOrigin+performance.now())/1000;
            if(f.outboundSamples.length<256)f.outboundSamples.push({unix_seconds:at,
                outbound:f.outboundMetadata(stats),source_draws:f.sourceObservation.draws,
                source_pulses:f.sourceObservation.pulses,last_source_draw_unix_seconds:f.sourceObservation.lastDrawUnix,
                last_source_pulse_unix_seconds:f.sourceObservation.lastPulseUnix,
                maximum_source_draw_gap_seconds:f.sourceObservation.maximumDrawGapSeconds});
            if(f.iceSamples.length<256)f.iceSamples.push({unix_seconds:(performance.timeOrigin+performance.now())/1000,
                remote_port:incoming?.selectedRemotePort,local_port:incoming?.selectedLocalPort,
                transport:incoming?.selectedTransport,candidate_type:incoming?.selectedCandidateType,
                observation_source:incoming?.selectedCandidatePairSource,
                decoded_keyframes:incoming?.keyframes,decoded_frames:incoming?.frames});
        };
        f.recordIce(initialStats);
        f.iceTimer=setInterval(async()=>{
            if(f.iceBusy)return;f.iceBusy=true;
            try {f.recordIce(await f.session.diagnostics());}catch {f.iceErrors++;}
            finally {f.iceBusy=false;}
        },500);
    },{initialStats:initial[index].stats})));
    if(spec.windowReadyPath)await fs.writeFile(spec.windowReadyPath,JSON.stringify({
        starts_unix_seconds:initial.map(row=>row.markerStartUnix)}),{flag:"wx",mode:0o600});
    const began=performance.now();
    if(spec.muteRecovery) await Promise.all(pages.map(page=>page.evaluate(()=>{
        const f=window.videoFixture;
        for(const [kind,at] of [["video",6],["audio",10]])for(const [muted,delay] of [[true,at],[false,at+2]])
            f.muteTimers.push(setTimeout(()=>{
                if(kind==="video")f.session.setCameraMuted(muted);else f.session.setMicrophoneMuted(muted);
                f.muteTransitions.push({kind,muted,unix_seconds:(performance.timeOrigin+performance.now())/1000});
            },delay*1000));
    })));
    await new Promise(resolve=>setTimeout(resolve,spec.seconds*1000));
    const elapsed=(performance.now()-began)/1000;
    const final = await Promise.all(pages.map((page,index)=>page.evaluate(async({expectedSourceId,allowMutedBlack,markerStart})=>{
        const f=window.videoFixture;clearInterval(f.iceTimer);
        f.observation.recordIdentity=false;
        const stats=await f.session.diagnostics();f.recordIce(stats);
        return {stats,sourceIdentity:f.sourceIdentityReport(f.observation.sourceIdentityCounts,expectedSourceId,allowMutedBlack),markerEndUnix:(performance.timeOrigin+performance.now())/1000,
        iceSamples:f.iceSamples,iceErrors:f.iceErrors,outboundSamples:f.outboundSamples,
        outbound:f.outboundMetadata(stats),source:{...f.sourceObservation},muteTransitions:f.muteTransitions,
        audioEnvelope:f.audioEnvelope.report(markerStart),audioSamples:f.observation.audioSamples,rendered:window.videoFixture.observation.rendered,
        changing:window.videoFixture.observation.changing,
        videoEdges:window.videoFixture.observation.videoEdges,audioEdges:window.videoFixture.observation.audioEdges,
        changingTimes:window.videoFixture.observation.changingTimes,
        audio:(()=>{const f=window.videoFixture;const samples=new Float32Array(f.frequencyAnalyser.fftSize);
            f.frequencyAnalyser.getFloatTimeDomainData(samples);
            const spectrum=new Float32Array(f.frequencyAnalyser.frequencyBinCount);f.frequencyAnalyser.getFloatFrequencyData(spectrum);
            return f.spectralTone(samples,spectrum,f.audio.sampleRate,f.frequencyAnalyser.fftSize);})() };
    },{expectedSourceId:spec.expectedSourceIds?.[index]??(index===0?1:0),allowMutedBlack:Boolean(spec.muteRecovery),markerStart:initial[index].markerStart/1000})));
    const observations=final.map((end,index)=>{
        const inbound=end.stats.find(s=>s.direction==="inbound");
        const before=initial[index].stats.find(s=>s.direction==="inbound");
        const outgoingBefore=initial[index].stats.find(s=>s.direction==="outbound");
        const outgoingDelta=key=>end.outbound&&outgoingBefore&&Number.isFinite(end.outbound[key])
            &&Number.isFinite(outgoingBefore[key])&&end.outbound[key]>=outgoingBefore[key]
            ?end.outbound[key]-outgoingBefore[key]:null;
        const delta=key=>inbound&&before&&Number.isFinite(inbound[key])&&Number.isFinite(before[key])
            &&inbound[key]>=before[key]?inbound[key]-before[key]:null;
        const observedElapsed=end.markerEndUnix-initial[index].markerStartUnix;
        const video=end.videoEdges.filter(time=>time>=initial[index].markerStart);
        const audio=end.audioEdges.filter(time=>time>=initial[index].markerStart);
        const sync=combineAudioEnvelopeSync(ordinalSync(video.map(time=>time/1000),audio.map(time=>time/1000),spec.seconds),end.audioEnvelope);
        sync.time_basis="independent decoded browser video rendering/audio gain edges on the same performance clock; no sender timing or RTP-origin inference";
        const expected=spec.expectedToneHz?.[index]??(index===0?660:440);
        const audioContent={window_start_unix_seconds:initial[index].markerStartUnix,
            window_end_unix_seconds:end.markerEndUnix,samples:end.audioSamples};
        const audioProof=spec.muteRecovery&&pages.length===1?{status:"requires-counterpart-setter-binding",
            scope:"actual native setter times are joined by the parent checker after peer termination"}
            :audioObservation(audioContent,expected,spec.profileMode,
                spec.muteRecovery?final[1-index].muteTransitions:[],spec.networkBursts??[]);
        const measuredTimes=end.changingTimes.filter(time=>time>=initial[index].markerStartUnix&&time<=end.markerEndUnix);
        const motionContent={window_start_unix_seconds:initial[index].markerStartUnix,window_end_unix_seconds:end.markerEndUnix,changing_times:end.changingTimes};
        const motionProof=spec.muteRecovery&&pages.length===1?{status:"requires-counterpart-setter-binding"}
            :motionObservation(motionContent,spec.profileMode,spec.muteRecovery?final[1-index].muteTransitions:[],spec.networkBursts??[]);
        const motionBoundaries=[initial[index].markerStartUnix,...measuredTimes,end.markerEndUnix];
        const maximumMotionGap=Math.max(...motionBoundaries.slice(1).map((time,index)=>time-motionBoundaries[index]));
        const recovery=(spec.networkBursts??[]).map(burst=>{
            const observedWindow=burst.start_unix_seconds>=initial[index].markerStartUnix&&burst.end_unix_seconds<end.markerEndUnix;
            const before=measuredTimes.some(time=>time<burst.start_unix_seconds);
            const resumed=measuredTimes.find(time=>time>=burst.end_unix_seconds);
            const delay=resumed===undefined?null:resumed-burst.end_unix_seconds;
            const baseline=end.iceSamples.filter(sample=>sample.unix_seconds<=burst.end_unix_seconds&&Number.isFinite(sample.decoded_keyframes)).at(-1);
            const increment=baseline?end.iceSamples.find(sample=>sample.unix_seconds>burst.end_unix_seconds
                &&sample.unix_seconds<=burst.end_unix_seconds+2&&sample.decoded_keyframes>baseline.decoded_keyframes):undefined;
            return {burst,burst_inside_measured_window:observedWindow,changing_content_before_burst:before,
                decoded_keyframe_increment_observation:increment?{status:"observed",
                    sampled_interval_start_unix_seconds:baseline.unix_seconds,sampled_interval_end_unix_seconds:increment.unix_seconds,
                    before:baseline.decoded_keyframes,after:increment.decoded_keyframes,
                    method:"public independent-decoder counter increase over a sampled interval; no per-rendered-frame keyframe flag"}:{status:"unobserved"},
                fresh_decoded_content_after_end_seconds:delay,status:observedWindow&&before&&delay!==null&&delay<=2?"passed":"failed"};
        });
        const videoBytes=delta("bytes");
        return {stats_available:Boolean(inbound&&before),source_identity:end.sourceIdentity,
            audio_readiness:audioReadiness[index],audio_content:audioContent,audio_profile:audioProof,motion_content:motionContent,motion_profile:motionProof,
            worker_pipeline:workerPipeline(outgoingBefore?.worker,end.outbound?.worker,end.outboundSamples,initial[index].markerStartUnix,end.markerEndUnix),
            window_start_unix_seconds:initial[index].markerStartUnix,window_end_unix_seconds:end.markerEndUnix,decoded:delta("frames"),changing:end.changing-initial[index].changing,
            selected_ice_samples:end.iceSamples,selected_ice_observation_errors:end.iceErrors,
            outbound_observation:{scope:"public browser encoder/sender counters; not independent decoded content or receiver arrival",
                sample_interval_seconds:.5,sample_bound:256,samples:end.outboundSamples,
                final:end.outbound,frames:outgoingDelta("frames"),keyframes:outgoingDelta("keyframes"),
                packets:outgoingDelta("packets"),bytes:outgoingDelta("bytes"),
                bitrate_bits_per_second:outgoingDelta("bytes")===null?null:outgoingDelta("bytes")*8/observedElapsed},
            source_observation:{scope:"generated source draw callback; not encoded or decoded content",
                initial:initial[index].source,final:end.source,
                draws:end.source.draws-initial[index].source.draws,
                draws_per_second:(end.source.draws-initial[index].source.draws)/observedElapsed,
                proven_pulse_rising_edges_unix_seconds:end.source.pulseTimes.filter(at=>at>=initial[index].markerStartUnix&&at<=end.markerEndUnix),
                pulse_time_bound:512,maximum_draw_gap_scope:"whole source participation including negotiation"},
            mute_transitions:end.muteTransitions,
            ...(spec.muteRecovery?{mute_content:{window_start_unix_seconds:initial[index].markerStartUnix,
                window_end_unix_seconds:end.markerEndUnix,
                changing_times:end.changingTimes,audio_samples:end.audioSamples}}:{}),
            rendered:end.rendered-initial[index].rendered,width:inbound?.width,height:inbound?.height,
            codec:inbound?.codec,packets:delta("packets"),lost:delta("lost"),jitter:inbound?.jitter,
            video_rtp_bytes:videoBytes,video_rtp_bitrate_bits_per_second:videoBytes===null?null:videoBytes*8/observedElapsed,
            keyframes:delta("keyframes"),audio_rms:end.audio.rms,audio_frequency_hz:end.audio.frequency_hz,
            maximum_changing_decoded_frame_observation_gap_seconds:maximumMotionGap,
            expected_audio_frequency_hz:expected,av_sync:sync,
            av_acceptance:audioVideoAcceptance(sync,spec.profileMode,spec.seconds),recovery,elapsed_seconds:observedElapsed};
    });
    const fallback=await Promise.all(pages.map(page=>page.evaluate(()=>window.videoFixture.fallback)));
    const relayFinal=spec.requireRelay?await relayPath():undefined;
    const closeAt=performance.now();
    const release=page=>page.evaluate(async()=>{
        const f=window.videoFixture;clearInterval(f.timer);clearInterval(f.audioTimer);f.muteTimers.forEach(clearTimeout);f.oscillator.stop();await f.audio.close();await f.session.close();
        return f.session.localPreview.getTracks().every(t=>t.readyState==="ended");
    });
    // End the durable Session explicitly; local capture close is not a peer end.
    await pages[0].evaluate(async terminal=>{
        const f=window.videoFixture, options=f.control.options;
        if(typeof terminal.operationId!=="string" || !Number.isSafeInteger(terminal.expectedRevision))throw Error("Explicit original Session end operation/revision required");
        await f.control.client.request("session.end",{by_participant:options.participantId,reason_code:200,reason:"fixture-complete"},
            {cid:options.conversationId,sid:options.sessionId,operation_id:terminal.operationId,expected_revision:terminal.expectedRevision},{terminalType:"operation.updated"});
        const state=await f.control.client.operationStatus(options.conversationId,terminal.operationId);
        if(state.state!=="completed")throw Error("Original Session end is not confirmed complete");
    },spec.terminal);
    const ended=[await release(pages[0])];
    let pairedTermination={status:"not-applicable",seconds:null};
    if(pages.length===2) {
        const peerAt=performance.now();
        const peerEnded=await pages[1].evaluate(async()=>{
            const f=window.videoFixture;
            return Promise.race([f.session.ended.then(()=>true),new Promise(resolve=>setTimeout(()=>resolve(false),4500))]);
        });
        pairedTermination={status:peerEnded?"passed":"failed",seconds:(performance.now()-peerAt)/1000,limit_seconds:5,
            method:"first participant submits the original signed Session end and confirms its journal status; second must receive terminal before its own local close"};
        ended.push(await release(pages[1]));
    }
    await Promise.all(pages.map(page=>page.evaluate(()=>window.videoFixture.control.client.close())));
    await cleanup();
    const teardown=(performance.now()-closeAt)/1000;
    const passed=observations.every(o=>o.worker_pipeline.status==="passed"&&(o.motion_profile.status==="passed"||(spec.muteRecovery&&pages.length===1&&o.motion_profile.status==="requires-counterpart-setter-binding"))&&(o.audio_profile.status==="passed"||(spec.muteRecovery&&pages.length===1&&o.audio_profile.status==="requires-counterpart-setter-binding"))&&o.source_identity.status==="passed"&&o.stats_available&&o.decoded!==null&&o.codec?.toLowerCase()==="video/vp8"&&o.width===640&&o.height===360
        &&o.decoded/o.elapsed_seconds>=(spec.profileMode==="clean"?24:1)&&o.changing/o.elapsed_seconds>=(spec.profileMode==="clean"?24:1)&&o.audio_rms>0.001
        &&Math.abs(o.audio_frequency_hz-o.expected_audio_frequency_hz)<35
        &&o.av_sync.audio_envelope?.status==="observer-valid"&&o.av_acceptance.status==="passed"
        &&o.recovery.every(item=>item.status==="passed"))&&teardown<5&&ended.every(Boolean)&&pairedTermination.status!=="failed";
    console.log(JSON.stringify({status:passed?"passed":"failed",profile:"connect-video-vp8/1",acceptance_policy:acceptancePolicy,
        browser:browser.version(),fallback,observations,ice_restarts:iceRestarts,
        relay_path:spec.requireRelay?{before:relayBefore,after:relayAfter,final:relayFinal}:undefined,
        teardown_seconds:teardown,paired_termination:pairedTermination,
        ended_unix_seconds:Date.now()/1000}));
    if(!passed)process.exitCode=1;
    }
} catch (error) {
    console.log(JSON.stringify(await browserFailure(observedPages, error)));
    throw error;
} finally {
    await cleanup();
}
