/** Actual Chrome/Firefox canonical fallback transition control. All addresses are loopback;
 * credentials arrive only through private stdin and never enter its report.
 */
import { pathToFileURL } from "node:url";
import process from "node:process";
import {connectFirefoxBidi} from "./browser_bidi_adapter.mjs";
import {createPrivateKey,webcrypto} from "node:crypto";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import {DatabaseSync} from "node:sqlite";
const moduleInput = process.env.CONNECT_PLAYWRIGHT_MODULE;

let input = "";
for await (const chunk of process.stdin) {
    input += chunk;
    if (input.length > 65536) throw new Error("Browser fixture input too large");
}
const spec = JSON.parse(input);
if(spec.controlOnly!==true||!Array.isArray(spec.controlTransitions)||spec.controlTransitions.length!==3)throw Error("Explicit bounded transition-control fixture required");
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
let assets;
if(spec.root) {
    const root=await fs.realpath(spec.root);
    const publicFolders=await Promise.all(["examples/conversation-client","sdk/typescript/dist"].map(name=>fs.realpath(path.join(root,name))));
    assets=http.createServer(async(request,response)=>{
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
    });
    await new Promise((resolve,reject)=>{assets.once("error",reject);assets.listen(spec.assetPort??0,"127.0.0.1",resolve)});
    spec.baseURL=`http://127.0.0.1:${assets.address().port}`;
}
for (const address of [spec.baseURL, ...spec.participants.map(p=>p.url)]) {
    const url = new URL(address);
    if (!["127.0.0.1", "localhost", "[::1]"].includes(url.hostname) || url.username || url.password || url.search)
        throw new Error("Use an isolated loopback browser fixture");
}
if (!Number.isInteger(spec.seconds) || spec.seconds < 1 || spec.seconds > 120 || ![1,2].includes(spec.participants.length))
    throw new Error("Invalid bounded browser rehearsal");
for(const tones of [spec.sourceToneHz,spec.expectedToneHz])if(tones!==undefined&&(!Array.isArray(tones)
    ||tones.length!==spec.participants.length||tones.some(value=>!Number.isFinite(value)||value<100||value>3000)))
    throw new Error("Generated/expected audio requires an identifiable 100–3000 Hz tone per participant");
let browser;
let database;
const contexts = [];
const observedPages = [];
let cleanupPromise;
const cleanup=()=>cleanupPromise??=(async()=>{
    await Promise.allSettled(contexts.map(context=>context.close()));
    await browser?.close();
    database?.close();database=undefined;
    if(assets)await new Promise(resolve=>assets.close(resolve));
})();
process.once("SIGTERM",()=>{cleanup().finally(()=>process.exit(1));});
try {
    const databasePath=await fs.realpath(spec.controlDatabase),privateRoot=await fs.realpath(path.join(spec.root,".local"));
    if(!databasePath.startsWith(privateRoot+path.sep))throw Error("Control oracle requires an isolated private fixture database");
    database=new DatabaseSync(databasePath,{readOnly:true});database.exec("PRAGMA busy_timeout=1000");
    browser=spec.browser==="firefox"?
        await connectFirefoxBidi(process.env.CONNECT_PUPPETEER_MODULE,spec.debugPort):
        await chromium.connectOverCDP(`http://127.0.0.1:${spec.debugPort}`);
    const pages=[];
    const results=await Promise.all(spec.participants.map(async(configuration,index)=>{
        const context=await browser.newContext();contexts.push(context);
        const page=await context.newPage();pages[index]=page;
        await page.goto(spec.baseURL+"/examples/conversation-client/");
        return page.evaluate(async({configuration,index,probes})=>{
            const {BrowserClient,BrowserMediaPeer}=await import("/sdk/typescript/dist/browser.js");
            const {canonicalMediaClient}=await import("/examples/conversation-client/canonical-media.js");
            const check=(ok,message)=>{if(!ok)throw Error(message);};
            const probe=configuration.fallbackProbe,url=new URL(probe.url);
            check(typeof WebTransport==="function"&&url.protocol==="https:"&&["127.0.0.1","localhost","[::1]"].includes(url.hostname)&&!url.username&&!url.password&&!url.search&&Number.isInteger(probe.timeoutMs)&&probe.timeoutMs>=100&&probe.timeoutMs<=2000,"actual bounded WebTransport probe");
            const algorithm=name=>name==="EdDSA"?{name:"Ed25519"}:name==="ES256"?{name:"ECDSA",namedCurve:"P-256"}:null;
            const key=await crypto.subtle.importKey("jwk",configuration.signing.jwk,algorithm(configuration.signing.algorithm),false,["sign"]);
            const trust=await crypto.subtle.importKey("jwk",configuration.server.jwk,algorithm(configuration.server.algorithm),false,["verify"]);
            const start=performance.now();let opened,errorCode;
            try{opened=await BrowserClient.open({transport:"webtransport",url:probe.url,timeoutMs:probe.timeoutMs,credential:configuration.credential,identityId:configuration.identityId,device:configuration.device,signing:{algorithm:configuration.signing.algorithm,keyId:configuration.signing.keyId,key},server:{algorithm:configuration.server.algorithm,keyId:configuration.server.keyId,key:trust,notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt}});}catch(error){errorCode=error?.code??"unclassified";}finally{opened?.close();}
            check(!opened&&["timeout","transport"].includes(errorCode),"owned UDP blackhole refusal");
            const fallback={status:"refused",error_code:errorCode,seconds:(performance.now()-start)/1000,application_mutations_attempted:0,replayed_originals:[]};
            const control=await canonicalMediaClient(configuration);window.transitionControl=control;
            check(control.client.controlTransport==="websocket","actual selected canonical WSS control");
            for(const original of configuration.fallbackOriginals){
                const terminalType={"message.send":"ack","session.invite":"operation.updated","session.accept":"operation.updated"}[original.type];
                check(terminalType,"known original intent");
                const frames=await control.client.request(original.type,original.payload,original.scope,{timeoutMs:5000,terminalType});
                const status=await control.client.operationStatus(original.scope.cid,original.scope.operation_id,{timeoutMs:5000});
                check(status.state==="completed","original completed operation preserved");
                if(original.type==="message.send"){
                    const details=frames.find(f=>f.type==="ack")?.payload.details;
                    check(details?.kind==="accepted"&&details.data?.message_id===original.payload.msg_id&&details.data?.request_id===original.scope.operation_id&&details.data?.conversation_id===original.scope.cid,"original message receipt preserved");
                }
                fallback.replayed_originals.push({type:original.type,operation_id:original.scope.operation_id,state:status.state});
            }
            let migrationRefusal;
            try{const unexpected=new BrowserMediaPeer(control.client,{});unexpected.close();}catch(error){migrationRefusal=error?.code;}
            check(migrationRefusal==="unsupported-transport"&&!control.client.closed&&control.client.controlTransport==="websocket","WSS cannot silently become a WT datagram connection");
            return {fallback,unsupported_migration:migrationRefusal,transitions:[]};
        },{configuration,index,probes:spec.controlTransitions});
    }));
    // The wire status deliberately omits effects/expiry. Observe those only in
    // this read-only private journal oracle, never invent browser visibility.
    const snapshot=operation=>{
        if(!spec.controlTransitions.some(p=>p.scope.operation_id===operation))throw Error("Unknown original oracle operation");
        const rows=database.prepare("SELECT record_id,expires_at FROM operations WHERE operation_id=?").all(operation);
        if(rows.length!==1)throw Error("Exactly one original operation row required");
        const effects=database.prepare("SELECT COUNT(*) AS count FROM operation_effects WHERE operation_record_id=?").get(rows[0].record_id).count;
        if(effects!==0)throw Error("Unready fixture transition acquired an external effect");
        return {...rows[0],effects};
    };
    for(let n=0;n<spec.controlTransitions.length;n++){
        const original=spec.controlTransitions[n],scope=original.scope;
        const first=await pages[0].evaluate(async original=>{
            const c=window.transitionControl.client;
            await c.request(original.type,original.payload,original.scope,{timeoutMs:5000,terminalType:"operation.updated"});
            const state=await c.operationStatus(original.scope.cid,original.scope.operation_id,{timeoutMs:5000});
            if(typeof state.record_id!=="string"||state.command_type!==original.type||state.expected_revision!==0)throw Error("Original admitted transition scope/record required");
            if(original.terminal==="cancelled"&&state.state!=="accepted")throw Error("Unprepared consult must stay accepted, not media ready");
            return {record_id:state.record_id,expected_revision:state.expected_revision};
        },original);
        const before=snapshot(scope.operation_id);
        const result=await pages[0].evaluate(async({original,first,foreignCid})=>{
            const c=window.transitionControl.client,scope=original.scope,check=(ok,message)=>{if(!ok)throw Error(message);};
            if(original.terminal==="cancelled")await c.request("operation.cancel",{operation_id:scope.operation_id,reason:"bounded control-only fixture"},{...scope,operation_id:scope.operation_id+"-cancel",expected_revision:undefined},{timeoutMs:5000,terminalType:"operation.updated"});
            const until=performance.now()+5000;let status;
            for(;;){
                check(performance.now()<until,"original transition terminal deadline");
                status=await c.operationStatus(scope.cid,scope.operation_id,{timeoutMs:Math.max(1,until-performance.now())});
                if(status.state===original.terminal)break;
                await new Promise(r=>setTimeout(r,20));
            }
            if(original.problem)check(status.problem?.code===original.problem.code&&status.problem?.source===original.problem.source,"exact original preflight refusal");
            await c.request(original.type,original.payload,scope,{timeoutMs:5000,terminalType:"operation.updated"});
            const replay=await c.operationStatus(scope.cid,scope.operation_id,{timeoutMs:5000});
            check(replay.state===status.state&&replay.record_id===first.record_id&&replay.expected_revision===first.expected_revision,"replay retains original record, revision and terminal state");
            let changed=false,foreign=false;
            try{await c.request(original.type,original.payload,{...scope,expected_revision:9},{timeoutMs:5000,terminalType:"operation.updated"});}catch(error){changed=error?.code==="refused";}
            try{await c.operationStatus(foreignCid,scope.operation_id,{timeoutMs:5000});}catch(error){foreign=error?.code==="refused";}
            check(changed&&foreign&&!c.closed,"changed original revision and foreign resource assertion refused without replacing intent");
            return {type:original.type,operation_id:scope.operation_id,terminal_status:status,original_record_id:first.record_id,original_expected_revision:first.expected_revision,terminal_state:status.state,replay_state:replay.state,changed_revision_refused:changed,foreign_scope_refused:foreign};
        },{original,first,foreignCid:spec.controlTransitions[(n+1)%spec.controlTransitions.length].scope.cid});
        const after=snapshot(scope.operation_id);
        if(before.record_id!==after.record_id||before.expires_at!==after.expires_at)throw Error("Original expiry/record changed across replay/refusal");
        results[0].transitions.push({...result,original_expires_at:before.expires_at,oracle_effects:after.effects,expiry_unchanged:true});
    }
    await pages[0].evaluate(async terminal=>{
        const {client,options}=window.transitionControl;
        await client.request("session.end",{by_participant:options.participantId,reason_code:200,reason:"control-fixture-complete"},{cid:options.conversationId,sid:options.sessionId,operation_id:terminal.operationId,expected_revision:terminal.expectedRevision},{terminalType:"operation.updated",timeoutMs:5000});
        const status=await client.operationStatus(options.conversationId,terminal.operationId,{timeoutMs:5000});if(status.state!=="completed")throw Error("original Session end not completed");
    },spec.terminal);
    const closed=await Promise.all(pages.map(page=>page.evaluate(()=>{const c=window.transitionControl.client;c.close();return c.closed;})));
    if(!closed.every(Boolean))throw Error("owned control cleanup");
    await cleanup();
    process.stdout.write(JSON.stringify({status:"passed",scope:"Actual signed browser fallback control only; no capture, decoded media or live movement quality claim.",transition_scope:"signed control admission/refusal/cancellation/replay; no live movement media claim",browser:spec.browser,fallback:results.map(r=>r.fallback),unsupported_migration:results.map(r=>r.unsupported_migration),transitions:results[0].transitions,owned_control_cleanup:true})+"\n");
}catch(error){await cleanup().catch(()=>{});throw error;}
