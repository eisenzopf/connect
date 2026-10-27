import test from 'node:test';
import assert from 'node:assert/strict';
import {CanonicalWebRtcSignaling} from '../typescript/dist/browser_webrtc_signaling.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserVoiceSession} from '../typescript/dist/browser.js';
import {canonicalJson,parseCanonicalJson} from '../typescript/dist/uctp_json.js';
import {signEnvelope,verifyEnvelope} from '../typescript/dist/uctp_signing.js';
const wait = ms => new Promise(resolve=>setTimeout(resolve,ms));
const sdp = 'v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=fixture-only\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:audio\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\n';
const options = () => ({conversationId:'conversation',sessionId:'session',connectionId:'connection',operationId:'original-offer',expectedRevision:0,participantId:'participant',capabilities:{audio:['opus']},streams:[{id:'audio-stream',kind:'audio',direction:'sendrecv',codec_preferences:['opus']}]});
const scope = () => ({cid:'conversation',sid:'session',connid:'connection',operation_id:'original-offer'});
const answer = (changes={}) => {
    const frame=envelope('connection.answer',{substrate:'websocket+webrtc',capabilities:{audio:['opus']},streams_answered:[{id:'audio-stream',kind:'audio',direction:'sendrecv',codec:{name:'opus'}}],substrate_setup:{kind:'websocket+webrtc',sdp}},scope());
    return Object.assign(frame,changes);
};
const ice = (n=1,changes={}) => Object.assign(envelope('connection.ice-candidate',{candidate:`candidate:${n} 1 udp 1 127.0.0.1 10000 typ host`,sdp_mid:'audio',sdp_m_line_index:0},scope()),changes);
class Peer extends EventTarget {
    iceGatheringState='complete';localDescription=null;remoteDescription=null;closed=0;offers=0;ice=[];activeIce=0;maximumIce=0;
    async createOffer(){this.offers++;return {type:'offer',sdp};}
    async setLocalDescription(value){this.localDescription={...value};}
    async setRemoteDescription(value){this.remoteDescription={...value};}
    async addIceCandidate(value){assert.ok(this.remoteDescription);this.activeIce++;this.maximumIce=Math.max(this.maximumIce,this.activeIce);await wait(1);this.ice.push(value);this.activeIce--;}
    close(){this.closed++;}
}
class Client {
    controlTransport="websocket";
    events=new Set();lifetimes=new Set();requests=[];closed=0;
    response=async()=>[answer()];
    onEvent(fn){this.events.add(fn);return()=>this.events.delete(fn);}
    onClose(fn){this.lifetimes.add(fn);return()=>this.lifetimes.delete(fn);}
    emit(frame){for(const fn of this.events)fn(frame);}
    end(error=new ConnectError('transport','fixture control loss')){for(const fn of [...this.lifetimes])fn(error);}
    async request(...args){this.requests.push(structuredClone(args.slice(0,3)));return this.response();}
}
const fixture=()=>{const peer=new Peer(),client=new Client(),input=options();return {peer,client,input,owner:new CanonicalWebRtcSignaling(client,peer,input)};};

test('canonical WebRTC emits one scoped original offer and snapshots intent before asynchronous IO',async()=>{
    const f=fixture();f.input.streams[0].id='mutated';f.input.capabilities.audio=[];
    await f.owner.offer();
    assert.equal(f.peer.offers,1);assert.equal(f.client.requests.length,1);
    const [type,payload,sentScope]=f.client.requests[0];
    assert.equal(type,'connection.offer');assert.deepEqual(sentScope,{...scope(),expected_revision:0});
    assert.equal(payload.substrate,'websocket+webrtc');assert.equal(payload.substrate_setup.sdp,sdp);
    assert.deepEqual(payload.capabilities,{audio:['opus']});assert.equal(payload.streams_offered[0].id,'audio-stream');
    assert.equal(f.owner.savedOffer.submissionAttempted,true);assert.equal(f.peer.remoteDescription.type,'answer');
    const saved=f.owner.savedOffer;saved.payload.substrate_setup.sdp='forged';saved.scope.operation_id='new';
    assert.equal(f.owner.savedOffer.payload.substrate_setup.sdp,sdp);assert.equal(f.owner.savedOffer.scope.operation_id,'original-offer');
    await assert.rejects(f.owner.offer(),{code:'conflict'});assert.equal(f.client.requests.length,1);
    f.owner.close();await f.owner.finished;assert.equal(f.peer.closed,1);assert.equal(f.client.events.size,0);assert.equal(f.client.lifetimes.size,0);assert.equal(f.client.closed,0);
});
test('complete ICE gathering precedes submission, including asynchronous gathering completion',async()=>{
    const f=fixture();f.peer.iceGatheringState='gathering';const result=f.owner.offer();
    await wait(5);assert.equal(f.client.requests.length,0);
    f.peer.iceGatheringState='complete';f.peer.localDescription.sdp=sdp+'a=end-of-candidates\r\n';f.peer.dispatchEvent(new Event('icegatheringstatechange'));
    await result;assert.match(f.client.requests[0][1].substrate_setup.sdp,/end-of-candidates/);f.owner.close();
});
test('ICE before answer waits, serializes after answer, and refuses foreign scope or operation',async()=>{
    const f=fixture();let release;f.client.response=()=>new Promise(r=>{release=r;});const result=f.owner.offer();
    while(!release)await wait(1);
    f.client.emit(ice(1,{connid:'foreign'}));f.client.emit(ice(2,{operation_id:'old-operation'}));f.client.emit(ice(3));f.client.emit(ice(4));
    assert.equal(f.peer.ice.length,0);release([ice(5),answer()]);await result;
    assert.deepEqual(f.peer.ice.map(i=>i.candidate.split(' ')[0]),['candidate:3','candidate:4','candidate:5']);assert.equal(f.peer.maximumIce,1);
    f.client.emit(ice(6));await wait(5);assert.equal(f.peer.ice.length,4);f.owner.close();
});
test('ICE lifetime budget is bounded even when the browser candidate method stalls',async()=>{
    const f=fixture();f.peer.addIceCandidate=()=>new Promise(()=>{});await f.owner.offer();
    for(let i=0;i<65;i++)f.client.emit(ice(i));
    await assert.rejects(f.owner.finished,{code:'backpressure'});assert.equal(f.peer.closed,1);assert.equal(f.client.events.size,0);
});
test('ICE byte budget and malformed m-line/line-injected candidate refuse without a second offer',async()=>{
    for(const payload of [
        {candidate:'candidate:1\r\na=evil',sdp_mid:'audio',sdp_m_line_index:0},
        {candidate:'candidate:1',sdp_mid:'audio',sdp_m_line_index:-1},
        {candidate:'candidate:1',sdp_mid:'audio',sdp_m_line_index:65536},
    ]){const f=fixture();await f.owner.offer();f.client.emit({...ice(),payload});await assert.rejects(f.owner.finished,{code:'protocol'});assert.equal(f.client.requests.length,1);}
    const f=fixture();f.peer.addIceCandidate=()=>new Promise(()=>{});await f.owner.offer();
    for(let i=0;i<17;i++)f.client.emit({...ice(),payload:{candidate:'x'.repeat(4096),sdp_mid:'audio',sdp_m_line_index:0}});
    await assert.rejects(f.owner.finished,{code:'backpressure'});
});
test('answer correlation, substrate, stream identity and SDP are checked before applying remote description',async()=>{
    for(const alter of [
        f=>{f.cid='foreign';},f=>{f.sid='foreign';},f=>{f.connid='foreign';},f=>{f.operation_id='foreign';},
        f=>{f.payload.substrate='webtransport';},f=>{f.payload.substrate_setup.kind='offer-ready';},
        f=>{f.payload.substrate_setup.sdp='invalid';},f=>{f.payload.substrate_setup.sdp='v=0\r\n'+'x'.repeat(512*1024);},
        f=>{f.payload.streams_answered[0].id='foreign-stream';},f=>{f.payload.streams_answered.push(f.payload.streams_answered[0]);},
        f=>{f.payload.streams_answered[0].kind='video';},
    ]){const f=fixture(),bad=answer();alter(bad);f.client.response=async()=>[bad];await assert.rejects(f.owner.offer(),{code:'protocol'});assert.equal(f.peer.remoteDescription,null);assert.equal(f.peer.closed,1);}
});
test('missing answer retains original uncertain operation with no status guess, retry or substrate change',async()=>{
    const f=fixture();f.client.response=async()=>[envelope('ack',{},scope())];
    await assert.rejects(f.owner.offer(),{code:'unresolved-operation'});
    assert.equal(f.owner.savedOffer.scope.operation_id,'original-offer');assert.equal(f.client.requests.length,1);assert.equal(f.peer.closed,1);
});
test('deadline and cancellation retire owned peer, listeners and gathering without an extra control command',async()=>{
    for(const phase of ['create','gather','remote','wire']){
        const f=fixture();if(phase==='create')f.peer.createOffer=()=>new Promise(()=>{});
        if(phase==='gather')f.peer.iceGatheringState='gathering';
        if(phase==='remote')f.peer.setRemoteDescription=()=>new Promise(()=>{});
        if(phase==='wire')f.client.response=()=>new Promise(()=>{});
        await assert.rejects(f.owner.offer({timeoutMs:10}),{code:'timeout'});assert.equal(f.peer.closed,1);assert.equal(f.client.events.size,0);assert.equal(f.client.lifetimes.size,0);assert.equal(f.client.closed,0);
        f.peer.iceGatheringState='complete';f.peer.dispatchEvent(new Event('icegatheringstatechange'));await wait(1);assert.equal(f.client.requests.length,phase==='create'||phase==='gather'?0:1);
    }
    const f=fixture(),abort=new AbortController();f.client.response=()=>new Promise(()=>{});const result=f.owner.offer({signal:abort.signal});await wait(1);abort.abort();await assert.rejects(result,{code:'cancelled'});assert.equal(f.client.requests.length,1);assert.equal(f.owner.savedOffer.submissionAttempted,true);
});
for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} WebRTC signaling uses the actual signed BrowserClient WSS envelope path`,async()=>{
    const parameters=algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
    const clientKeys=await crypto.subtle.generateKey(parameters,false,['sign','verify']),serverKeys=await crypto.subtle.generateKey(parameters,false,['sign','verify']);
    const signing={key:clientKeys.privateKey,keyId:'webrtc-client-key',algorithm},serverSigning={key:serverKeys.privateKey,keyId:'webrtc-server-key',algorithm};
    const trust=key=>({...key,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300});
    const clientTrust=trust({...signing,key:clientKeys.publicKey}),server=trust({...serverSigning,key:serverKeys.publicKey});
    const originals=['WebSocket','WebTransport','isSecureContext'].map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]);
    const sent=[],tasks=[];let socket;
    class Socket extends EventTarget{
        static CONNECTING=0;static OPEN=1;readyState=0;bufferedAmount=0;
        constructor(){super();socket=this;queueMicrotask(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));});}
        async emit(frame){this.dispatchEvent(new MessageEvent('message',{data:canonicalJson(await signEnvelope(frame,serverSigning))}));}
        send(text){const frame=parseCanonicalJson(new TextEncoder().encode(text));sent.push(frame);tasks.push((async()=>{
            if(frame.type!=='auth.hello')await verifyEnvelope(frame,clientTrust);
            const respond=(type,payload,scope={})=>this.emit(envelope(type,payload,{...scope,in_reply_to:frame.id}));
            if(frame.type==='auth.hello')await respond('auth.challenge',{accepted_methods:['bearer'],server_capabilities:{application_profiles:['connect/2']}});
            else if(frame.type==='auth.response')await respond('auth.session',{identity_id:'identity',participant_id:'participant',session_token:'synthetic',expires_at:new Date(Date.now()+60000).toISOString(),application_profile:'connect/2'});
            else if(frame.type==='connection.offer'){
                assert.equal(frame.payload.substrate,'websocket+webrtc');assert.equal(frame.operation_id,'original-offer');
                await respond('connection.ice-candidate',ice().payload,scope());await respond('connection.answer',answer().payload,scope());
            }else throw Error('unexpected extra control command');
        })());}
        close(){if(this.readyState===3)return;this.readyState=3;this.dispatchEvent(new Event('close'));}
    }
    let client,owner;
    try{
        Object.defineProperty(globalThis,'WebSocket',{configurable:true,value:Socket});Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:undefined});Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:true});
        client=await BrowserClient.open({transport:'websocket',url:'wss://fixture.invalid/connect',credential:'fixture-only',identityId:'identity',device:{id:'device',kind:'browser',platform:'fixture'},signing,server,timeoutMs:500});
        const peer=new Peer();owner=new CanonicalWebRtcSignaling(client,peer,options());await owner.offer();
        assert.equal(peer.ice.length,1);assert.equal(peer.remoteDescription.sdp,sdp);assert.equal(sent.filter(f=>f.type==='connection.offer').length,1);
        assert.ok(sent.every(f=>!['offer-ready','answer','ready'].includes(f.type)));assert.ok(sent.find(f=>f.type==='connection.offer').signature);
        owner.close();assert.equal(client.closed,false);client.close();await Promise.all(tasks);
    }finally{owner?.close();client?.close();socket?.close();for(const[k,d]of originals)if(d)Object.defineProperty(globalThis,k,d);else delete globalThis[k];}
});
test('retained terminal event and current control lifetime loss retire only the correct Connection',async()=>{
    const f=fixture();await f.owner.offer();f.client.emit(envelope('session.ended',{}, {cid:'foreign',sid:'session'}));assert.equal(f.peer.closed,0);
    f.client.emit(envelope('connection.end',{reason:'normal'},scope()));await f.owner.finished;assert.equal(f.peer.closed,1);
    const g=fixture();await g.owner.offer();g.client.end();await assert.rejects(g.owner.finished,{code:'transport'});assert.equal(g.peer.closed,1);
    const h=fixture();h.client.onEvent=fn=>{h.client.events.add(fn);fn(envelope('conversation.closed',{}, {cid:'conversation'}));return()=>h.client.events.delete(fn);};
    await assert.rejects(h.owner.offer(),{code:'closed'});assert.equal(h.client.events.size,0);assert.equal(h.client.lifetimes.size,0);assert.equal(h.peer.offers,0);
});
test('validation precedes ownership transfer and invalid timeout does not consume the original offer',async()=>{
    const peer=new Peer(),client=new Client();assert.throws(()=>new CanonicalWebRtcSignaling(client,peer,{...options(),operationId:'bad id'}),{code:'invalid-request'});assert.equal(peer.closed,0);
    const f=fixture();await assert.rejects(f.owner.offer({timeoutMs:0}),{code:'invalid-request'});await f.owner.offer();assert.equal(f.client.requests.length,1);f.owner.close();
});
test('peer-state failure retires the signaling owner while preserving its original intent',async()=>{
    const f=fixture();await f.owner.offer();
    f.peer.connectionState='failed';f.peer.dispatchEvent(new Event('connectionstatechange'));
    await assert.rejects(f.owner.finished,{code:'webrtc-failed'});
    assert.equal(f.peer.closed,1);assert.equal(f.client.events.size,0);assert.equal(f.client.lifetimes.size,0);
    assert.equal(f.client.requests.length,1);assert.equal(f.owner.savedOffer.scope.operation_id,'original-offer');assert.equal(f.owner.savedOffer.submissionAttempted,true);
    f.owner.close();assert.equal(f.peer.closed,1);assert.equal(f.client.closed,0);
});
test('failed or closed supplied peer refuses before offer creation or control submission',async()=>{
    for(const [state,code] of [['failed','webrtc-failed'],['closed','closed']]){
        const f=fixture();f.peer.connectionState=state;
        await assert.rejects(f.owner.offer(),{code});assert.equal(f.peer.offers,0);assert.equal(f.client.requests.length,0);assert.equal(f.peer.closed,1);
        assert.equal(f.client.events.size,0);assert.equal(f.client.lifetimes.size,0);
    }
});
test('terminal retirement removes peer-state callbacks and late failure cannot change completion',async()=>{
    const f=fixture();await f.owner.offer();f.owner.close();await f.owner.finished;
    f.peer.connectionState='failed';f.peer.dispatchEvent(new Event('connectionstatechange'));await f.owner.finished;
    assert.equal(f.peer.closed,1);assert.equal(f.client.requests.length,1);
});

test('WebRTC refuses a different or retired physical control before peer ownership',()=>{
    for(const transport of ['webtransport',undefined]){
        const client=new Client(),peer=new Peer();client.controlTransport=transport;
        assert.throws(()=>new CanonicalWebRtcSignaling(client,peer,options()),{code:'unsupported-transport'});
        assert.equal(peer.closed,0);assert.equal(peer.offers,0);assert.equal(client.requests.length,0);
    }
    const client=new Client(),peer=new Peer();client.closed=true;
    assert.throws(()=>new CanonicalWebRtcSignaling(client,peer,options()),{code:'closed'});
    assert.equal(peer.closed,0);assert.equal(client.events.size,0);assert.equal(client.lifetimes.size,0);
});
test('ICE failure retires the original signaling owner even before aggregate peer failure',async()=>{
    const f=fixture();await f.owner.offer();f.peer.connectionState='connected';
    f.peer.iceConnectionState='failed';f.peer.dispatchEvent(new Event('iceconnectionstatechange'));
    await assert.rejects(f.owner.finished,{code:'webrtc-failed'});assert.equal(f.peer.closed,1);
    assert.equal(f.client.events.size,0);assert.equal(f.client.lifetimes.size,0);assert.equal(f.client.closed,0);
    assert.equal(f.owner.savedOffer.scope.operation_id,'original-offer');assert.equal(f.client.requests.length,1);
    f.peer.dispatchEvent(new Event('iceconnectionstatechange'));assert.equal(f.peer.closed,1);
});

for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} public capture wrapper uses actual signed WSS offers and journaled end`,async()=>{
    const parameters=algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
    const clientKeys=await crypto.subtle.generateKey(parameters,false,['sign','verify']),serverKeys=await crypto.subtle.generateKey(parameters,false,['sign','verify']);
    const signing={key:clientKeys.privateKey,keyId:'webrtc-client-key',algorithm},serverSigning={key:serverKeys.privateKey,keyId:'webrtc-server-key',algorithm};
    const trust=key=>({...key,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300});
    const clientTrust=trust({...signing,key:clientKeys.publicKey}),server=trust({...serverSigning,key:serverKeys.publicKey});
    const originals=['WebSocket','WebTransport','isSecureContext','RTCPeerConnection','MediaStream'].map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]);
    const sent=[],tasks=[];let socket;
    class Socket extends EventTarget{
        static CONNECTING=0;static OPEN=1;readyState=0;bufferedAmount=0;
        constructor(){super();socket=this;queueMicrotask(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));});}
        async emit(frame){this.dispatchEvent(new MessageEvent('message',{data:canonicalJson(await signEnvelope(frame,serverSigning))}));}
        send(text){const frame=parseCanonicalJson(new TextEncoder().encode(text));sent.push(frame);tasks.push((async()=>{
            if(frame.type!=='auth.hello')await verifyEnvelope(frame,clientTrust);
            const respond=(type,payload,scope={})=>this.emit(envelope(type,payload,{...scope,in_reply_to:frame.id}));
            if(frame.type==='auth.hello')await respond('auth.challenge',{accepted_methods:['bearer'],server_capabilities:{application_profiles:['connect/2']}});
            else if(frame.type==='auth.response')await respond('auth.session',{identity_id:'identity',participant_id:'participant',session_token:'synthetic',expires_at:new Date(Date.now()+60000).toISOString(),application_profile:'connect/2'});
            else if(frame.type==='connection.offer'){
                assert.equal(frame.payload.substrate,'websocket+webrtc');assert.equal(frame.operation_id,'original-offer');
                await respond('connection.ice-candidate',ice().payload,scope());await respond('connection.answer',answer().payload,scope());
            }else if(frame.type==='connection.end')await respond('ack',{}, {...scope(),operation_id:'original-end'});
            else if(frame.type==='operation.get'){
                assert.equal(frame.payload.operation_id,'original-end');
                await respond('operation.updated',{operation_id:'original-end',state:'completed'},{cid:'conversation'});await respond('ack',{});
            }else throw Error('unexpected extra control command');
        })());}
        close(){if(this.readyState===3)return;this.readyState=3;this.dispatchEvent(new Event('close'));}
    }
    let client,owner;
    try{
        Object.defineProperty(globalThis,'WebSocket',{configurable:true,value:Socket});Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:undefined});Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:true});
        client=await BrowserClient.open({transport:'websocket',url:'wss://fixture.invalid/connect',credential:'fixture-only',identityId:'identity',device:{id:'device',kind:'browser',platform:'fixture'},signing,server,timeoutMs:500});
        const peer=new Peer(),receiver={jitterBufferTarget:null};peer.addTrack=()=>{};
        peer.getTransceivers=()=>[{receiver}];
        Object.defineProperty(globalThis,'RTCPeerConnection',{configurable:true,value:class{constructor(){return peer;}}});
        Object.defineProperty(globalThis,'MediaStream',{configurable:true,value:class{tracks=[];getTracks(){return this.tracks;}addTrack(track){this.tracks.push(track);}}});
        let trackStops=0;const track=Object.assign(new EventTarget(),{kind:'audio',readyState:'live',stop(){trackStops++;}});
        owner=await BrowserVoiceSession.open(client,{...options(),audioStreamId:'audio-stream',inputStream:{getTracks:()=>[track]}},{timeoutMs:500});
        assert.equal(receiver.jitterBufferTarget,80);assert.equal(peer.ice.length,1);assert.equal(peer.remoteDescription.sdp,sdp);assert.equal(sent.filter(f=>f.type==='connection.offer').length,1);
        assert.ok(sent.every(f=>!['offer-ready','answer','ready'].includes(f.type)));assert.ok(sent.find(f=>f.type==='connection.offer').signature);
        assert.equal(owner.savedOffer.scope.operation_id,'original-offer');
        const ending=owner.end('original-end',1);assert.equal(ending,owner.end('original-end',1));await ending;
        assert.equal(sent.filter(f=>f.type==='connection.end').length,1);assert.equal(sent.filter(f=>f.type==='operation.get').length,1);
        assert.equal(owner.savedEnd.scope.expected_revision,1);assert.equal(trackStops,1);assert.equal(peer.closed,1);
        await owner.close();assert.equal(client.closed,false);client.close();await Promise.all(tasks);
    }finally{await owner?.close();client?.close();socket?.close();for(const[k,d]of originals)if(d)Object.defineProperty(globalThis,k,d);else delete globalThis[k];}
});

test('canonical answer cannot silently omit an original logical video stream',async()=>{
    const input=options(),client=new Client(),peer=new Peer();
    input.streams.push({id:'video-stream',kind:'video',direction:'sendrecv',codec_preferences:['vp8']});
    const owner=new CanonicalWebRtcSignaling(client,peer,input);
    await assert.rejects(owner.offer(),{code:'protocol'});
    assert.equal(peer.remoteDescription,null);assert.equal(peer.closed,1);assert.equal(client.requests.length,1);
    assert.equal(owner.savedOffer.payload.streams_offered.length,2);assert.equal(owner.savedOffer.scope.operation_id,'original-offer');
});
