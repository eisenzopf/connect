/** Explicit loopback fixture faults. WSS payloads are forwarded byte-for-byte;
 * TLS private material/operator authority never enters the browser page. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import https from 'node:https';
import dgram from 'node:dgram';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash,createPrivateKey,X509Certificate} from 'node:crypto';

const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function local(url,protocol) {
    const u=new URL(url);
    assert.equal(u.protocol,protocol);assert.ok(['127.0.0.1','localhost'].includes(u.hostname));
    assert.ok(!u.username&&!u.password&&!u.search&&!u.hash);return u;
}
export async function startTransportFaults(spec) {
    assert.equal(spec.transportRecovery,true);assert.equal(spec.controlOnly,true);
    const configuration=spec.transportFaults,origin=new URL(spec.baseURL).origin;
    assert.ok(configuration&&configuration.credentials.length===2);
    assert.ok(Number.isInteger(configuration.quicPort)&&configuration.quicPort>0&&configuration.quicPort<65536);
    const privateRoot=await fs.realpath(path.join(spec.root,'.local'));
    const read=async name=>{
        const file=await fs.realpath(name);assert.ok(file.startsWith(privateRoot+path.sep));
        const data=await fs.readFile(file);assert.ok(data.length>0&&data.length<=65536);return data;
    };
    const [certificate,keyDer,root]=await Promise.all([read(configuration.certificateDer),read(configuration.privateKeyDer),read(configuration.rootDer)]);
    const ca=new X509Certificate(root).toString();
    const key=createPrivateKey({key:keyDer,format:'der',type:'pkcs8'}).export({type:'pkcs8',format:'pem'});
    const {WebSocket,WebSocketServer}=await import(pathToFileURL(path.join(spec.root,'.local/gate-6/browser-bidi-tools/node_modules/ws/wrapper.mjs')).href);
    const upstream=local(spec.participants[0].url,'wss:');upstream.hostname='127.0.0.1';
    const pairs=new Set(),udpPairs=new Map();let offlineWs=false,offlineUdp=false,closed=false;
    const counts={message_commands:0,original_commands:0,dropped_replies:0,dropped_acks:0,cut_websockets:0,outage_upgrades:0,forwarded_udp_client:0,forwarded_udp_server:0,dropped_udp:0};
    const server=https.createServer({key,cert:new X509Certificate(certificate).toString(),requestTimeout:5000},(_q,r)=>{r.writeHead(404);r.end();});
    const sockets=new WebSocketServer({noServer:true,maxPayload:1048576,perMessageDeflate:false});
    server.on('upgrade',(request,socket,head)=>{
        if(offlineWs||request.url!=='/connect'||request.headers.origin!==origin||pairs.size>=8){
            if(offlineWs)counts.outage_upgrades++;
            socket.end('HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');return;
        }
        sockets.handleUpgrade(request,socket,head,front=>{
            const back=new WebSocket(upstream,{ca,servername:'localhost',origin,handshakeTimeout:3000,maxPayload:1048576,perMessageDeflate:false});
            const pair={front,back,held:undefined,queue:[],queuedBytes:0,frames:0};pairs.add(pair);
            const end=()=>{pairs.delete(pair);pair.queue=[];pair.queuedBytes=0;front.terminate();back.terminate();};
            front.on('error',end);back.on('error',end);front.on('close',end);back.on('close',end);
            const send=(peer,data)=>{if(peer.readyState===WebSocket.OPEN&&peer.bufferedAmount<=1048576)peer.send(data,{binary:false});else end();};
            front.on('message',(data,binary)=>{
                if(binary||data.length>1048576||++pair.frames>512)return end();
                let frame;try{frame=JSON.parse(data.toString('utf8'));}catch{return end();}
                if(frame.type==='message.send')counts.message_commands++;
                if(frame.type==='message.send'&&frame.operation_id==='browser-dropped-message-op'){
                    counts.original_commands++;assert.equal(counts.original_commands,1,'No duplicate original mutation permitted');pair.held=frame.id;
                }
                if(back.readyState===WebSocket.CONNECTING){
                    if(pair.queue.length>=16||pair.queuedBytes+data.length>1048576)return end();
                    pair.queue.push(data);pair.queuedBytes+=data.length;
                }else send(back,data);
            });
            back.on('open',()=>{for(const data of pair.queue)send(back,data);pair.queue=[];pair.queuedBytes=0;});
            back.on('message',(data,binary)=>{
                if(binary||data.length>1048576)return end();
                let frame;try{frame=JSON.parse(data.toString('utf8'));}catch{return end();}
                if(pair.held&&frame.in_reply_to===pair.held){counts.dropped_replies++;if(frame.type==='ack')counts.dropped_acks++;return;}
                send(front,data);
            });
        });
    });
    await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
    const udp=dgram.createSocket('udp4');
    udp.on('message',(data,remote)=>{
        if(remote.address!=='127.0.0.1'||closed)return;
        if(offlineUdp){counts.dropped_udp++;return;}
        const id=remote.address+':'+remote.port;let pair=udpPairs.get(id);
        if(!pair){
            if(udpPairs.size>=16)throw Error('Bounded QUIC fixture peer count');
            const back=dgram.createSocket('udp4');pair={back,ready:false,queue:[],remote};udpPairs.set(id,pair);
            back.on('error',()=>{});
            back.on('message',(reply,from)=>{
                if(from.address!=='127.0.0.1'||from.port!==configuration.quicPort||closed)return;
                if(offlineUdp){counts.dropped_udp++;return;}
                counts.forwarded_udp_server++;udp.send(reply,pair.remote.port,'127.0.0.1');
            });
            back.bind(0,'127.0.0.1',()=>{pair.ready=true;for(const bytes of pair.queue){counts.forwarded_udp_client++;back.send(bytes,configuration.quicPort,'127.0.0.1');}pair.queue=[];});
        }
        if(!pair.ready){if(pair.queue.length>=16)throw Error('Bounded QUIC fixture opening queue');pair.queue.push(data);}
        else{counts.forwarded_udp_client++;pair.back.send(data,configuration.quicPort,'127.0.0.1');}
    });
    await new Promise((resolve,reject)=>{udp.once('error',reject);udp.bind(0,'127.0.0.1',resolve);});
    return {
        wssUrl:`wss://127.0.0.1:${server.address().port}/connect`,wtUrl:`https://127.0.0.1:${udp.address().port}/connect`,
        certificateHash:[...createHash('sha256').update(certificate).digest()],
        counts,
        cutWss(){offlineWs=true;for(const pair of pairs){counts.cut_websockets++;pair.front.terminate();pair.back.terminate();}},
        resumeWss(){offlineWs=false;},
        setUdpOffline(value){offlineUdp=value;},
        async close(){
            if(closed)return;closed=true;
            for(const pair of pairs){pair.front.terminate();pair.back.terminate();}
            await Promise.all([new Promise(resolve=>sockets.close(resolve)),new Promise(resolve=>server.close(resolve)),new Promise(resolve=>udp.close(resolve)),...Array.from(udpPairs.values(),pair=>new Promise(resolve=>pair.back.close(resolve)))]);
            assert.equal(pairs.size,0);udpPairs.clear();
        },
    };
}

export async function exerciseTransportFaults({spec,page,proxy,database}) {
    const baselineMessages=proxy.counts.message_commands;
    const original=structuredClone(spec.participants[0].fallbackOriginals.find(f=>f.type==='message.send'));
    original.scope.operation_id='browser-dropped-message-op';original.payload.msg_id='browser-dropped-message';
    const reference={operationId:original.scope.operation_id,command:'send',scope:{conversationId:original.scope.cid}};
    const dropped=await page.evaluate(async({original,reference})=>{
        const control=window.transitionControl;let code;
        try{await control.client.request(original.type,original.payload,original.scope,{timeoutMs:300});}catch(error){code=error?.code;}
        if(code!=='timeout'||control.owner.status.state!=='disconnected')throw Error('Withheld actual reply must leave an uncertain disconnected owner');
        await control.owner.reconnect({preferredTransport:'wss-webrtc'});control.client=control.owner.client;
        const recovered=await control.owner.recoverOperation(reference);
        if(recovered.state!=='completed')throw Error('Original accepted Message not recovered');
        return {uncertain_code:code,state:recovered.state,record_id:recovered.record_id};
    },{original,reference});
    assert.equal(proxy.counts.original_commands,1);assert.equal(proxy.counts.dropped_acks,1);
    proxy.cutWss();
    const outage=await page.evaluate(async()=>{
        const control=window.transitionControl,until=performance.now()+2000;
        while(control.owner.status.state!=='disconnected'&&performance.now()<until)await new Promise(r=>setTimeout(r,10));
        if(control.owner.status.state!=='disconnected')throw Error('Actual WSS cut did not retire physical');
        let code;try{await control.owner.reconnect({timeoutMs:1800});}catch(error){code=error?.code;}
        if(!['transport','timeout'].includes(code)||control.owner.status.state!=='disconnected')throw Error('Offline WSS/WT attempts were not bounded');
        const epoch=control.owner.status.physicalEpoch,resolutions=control.resolutions();await new Promise(r=>setTimeout(r,100));
        if(control.owner.status.physicalEpoch!==epoch||control.resolutions()!==resolutions)throw Error('Unexpected automatic reconnect');
        return {code,physical_epoch:epoch,no_automatic_reconnect:true};
    });
    proxy.resumeWss();
    await page.evaluate(async reference=>{const c=window.transitionControl;await c.owner.reconnect();c.client=c.owner.client;if((await c.owner.recoverOperation(reference)).state!=='completed')throw Error('WSS recovery lost original status');},reference);
    const wt=await page.evaluate(async({url,wssUrl,reference,hash})=>{
        const {BrowserTransportClient}=await import('/sdk/typescript/dist/browser.js');const control=window.transitionControl;
        const owner=await BrowserTransportClient.connect({identityId:control.enrollment.identityId,deviceId:control.enrollment.device.id,endpoints:{webtransport:url,'wss-webrtc':wssUrl},timeoutMs:5000,attemptTimeoutMs:1000,resolveEnrollment:async()=>({...control.enrollment,serverCertificateHashes:[{algorithm:'sha-256',value:new Uint8Array(hash)}]})});
        window.faultWtOwner=owner;
        if(owner.status.selectedTransport!=='webtransport'||(await owner.recoverOperation(reference)).state!=='completed')throw Error('Actual QUIC proxy connection required');
        return {initial_path:owner.status.selectedTransport};
    },{url:proxy.wtUrl,wssUrl:proxy.wssUrl,reference,hash:proxy.certificateHash});
    proxy.setUdpOffline(true);
    const wtOutage=await page.evaluate(async reference=>{
        const owner=window.faultWtOwner,old=owner.client;let retired=0,code;old.onClose(()=>{retired++;});
        try{await owner.recoverOperation(reference,{timeoutMs:250});}catch(error){code=error?.code;}
        if(code!=='timeout'||!old.closed||retired!==1||owner.status.state!=='disconnected')throw Error('Established WT blackhole did not retire uncertain physical');
        await owner.reconnect();
        if(owner.status.selectedTransport!=='wss-webrtc'||owner.status.fallbackReason!=='preferred-transport-timeout'||(await owner.recoverOperation(reference)).state!=='completed')throw Error('Explicit WT outage recovery must observe original status over WSS');
        return {code,old_lifetime_callbacks:retired,recovered_path:owner.status.selectedTransport,fallback_reason:owner.status.fallbackReason};
    },reference);
    proxy.setUdpOffline(false);
    await page.evaluate(async reference=>{const owner=window.faultWtOwner;await owner.reconnect({preferredTransport:'webtransport'});if(owner.status.selectedTransport!=='webtransport'||(await owner.recoverOperation(reference)).state!=='completed')throw Error('WT restore did not preserve original status');owner.close();},reference);

    const expiring=spec.transportFaults.credentials.find(c=>c.kind==='expiring');
    assert.ok(Date.now()<expiring.expiresAt*1000-1000,'Must authenticate before real expiry');
    const positiveExpiry=await page.evaluate(async({token,reference})=>{
        const c=window.transitionControl;c.enrollment.credential=token;await c.owner.reconnect();c.client=c.owner.client;
        return (await c.owner.recoverOperation(reference)).state==='completed';
    },{token:expiring.token,reference});
    assert.equal(positiveExpiry,true);
    await delay(Math.max(0,expiring.expiresAt*1000-Date.now()+200));
    const expiry=await page.evaluate(async({valid,reference})=>{
        const c=window.transitionControl;let readCode,refusal;
        try{await c.owner.recoverOperation(reference,{timeoutMs:1000});}catch(error){readCode=error?.code;}
        if(!readCode)throw Error('Expired credential read was authorized');
        try{await c.owner.reconnect();}catch(error){refusal={code:error?.code,phase:error?.phase};}
        if(refusal?.code!=='refused'||refusal.phase!=='authentication')throw Error('Expired credential reconnect did not reauthorize');
        c.enrollment.credential=valid;await c.owner.reconnect();c.client=c.owner.client;
        if((await c.owner.recoverOperation(reference)).state!=='completed')throw Error('Current valid credential recovery failed');
        return {positive_before_expiry:true,post_expiry_read_code:readCode,reconnect_refused:true,refusal};
    },{valid:spec.participants[0].credential,reference});
    const expiryRow=database.prepare('SELECT revoked,expires_at FROM credentials WHERE id=?').get(expiring.id);
    assert.equal(expiryRow.revoked,0);assert.equal(expiryRow.expires_at,expiring.expiresAt);assert.ok(expiryRow.expires_at*1000<Date.now());

    const revocable=spec.transportFaults.credentials.find(c=>c.kind==='revocable');
    const positiveRevoke=await page.evaluate(async({token,reference})=>{const c=window.transitionControl;c.enrollment.credential=token;await c.owner.reconnect();c.client=c.owner.client;return (await c.owner.recoverOperation(reference)).state==='completed';},{token:revocable.token,reference});
    assert.equal(positiveRevoke,true);
    const operator=local(spec.transportFaults.operatorEndpoint,'http:');
    const response=await fetch(new URL(`/v2/credentials/${revocable.id}/revoke`,operator),{method:'POST',headers:{Authorization:`Bearer ${spec.transportFaults.operatorCredential}`,'Content-Type':'application/json'},body:JSON.stringify({type:'revoke_credential',operation_id:'browser-recovery-revoke',credential_id:revocable.id}),signal:AbortSignal.timeout(5000),redirect:'error'});
    assert.equal(response.status,200);const body=await response.text();assert.ok(body.length<=65536);
    const revocation=await page.evaluate(async({valid,reference})=>{
        const c=window.transitionControl;let readCode,refusal;
        try{await c.owner.recoverOperation(reference,{timeoutMs:1000});}catch(error){readCode=error?.code;}
        if(!readCode)throw Error('Revoked credential read was authorized');
        try{await c.owner.reconnect();}catch(error){refusal={code:error?.code,phase:error?.phase};}
        if(refusal?.code!=='refused'||refusal.phase!=='authentication')throw Error('Revoked credential reconnect did not reauthorize');
        c.enrollment.credential=valid;await c.owner.reconnect();c.client=c.owner.client;
        if((await c.owner.recoverOperation(reference)).state!=='completed')throw Error('Current valid authorization recovery failed');
        return {positive_before_revocation:true,post_revocation_read_code:readCode,reconnect_refused:true,refusal};
    },{valid:spec.participants[0].credential,reference});
    assert.equal(database.prepare('SELECT revoked FROM credentials WHERE id=?').get(revocable.id).revoked,1);
    assert.equal(proxy.counts.original_commands,1);
    assert.equal(proxy.counts.message_commands-baselineMessages,1);
    return {passed:true,scope:'Actual control-only faults; WSS platform CA trust and explicit short-lived WT certificate pin; no live movement/media recovery claim',new_message_commands:proxy.counts.message_commands-baselineMessages,dropped_ack:dropped,wss_outage:outage,wt_initial:wt,wt_outage:wtOutage,wt_restored:true,expiry,revocation,proxy:{...proxy.counts}};
}
