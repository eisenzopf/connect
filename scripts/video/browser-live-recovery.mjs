/** Bounded stdin/stdout fixture controller. Actual browser owner, signed WT
 * control and independent live native media; no mutation is submitted here. */
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import readline from 'node:readline';
import {pathToFileURL} from 'node:url';
import {createPrivateKey,webcrypto} from 'node:crypto';
import {connectFirefoxBidi} from './browser_bidi_adapter.mjs';

const lines=readline.createInterface({input:process.stdin,crlfDelay:Infinity});
const iterator=lines[Symbol.asyncIterator]();
async function next(){const item=await iterator.next();if(item.done)throw Error('Controller input ended');if(Buffer.byteLength(item.value)>65536)throw Error('Controller input bound');return JSON.parse(item.value);}
const reply=value=>{const line=JSON.stringify(value);if(Buffer.byteLength(line)>65536)throw Error('Controller output bound');process.stdout.write(line+'\n');};
const check=(value,message)=>{if(!value)throw Error(message);};
let browser,assets,page,closed=false;
const cleanup=async()=>{
    if(closed)return;closed=true;
    await browser?.close();
    if(assets)await new Promise(resolve=>assets.close(resolve));
};
process.once('SIGTERM',()=>{cleanup().finally(()=>process.exit(1));});
try{
    const spec=await next();check(['chrome','firefox'].includes(spec.browser),'designated installed browser');
    for(const p of [spec.assetPort,spec.debugPort])check(Number.isInteger(p)&&p>0&&p<65536,'bounded local port');
    const contact=new URL(spec.url);check(contact.protocol==='https:'&&['127.0.0.1','localhost'].includes(contact.hostname)&&!contact.username&&!contact.password&&!contact.search&&!contact.hash,'owned loopback WT');
    const root=await fs.realpath(spec.root),sdk=await fs.realpath(path.join(root,'sdk/typescript/dist'));
    assets=http.createServer(async(request,response)=>{
        try{
            check(request.method==='GET','GET assets only');const url=new URL(request.url,'http://127.0.0.1');
            if(url.pathname==='/'){response.writeHead(200,{'Content-Type':'text/html'});response.end('<!doctype html><meta charset=utf-8><title>Local signed recovery fixture</title>');return;}
            check(url.pathname.startsWith('/sdk/')&&!url.search,'public SDK assets only');
            const file=await fs.realpath(path.resolve(sdk,decodeURIComponent(url.pathname.slice(5))));
            check(file.startsWith(sdk+path.sep)&&file.endsWith('.js'),'bounded public SDK path');
            response.writeHead(200,{'Content-Type':'text/javascript'});response.end(await fs.readFile(file));
        }catch{response.writeHead(404);response.end('asset unavailable');}
    });
    await new Promise((resolve,reject)=>{assets.once('error',reject);assets.listen(spec.assetPort,'127.0.0.1',resolve);});
    const parameters=name=>name==='EdDSA'?{name:'Ed25519'}:name==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:undefined;
    spec.signing.jwk=createPrivateKey({key:Buffer.from(spec.signing.privatePkcs8,'base64url'),format:'der',type:'pkcs8'}).export({format:'jwk'});delete spec.signing.privatePkcs8;
    const trust=await webcrypto.subtle.importKey('raw',Buffer.from(spec.server.publicRaw,'base64url'),parameters(spec.server.algorithm),true,['verify']);
    spec.server.jwk=await webcrypto.subtle.exportKey('jwk',trust);delete spec.server.publicRaw;
    if(spec.browser==='firefox')browser=await connectFirefoxBidi(process.env.CONNECT_PUPPETEER_MODULE,spec.debugPort);
    else{
        const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
        browser=await chromium.connectOverCDP(`http://127.0.0.1:${spec.debugPort}`);
    }
    const context=await browser.newContext();page=await context.newPage();await page.goto(`http://127.0.0.1:${spec.assetPort}/`);
    await page.evaluate(async configuration=>{
        const {BrowserTransportClient}=await import('/sdk/browser.js');
        const algorithm=name=>name==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
        const key=await crypto.subtle.importKey('jwk',configuration.signing.jwk,algorithm(configuration.signing.algorithm),false,['sign']);
        const trust=await crypto.subtle.importKey('jwk',configuration.server.jwk,algorithm(configuration.server.algorithm),false,['verify']);
        window.liveRecovery={BrowserTransportClient,owner:undefined,cursors:new Map(),enrollment:{identityId:configuration.identityId,device:configuration.device,credential:'',signing:{key,keyId:configuration.signing.keyId,algorithm:configuration.signing.algorithm},server:{key:trust,keyId:configuration.server.keyId,algorithm:configuration.server.algorithm,notBefore:configuration.server.notBefore,expiresAt:configuration.server.expiresAt},serverCertificateHashes:[{algorithm:'sha-256',value:new Uint8Array(configuration.certificateHash)}]},endpoint:configuration.url};
    },spec);
    reply({ready:true});let phases=0;
    for(;;){
        const command=await next();
        if(command.mode==='close'){
            await page.evaluate(()=>{const c=window.liveRecovery;c.owner?.close();if(c.owner&&c.owner.status.state!=='closed')throw Error('Owner cleanup');});
            await cleanup();reply({closed:true});break;
        }
        if(command.mode==='observe'){
            check(phases===2,'completion follows both live recovery phases');
            const result=await page.evaluate(async reference=>{
                const c=window.liveRecovery,operation=await c.owner.recoverOperation(reference);
                if(operation.state!=='completed')throw Error('Original handoff not complete');
                const journal=await c.owner.recoverConversation({conversationId:reference.scope.conversationId,cursor:c.cursors.get(reference.scope.conversationId),limit:32});
                return {passed:true,operation_id:operation.operation_id,record_id:operation.record_id,state:operation.state,joined_reopens:0,completion_observed:true,event_count:journal.frames.length,selected_transport:c.owner.status.selectedTransport};
            },command.reference);reply(result);continue;
        }
        check(command.mode==='recover'&&++phases<=2,'two explicit live reference phases only');
        check(['accepted','completed'].includes(command.expectedState),'known observation state');
        const result=await page.evaluate(async command=>{
            const c=window.liveRecovery,check=(ok,message)=>{if(!ok)throw Error(message);};
            c.enrollment.credential=command.credential;
            if(!c.owner)c.owner=await c.BrowserTransportClient.connect({identityId:c.enrollment.identityId,deviceId:c.enrollment.device.id,endpoints:{webtransport:c.endpoint},allowFallback:false,timeoutMs:5000,attemptTimeoutMs:3000,resolveEnrollment:async()=>c.enrollment});
            else await c.owner.reconnect();
            check(c.owner.status.selectedTransport==='webtransport','actual WT control selected');
            const reference=command.reference,cid=reference.scope.conversationId;
            if(!c.cursors.has(cid)){
                const initial=await c.owner.recoverConversation({conversationId:cid,limit:32});check(initial.kind==='snapshot','initial current authorized snapshot');c.cursors.set(cid,initial.cursor);
            }
            const original=await c.owner.recoverOperation(reference),rounds=[];
            check(original.state===command.expectedState,'original live operation state');
            for(let round=0;round<3;round++){
                const old=c.owner.client,epoch=c.owner.status.physicalEpoch;let retired=0;old.onClose(()=>{retired++;});
                await c.owner.reconnect();const observed=await c.owner.recoverOperation(reference);
                check(old.closed&&retired===1&&c.owner.status.physicalEpoch===epoch+1,'old physical lifetime retired once');
                for(const key of ['operation_id','record_id','command_type','state','revision','expected_revision'])check(observed[key]===original[key],'original record/precondition unchanged');
                check(observed.state===command.expectedState,'live state not inferred');
                const journal=await c.owner.recoverConversation({conversationId:cid,cursor:c.cursors.get(cid),limit:32});
                check(journal.kind==='events'&&journal.frames.every(f=>f.type==='conversation.event'),'finite authoritative journal recovery');c.cursors.set(cid,journal.cursor);
                rounds.push({physical_epoch:c.owner.status.physicalEpoch,selected_transport:c.owner.status.selectedTransport,retired_callbacks:retired,event_count:journal.frames.length,has_more:journal.hasMore});
                await new Promise(resolve=>setTimeout(resolve,100));
            }
            return {passed:true,operation_id:original.operation_id,record_id:original.record_id,state:original.state,command:original.command_type,expected_revision:original.expected_revision,joined_reopens:rounds.length,rounds,control_device:c.enrollment.device.id,mutations_submitted:0};
        },command);
        reply(result);
    }
}catch(error){await cleanup().catch(()=>{});throw error;}finally{lines.close();}
