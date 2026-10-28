import assert from 'node:assert/strict';
import { test } from 'node:test';
import { BrowserVoiceSession, BrowserVideoSession, BrowserVoiceOpenError } from '../typescript/dist/browser.js';
import { validateVideoAnswer } from '../typescript/dist/browser_capture.js';
import { constrainVideoCapture } from '../typescript/dist/browser_video_setup.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
import {ConnectError} from '../typescript/dist/errors.js';
const audioSdp='v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=synthetic-fixture\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:audio\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\n';
const videoSdp='m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:video\r\na=sendrecv\r\na=rtpmap:96 VP8/90000\r\n';
const intent=(options={})=>({conversationId:'conversation',sessionId:'session',connectionId:'connection',operationId:'original-offer',expectedRevision:0,participantId:'participant',audioStreamId:'audio-stream',...(options.video?{videoStreamId:'video-stream'}:{}),...options});
const open=(state,options={},timeoutMs=100)=>BrowserVoiceSession.open(state.client,intent(options),{timeoutMs});
const tick=()=>new Promise(resolve=>setImmediate(resolve));

// Synthetic capture/RTC boundary; actual signed control is covered separately.
async function withBrowser(mode,run){
    const names=['navigator','MediaStream','RTCPeerConnection','WebSocket'];
    const original=new Map(names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
    const state={offers:[],trackStopped:false,trackStops:0,peerClosed:false,peerCloses:0,microphoneRequests:0};
    const track=Object.assign(new EventTarget(),{kind:'audio',readyState:'live',stop:()=>{state.trackStopped=true;state.trackStops++;}});
    const stream={getTracks:()=>[track]};state.track=track;state.stream=stream;
    class Peer extends EventTarget {
        iceGatheringState='complete';connectionState='new';
        transceivers=[];
        constructor(configuration){super();state.peerConfiguration=structuredClone(configuration);state.peer=this;if(mode==='peer-constructor')throw Error('fixture peer failure');}
        getConfiguration(){return mode==='relay-ignored'?{iceTransportPolicy:'all'}:structuredClone(state.peerConfiguration??{});}
        addTrack(track){if(mode==='track-add')throw Error('fixture track failure');
            const receiver={track:{kind:track.kind},jitterBufferTarget:null};
            if(mode==='receiver-unsupported')delete receiver.jitterBufferTarget;
            if(mode==='receiver-ignored')Object.defineProperty(receiver,'jitterBufferTarget',{get:()=>null,set(){}});
            if(mode==='receiver-throws')Object.defineProperty(receiver,'jitterBufferTarget',{get:()=>null,set(){throw Error('fixture native refusal');}});
            this.transceivers.push({receiver});}
        getTransceivers(){return this.transceivers;}
        createDataChannel(){throw Error('Unoffered datachannel must never be created');}
        async createOffer(){assert.ok(this.getTransceivers().every(t=>t.receiver.jitterBufferTarget===80));if(mode==='offer-stall')return new Promise(()=>{});await Promise.resolve();return {type:'offer',sdp:audioSdp};}
        async setLocalDescription(offer){this.localDescription=offer;}
        async setRemoteDescription(answer){state.answer=answer;this.remoteDescription=answer;}
        close(){state.peerClosed=true;state.peerCloses++;}
    }
    class Control {
        controlTransport='websocket';closed=false;events=new Set();lifetimes=new Set();commands=[];
        onClose(fn){if(mode==='closed'){fn(new ConnectError('transport','fixture control closed'));return()=>{};}this.lifetimes.add(fn);return()=>this.lifetimes.delete(fn);}
        onEvent(fn){if(mode==='signaling-listener')throw Error('fixture listener failure');this.events.add(fn);return()=>this.events.delete(fn);}
        emit(frame){for(const fn of [...this.events])fn(frame);}
        lose(){this.closed=true;for(const fn of [...this.lifetimes])fn(new ConnectError('transport','fixture control loss'));}
        async request(type,payload,scope){
            this.commands.push({type,payload:structuredClone(payload),scope:{...scope}});
            if(type==='connection.end')return [envelope('ack',{},scope)];
            assert.equal(type,'connection.offer');state.offers.push({type,payload:structuredClone(payload),scope:{...scope}});
            if(mode==='late-send-error')throw Error('fixture send failure');
            if(mode==='stalled')return new Promise(()=>{});
            if(mode==='late')await new Promise(resolve=>setTimeout(resolve,5));
            const video=payload.streams_offered.some(s=>s.kind==='video');
            return [envelope('connection.answer',{substrate:'websocket+webrtc',capabilities:payload.capabilities,
                streams_answered:payload.streams_offered.map(s=>({...s,codec:{name:s.kind==='audio'?'opus':'vp8'}})),
                substrate_setup:{kind:'websocket+webrtc',sdp:audioSdp+(video?videoSdp:'')}},scope)];
        }
        async operationStatus(cid,operationId){return {operation_id:operationId,state:'completed'};}
        close(){throw Error('Borrowed control must not be closed by media');}
    }
    state.client=new Control();
    const replacements={navigator:{mediaDevices:{getUserMedia:async constraints=>{state.constraints=constraints;state.microphoneRequests++;if(mode==='denied')throw Error('fixture permission denied');return stream;}}},
        MediaStream:class {tracks=[];getTracks(){return [...this.tracks];}addTrack(track){this.tracks.push(track);}},RTCPeerConnection:Peer,
        WebSocket:class {constructor(){throw Error('Wrapper must borrow signed control');}}};
    for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:replacements[name]});
    try{await run(state);}finally{for(const name of names){const d=original.get(name);if(d)Object.defineProperty(globalThis,name,d);else delete globalThis[name];}}
}
for(const mode of ['early','late'])test(`browser uses exactly one canonical original offer with ${mode} reply`,()=>withBrowser(mode,async state=>{
    const voice=await open(state);assert.equal(state.offers.length,1);const offer=state.offers[0];
    assert.equal(offer.type,'connection.offer');assert.equal(offer.scope.operation_id,'original-offer');assert.equal(offer.scope.connid,'connection');
    assert.equal(offer.payload.by_participant,'participant');assert.equal(offer.payload.streams_offered[0].id,'audio-stream');
    assert.equal(state.answer.type,'answer');assert.equal(voice.savedOffer.submissionAttempted,true);
    await voice.close();await voice.ended;assert.ok(state.trackStopped&&state.peerClosed&&!state.client.closed);
    assert.equal(state.peerCloses,1);assert.equal(state.trackStops,1);assert.equal(state.client.commands.length,1);
    assert.equal(state.client.events.size,0);assert.equal(state.client.lifetimes.size,0);
}));
test('control closure before capture refuses promptly without permission or offer',()=>withBrowser('closed',async state=>{
    await assert.rejects(open(state),{code:'transport'});assert.equal(state.microphoneRequests,0);assert.equal(state.offers.length,0);
}));
test('uncertain signaling timeout retains original SDP and releases only owned resources',()=>withBrowser('stalled',async state=>{
    await assert.rejects(open(state,{},10),error=>error instanceof BrowserVoiceOpenError&&error.code==='timeout'&&error.savedOffer.scope.operation_id==='original-offer'&&error.savedOffer.submissionAttempted);
    assert.ok(state.trackStopped&&state.peerClosed&&!state.client.closed);assert.equal(state.client.commands.length,1);
}));
test('server end with another operation retires only the selected Connection and owned microphone',()=>withBrowser('early',async state=>{
    const voice=await open(state);state.client.emit(envelope('connection.end',{}, {cid:'foreign',sid:'session',connid:'connection'}));assert.equal(state.trackStopped,false);
    state.client.emit(envelope('connection.end',{}, {cid:'conversation',sid:'session',connid:'connection',operation_id:'another-end'}));await voice.ended;
    assert.ok(state.trackStopped&&state.peerClosed&&!state.client.closed);assert.equal(state.client.commands.length,1);
}));
test('application audio uses canonical signaling without requesting a microphone',()=>withBrowser('early',async state=>{
    const voice=await open(state,{inputStream:state.stream});assert.equal(state.microphoneRequests,0);assert.equal(state.offers.length,1);await voice.close();assert.equal(state.trackStops,1);
}));
test('invalid supplied capture refuses before ownership or signaling',()=>withBrowser('early',async state=>{
    for(const tracks of [[],[{kind:'audio',readyState:'ended'}],[{kind:'video',readyState:'live'}],[{kind:'audio',readyState:'live'},{kind:'other',readyState:'live'}]])
        await assert.rejects(open(state,{inputStream:{getTracks:()=>tracks}}),{code:'invalid-request'});
    assert.equal(state.microphoneRequests,0);assert.equal(state.offers.length,0);assert.equal(state.trackStops,0);
}));
for(const mode of ['peer-constructor','track-add'])test(`browser releases original capture after ${mode} failure`,()=>withBrowser(mode,async state=>{
    await assert.rejects(open(state),{code:'webrtc-failed'});assert.equal(state.microphoneRequests,1);assert.equal(state.trackStops,1);
    if(mode!=='peer-constructor')assert.equal(state.peerCloses,1);assert.equal(state.offers.length,0);
}));
for(const mode of ['receiver-unsupported','receiver-ignored','receiver-throws'])test(`native receiver ${mode} refuses before offer and releases owned resources`,()=>withBrowser(mode,async state=>{
    await assert.rejects(open(state),error=>error instanceof BrowserVoiceOpenError&&error.code==='unsupported-media-profile'&&error.savedOffer===undefined);
    assert.equal(state.offers.length,0);assert.equal(state.client.commands.length,0);
    assert.equal(state.trackStops,1);assert.equal(state.peerCloses,1);
    assert.equal(state.client.closed,false);assert.equal(state.client.events.size,0);assert.equal(state.client.lifetimes.size,0);
}));
test('media local close is shared and sends no end while retaining borrowed control',()=>withBrowser('early',async state=>{
    const voice=await open(state);let terminated=0;voice.videoWorker={terminate(){terminated++;}};
    const first=voice.close();assert.equal(first,voice.close());await first;await voice.ended;
    assert.equal(terminated,1);assert.equal(state.peerCloses,1);assert.equal(state.trackStops,1);assert.equal(state.client.commands.length,1);
}));
test('capture denial remains explicit before creating a peer or submitting an offer',()=>withBrowser('denied',async state=>{
    await assert.rejects(open(state),{code:'media-denied'});assert.equal(state.offers.length,0);assert.equal(state.peer,undefined);assert.equal(state.client.lifetimes.size,0);
}));
test('capture deadline stops a late permission result without an offer',()=>withBrowser('early',async state=>{
    let finish;const pending=new Promise(resolve=>finish=resolve);navigator.mediaDevices.getUserMedia=()=>pending;
    await assert.rejects(open(state,{},5),{code:'timeout'});finish(state.stream);await tick();assert.equal(state.trackStops,1);assert.equal(state.offers.length,0);
}));
test('control loss during permission acquisition immediately retires the pending capture owner',()=>withBrowser('early',async state=>{
    let finish;const pending=new Promise(resolve=>finish=resolve);navigator.mediaDevices.getUserMedia=()=>pending;
    const opening=open(state,{},500);state.client.lose();await assert.rejects(opening,{code:'transport'});
    finish(state.stream);await tick();assert.equal(state.trackStops,1);assert.equal(state.peer,undefined);assert.equal(state.client.events.size,0);assert.equal(state.client.lifetimes.size,0);
}));
test('explicit durable end submits once with original revision and confirms journal status',()=>withBrowser('early',async state=>{
    const voice=await open(state);const first=voice.end('original-end',1);assert.equal(first,voice.end('original-end',1));
    await assert.rejects(voice.end('replacement-end',1),{code:'conflict'});await first;await voice.ended;
    const ends=state.client.commands.filter(c=>c.type==='connection.end');assert.equal(ends.length,1);
    assert.equal(ends[0].scope.operation_id,'original-end');assert.equal(ends[0].scope.expected_revision,1);assert.equal(voice.savedEnd.submissionAttempted,true);
    const saved=voice.savedEnd;saved.scope.operation_id='forged';assert.equal(voice.savedEnd.scope.operation_id,'original-end');
    assert.equal(state.trackStops,1);assert.equal(state.peerCloses,1);assert.equal(state.client.closed,false);
}));
test('uncertain explicit end keeps one original intent and releases local capture',()=>withBrowser('early',async state=>{
    const voice=await open(state);state.client.operationStatus=async()=>({state:'unknown'});
    const first=voice.end('uncertain-end',1,{timeoutMs:5});await assert.rejects(first,{code:'timeout'});
    assert.equal(first,voice.end('uncertain-end',1));await assert.rejects(voice.end('new-end',2),{code:'conflict'});
    assert.equal(state.client.commands.filter(c=>c.type==='connection.end').length,1);assert.equal(state.trackStops,1);assert.equal(state.peerCloses,1);
}));
test('invalid or cancelled end does not consume the caller original intent',()=>withBrowser('early',async state=>{
    const voice=await open(state);await assert.rejects(voice.end('original-offer',1),{code:'invalid-request'});
    const abort=new AbortController();abort.abort();await assert.rejects(voice.end('valid-end',1,{signal:abort.signal}),{code:'cancelled'});
    assert.equal(voice.savedEnd,undefined);await voice.end('valid-end',1);assert.equal(state.client.commands.filter(c=>c.type==='connection.end').length,1);
}));

test('video answer rejects rejected, partial, nonduplex and nonVP8 profiles', () => {
    const answer = 'v=0\r\na=sendrecv\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n';
    assert.doesNotThrow(() => validateVideoAnswer(answer));
    const control = 'm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sctp-port:5000\r\n';
    assert.throws(() => validateVideoAnswer(answer + control), {code:'unsupported-media-profile'});
    for (const invalid of [answer.replace('m=video 9', 'm=video 0'),
        answer.replace('VP8/90000', 'H264/90000'), answer.replace('a=sendrecv', 'a=recvonly'),
        answer.slice(0, answer.indexOf('m=video')), answer + 'm=video 9 UDP/TLS/RTP/SAVPF 96\r\n',
        answer.replace('m=video 9 UDP/TLS/RTP/SAVPF 96', 'm=video 9 UDP/TLS/RTP/SAVPF 96 102'),
        answer + control + control, answer + 'm=message 9 TCP/MSRP *\r\n']) {
        assert.throws(() => validateVideoAnswer(invalid), error => error.code === 'unsupported-media-profile');
    }
});

test('supplied video capture enforces the selected profile and has a bounded constraint deadline', async () => {
    let constraints;
    const settings = { width: 640, height: 360, frameRate: 30 };
    const track = { applyConstraints: async value => { constraints = value; }, getSettings: () => settings };
    const stream = { getVideoTracks: () => [track] };
    await constrainVideoCapture(stream, 100);
    assert.deepEqual(constraints, { width: {max: 640}, height: {max: 360}, frameRate: {max: 30} });
    settings.width = 1920;
    await assert.rejects(() => constrainVideoCapture(stream, 100), error => error.code === 'unsupported-media-profile');
    track.applyConstraints = () => new Promise(() => {});
    await assert.rejects(() => constrainVideoCapture(stream, 5), error => error.code === 'timeout');
});


test('video diagnostics bind only selected ICE metadata without addresses or credentials', async () => {
    const { videoDiagnostics } = await import('../typescript/dist/browser_video_stats.js');
    const report = new Map([
        ['inbound', {type: 'inbound-rtp', kind: 'video', transportId: 'transport', framesDecoded: 10}],
        ['transport', {selectedCandidatePairId: 'selected'}],
        ['selected', {state: 'succeeded', remoteCandidateId: 'remote', localCandidateId: 'local'}],
        ['remote', {port: 12345, protocol: 'udp', candidateType: 'host', address: 'private-address', usernameFragment: 'private-ice'}],
        ['local', {port: 23456, address: 'private-local'}],
        ['unselected', {state: 'succeeded', remoteCandidateId: 'bypass'}],
        ['bypass', {port: 34567, protocol: 'udp', candidateType: 'prflx'}],
    ]);
    const result = await videoDiagnostics({getStats: async () => report});
    assert.equal(result[0].selectedRemotePort, 12345);
    assert.equal(result[0].selectedLocalPort, 23456);
    assert.equal(result[0].selectedTransport, 'udp');
    assert.equal(result[0].selectedCandidateType, 'host');
    assert.doesNotMatch(JSON.stringify(result), /private-address|private-ice|private-local|34567/);
    report.get('selected').state = 'in-progress';
    assert.equal((await videoDiagnostics({getStats: async () => report}))[0].selectedRemotePort, undefined);
});

test('video diagnostics omit malformed ICE metadata instead of exporting it', async () => {
    const { videoDiagnostics } = await import('../typescript/dist/browser_video_stats.js');
    const report = new Map([
        ['inbound', {type: 'inbound-rtp', kind: 'video', transportId: 'transport'}],
        ['transport', {selectedCandidatePairId: 'pair'}],
        ['pair', {state: 'succeeded', remoteCandidateId: 'remote', localCandidateId: 'local'}],
        ['remote', {port: 70000, protocol: 'private-protocol', candidateType: 'private-type'}],
        ['local', {port: 'private-port'}],
    ]);
    const result = await videoDiagnostics({getStats: async () => report});
    assert.equal(result[0].selectedRemotePort, undefined);
    assert.equal(result[0].selectedLocalPort, undefined);
    assert.equal(result[0].selectedTransport, undefined);
    assert.equal(result[0].selectedCandidateType, undefined);
    assert.doesNotMatch(JSON.stringify(result), /private/);
});


test('browser session selection snapshots every original ID before capture',async()=>{
    const {browserOfferSelection}=await import('../typescript/dist/browser_binding.js');
    const input=intent({video:true}),selected=browserOfferSelection(input);input.sessionId='changed';input.operationId='replacement';
    assert.equal(selected.sessionId,'session');assert.equal(selected.operationId,'original-offer');
    for(const invalid of [{conversationId:undefined},{sessionId:undefined},{connectionId:'has space'},{operationId:'\u0000'},
        {audioStreamId:'é'.repeat(65)},{expectedRevision:-1},{video:true,videoStreamId:undefined},
        {video:true,videoStreamId:'audio-stream'},{video:false,videoStreamId:'video-stream'}])
        assert.throws(()=>browserOfferSelection({...intent(),...invalid}),{code:'invalid-request'});
});
test('invalid explicit selection and wrong physical transport fail before capture',async()=>withBrowser('early',async state=>{
    await assert.rejects(BrowserVoiceSession.open(state.client,{...intent(),sessionId:undefined}),{code:'invalid-request'});
    state.client.controlTransport='webtransport';await assert.rejects(open(state),{code:'unsupported-transport'});
    assert.equal(state.microphoneRequests,0);assert.equal(state.offers.length,0);
}));

test('a stalled offer obeys the whole participation deadline and releases capture', async () => {
    await withBrowser('offer-stall', async state => {
        await assert.rejects(open(state, {}, 20), error => error.code === 'timeout');
        assert.ok(state.trackStopped && state.peerClosed && !state.client.closed);
        assert.equal(state.offers.length, 0);
    });
});

test('caller option mutation cannot change the selected media profile during capture', async () => {
    await withBrowser('early', async state => {
        const options = intent();
        const pending = open(state, options, 100);
        options.video = true;
        const voice = await pending;
        assert.equal(state.offers.length, 1);
        assert.equal(state.offers[0].scope.cid, "conversation");
        await voice.close();
        assert.ok(state.trackStopped && state.peerClosed);
    });
});


test('asynchronous offer send failure settles promptly and releases owned browser resources', async () => {
    await withBrowser('late-send-error', async state => {
        await assert.rejects(open(state, {}, 100), error=>error.code==='webrtc-failed');
        assert.ok(state.trackStopped && state.peerClosed && !state.client.closed);
    });
});

for (const failure of ['capture-ended', 'ice-failed']) {
    test(`browser ${failure} releases capture and signaling exactly once`, async () => {
        await withBrowser('early', async state => {
            const voice = await open(state, {}, 100);
            if (failure === 'capture-ended') state.track.dispatchEvent(new Event('ended'));
            else {
                state.peer.iceConnectionState = 'failed';
                state.peer.dispatchEvent(new Event('iceconnectionstatechange'));
            }
            await voice.ended;
            assert.ok(state.trackStopped && state.peerClosed && !state.client.closed);
            assert.equal(await voice.close(), undefined);
        });
    });
}

test('capture uses selected local device and rejects incompatible selectors before signaling', async () => {
    await withBrowser('early', async state => {
        const voice = await open(state, {audioDeviceId: 'fixture-microphone'}, 100);
        assert.deepEqual(state.constraints.audio, {deviceId: {exact: 'fixture-microphone'}});
        await voice.close();
        for (const options of [{audioDeviceId:''}, {audioDeviceId:'private-device-canary\u0000'},
            {videoDeviceId:'camera'}, {audioDeviceId:'mic',inputStream:state.stream}]) {
            await assert.rejects(() => open(state, options, 100),
                error=>error.code==='invalid-request'&&!error.message.includes('private-device-canary'));
        }
        assert.equal(state.microphoneRequests, 1);
    });
});

test('video capture awaits finish before sender creation so queued bypass cannot detach the transform', async () => {
    await withBrowser('early', async state => {
        const names=['RTCRtpSender','RTCRtpScriptTransform','Worker','RTCPeerConnection'];
        const originals=new Map(names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
        const PeerBase=globalThis.RTCPeerConnection;
        const video=Object.assign(new EventTarget(),{kind:'video',readyState:'live',
            stop:()=>{state.videoStopped=true;},getSettings:()=>({width:640,height:360,frameRate:30}),
            applyConstraints:()=>new Promise(resolve=>setTimeout(()=>{state.constraintsFinished=true;resolve();},5))});
        const stream={getTracks:()=>[state.track,video],getVideoTracks:()=>[video]};
        class Sender {
            static getCapabilities(){return{codecs:[{mimeType:'video/VP8'}]};}
            constructor(track,encodings=[{}]){this.track=track;this.transform=null;this.encodings=encodings;}
            getParameters(){return{encodings:structuredClone(this.encodings)};} async setParameters(){}
        }
        class Peer extends PeerBase {
            senders=[];
            constructor(){super();state.peerCreations=(state.peerCreations??0)+1;}
            addTrack(track){
                const sender=new Sender(track);this.senders.push(sender);
                if(track.kind==='video')queueMicrotask(()=>{state.bypassed=!sender.transform;});
                return sender;
            }
            addTransceiver(track,init){
                assert.equal(track.kind,'video');assert.equal(init.direction,'sendrecv');
                assert.equal(state.offers.length,state.expectedExistingOffers??0);assert.equal(init.streams[0].getVideoTracks()[0],video);
                assert.deepEqual(init.sendEncodings,[{maxFramerate:30,maxBitrate:1_000_000,scaleResolutionDownBy:1}]);
                state.initialVideoEncoding=structuredClone(init.sendEncodings);
                const sender=new Sender(track,state.ignoreInitialBounds?[{}]:structuredClone(init.sendEncodings));this.senders.push(sender);
                queueMicrotask(()=>{state.bypassed=!sender.transform;});return {sender};
            }
            getTransceivers(){return this.senders.map(sender=>({sender,receiver:sender.receiver??=( {track:{kind:sender.track.kind},jitterBufferTarget:null}),setCodecPreferences(){}}));}
            getSenders(){return this.senders;}
        }
        const replacements={RTCRtpSender:Sender,RTCRtpScriptTransform:class {},RTCPeerConnection:Peer,
            Worker:class {constructor(){state.workerCreations=(state.workerCreations??0)+1;}terminate(){state.workerStopped=true;}}};
        for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:replacements[name]});
        try {
            const session=await BrowserVideoSession.open(state.client,intent({video:true,inputStream:stream}),{timeoutMs:200});
            assert.equal(state.bypassed,false);assert.equal(state.offers.length,1);assert.equal(state.initialVideoEncoding[0].maxFramerate,30);
            await session.end("video-end",1);assert.equal(session.savedEnd.scope.operation_id,"video-end");assert.ok(state.trackStopped&&state.videoStopped&&state.workerStopped);
            state.trackStopped=false;state.videoStopped=false;state.ignoreInitialBounds=true;state.expectedExistingOffers=1;
            await assert.rejects(open(state,{video:true,inputStream:stream},200),{code:'unsupported-media-profile'});
            assert.equal(state.offers.length,1);assert.equal(state.workerCreations,1);assert.equal(state.peerCreations,2);
            assert.ok(state.trackStopped&&state.videoStopped);state.ignoreInitialBounds=false;
            state.constraintsFinished=false;video.applyConstraints=async()=>{throw Error('controlled constraints refusal');};
            await assert.rejects(open(state, {video:true,inputStream:stream}, 200));
            assert.equal(state.constraintsFinished,false);assert.equal(state.peerCreations,2);assert.ok(state.trackStopped&&state.videoStopped);
        } finally {
            for(const name of names){const descriptor=originals.get(name);if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
        }
    });
});

test('video transform support rejects an expando-only sender or ignored native setter', async () => {
    const {startVideoTransform}=await import('../typescript/dist/browser_video_setup.js');
    const names=['RTCRtpSender','RTCRtpScriptTransform','Worker'];
    const original=new Map(names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
    let created=0,terminated=0;
    const globals={RTCRtpSender:class{static getCapabilities(){return{codecs:[{mimeType:'video/VP8'}]};}},
        RTCRtpScriptTransform:class {},Worker:class{constructor(){created++;}terminate(){terminated++;}}};
    for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:globals[name]});
    try {
        const sender={track:{kind:'video'}};
        const peer={getTransceivers:()=>[{sender,setCodecPreferences(){}}]};
        assert.throws(()=>startVideoTransform(peer),error=>error.code==='unsupported-media-profile');
        assert.equal(created,0);
        Object.defineProperty(sender,'transform',{get:()=>null,set:()=>{}});
        assert.throws(()=>startVideoTransform(peer),error=>error.code==='unsupported-media-profile');
        assert.equal(created,1);assert.equal(terminated,1);
    } finally {
        for(const name of names){const descriptor=original.get(name);if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
    }
});

test('video preferences retain generic RTX capability without adding another primary codec', async () => {
    const {startVideoTransform,stopVideoWorker}=await import('../typescript/dist/browser_video_setup.js');
    const names=['RTCRtpSender','RTCRtpScriptTransform','Worker'];
    const originals=new Map(names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
    const vp8=Object.freeze({mimeType:'video/VP8',clockRate:90000});
    const rtx=Object.freeze({mimeType:'video/rtx',clockRate:90000});
    let available=[vp8,{mimeType:'video/H264',clockRate:90000},rtx,{mimeType:'video/red',clockRate:90000}];
    let selected;
    const replacements={RTCRtpSender:class{static getCapabilities(){return{codecs:available};}},
        RTCRtpScriptTransform:class{},Worker:class{terminate(){}}};
    for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:replacements[name]});
    const peer={getTransceivers:()=>[{sender:{track:{kind:'video'},transform:null},setCodecPreferences(codecs){selected=codecs;}}]};
    try {
        const worker=startVideoTransform(peer);
        try {assert.deepEqual(selected,[vp8,rtx]);assert.equal(selected[1],rtx);assert.equal('sdpFmtpLine' in rtx,false);}
        finally {stopVideoWorker(worker);}
        available=[vp8];const optional=startVideoTransform(peer);
        try {assert.deepEqual(selected,[vp8]);} finally {stopVideoWorker(optional);}
        available=[rtx];assert.throws(()=>startVideoTransform(peer),error=>error.code==='unsupported-media-profile');
    } finally {
        for(const name of names){const descriptor=originals.get(name);if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
    }
});

test('video answer accepts optional RTX only when its apt names negotiated VP8', () => {
    const plain='v=0\r\na=sendrecv\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n';
    const repaired=plain.replace('SAVPF 96','SAVPF 96 97')+'a=rtpmap:97 rtx/90000\r\na=fmtp:97 apt=96\r\n';
    assert.doesNotThrow(()=>validateVideoAnswer(plain));
    assert.doesNotThrow(()=>validateVideoAnswer(repaired));
    assert.doesNotThrow(()=>validateVideoAnswer(repaired.replace('apt=96','rtx-time=200; apt=96')));
    const bad=[
        repaired.replace('apt=96','apt=98'),
        repaired.replace('apt=96','apt=97'),
        repaired.replace('apt=96','apt=0'),
        repaired.replace('apt=96','rtx-time=200'),
        repaired.replace('apt=96','apt=96; apt=96'),
        repaired.replace('apt=96','apt=invalid'),
        repaired+'a=fmtp:97 apt=96\r\n',
        repaired+'a=rtpmap:97 rtx/90000\r\n',
        repaired+'a=rtpmap:96 H264/90000\r\n',
        repaired.replace('SAVPF 96 97','SAVPF 96 97 97'),
        repaired.replace('SAVPF 96 97','SAVPF 96 097 97'),
        repaired.replace('SAVPF 96 97','SAVPF 97'),
        repaired.replace('rtx/90000','rtx/8000'),
        repaired.replace('VP8/90000','H264/90000'),
        repaired.replace('SAVPF 96 97','SAVPF 96 97 128')+'a=rtpmap:128 VP8/90000\r\n',
    ];
    for(const answer of bad)assert.throws(()=>validateVideoAnswer(answer),error=>error.code==='unsupported-media-profile');
});

test('listener registration refusal leaves no lifetime observer or capture owner',()=>withBrowser('signaling-listener',async state=>{
    await assert.rejects(open(state),{code:'webrtc-failed'});assert.equal(state.client.lifetimes.size,0);assert.equal(state.microphoneRequests,0);
}));
test('cancelled open before capture and during permission never submits replacement intent',()=>withBrowser('early',async state=>{
    const abort=new AbortController();abort.abort();await assert.rejects(BrowserVoiceSession.open(state.client,intent(),{signal:abort.signal}),{code:'cancelled'});
    assert.equal(state.microphoneRequests,0);
    let finish;const pending=new Promise(resolve=>finish=resolve);navigator.mediaDevices.getUserMedia=()=>pending;
    const next=new AbortController(),opening=BrowserVoiceSession.open(state.client,intent(),{signal:next.signal});next.abort();
    await assert.rejects(opening,{code:'cancelled'});finish(state.stream);await tick();assert.equal(state.trackStops,1);assert.equal(state.offers.length,0);
}));
test('control lifetime retires established capture with original once-only ownership',()=>withBrowser('early',async state=>{
    const voice=await open(state);state.client.lose();await voice.ended;assert.equal(state.trackStops,1);assert.equal(state.peerCloses,1);
    assert.equal(state.client.events.size,0);assert.equal(state.client.lifetimes.size,0);assert.equal(state.client.commands.length,1);
}));
test('remote track routing uses the negotiated event track and refuses extra media',()=>withBrowser('early',async state=>{
    const voice=await open(state);const track={kind:'audio'};
    state.peer.ontrack({track,streams:[{getTracks:()=>[track,{kind:'video'}]}]});assert.deepEqual(voice.remoteStream.getTracks(),[track]);
    state.peer.ontrack({track:{kind:'video'},streams:[]});await voice.ended;assert.equal(state.trackStops,1);assert.equal(state.peerCloses,1);
}));

test('terminal Session notification cancels pending permission without retargeting capture',()=>withBrowser('early',async state=>{
    let finish;const pending=new Promise(resolve=>finish=resolve);navigator.mediaDevices.getUserMedia=()=>pending;
    const opening=open(state);state.client.emit(envelope('session.ended',{}, {cid:'conversation',sid:'session',operation_id:'external-end'}));
    await assert.rejects(opening,{code:'closed'});finish(state.stream);await tick();assert.equal(state.trackStops,1);assert.equal(state.offers.length,0);
}));
test('original cancellation signal is retained when request options mutate during capture',()=>withBrowser('early',async state=>{
    let finish;const pending=new Promise(resolve=>finish=resolve);navigator.mediaDevices.getUserMedia=()=>pending;
    const original=new AbortController(),replacement=new AbortController(),request={signal:original.signal};
    const opening=BrowserVoiceSession.open(state.client,intent(),request);request.signal=replacement.signal;original.abort();
    await assert.rejects(opening,{code:'cancelled'});finish(state.stream);await tick();assert.equal(state.trackStops,1);assert.equal(state.offers.length,0);
}));

test('public answer profile refuses partial, nonduplex and unoffered audio/data',async()=>{
    const {validateBrowserAnswer}=await import('../typescript/dist/browser_capture.js');
    assert.doesNotThrow(()=>validateBrowserAnswer(audioSdp,false));assert.doesNotThrow(()=>validateBrowserAnswer(audioSdp+videoSdp,true));
    for(const sdp of [audioSdp.replace('opus/48000/2','PCMU/8000'),audioSdp.replace('sendrecv','recvonly'),audioSdp.replace('m=audio 9','m=audio 0'),audioSdp+videoSdp,
        audioSdp+'m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n'])assert.throws(()=>validateBrowserAnswer(sdp,false),{code:'unsupported-media-profile'});
    assert.throws(()=>validateBrowserAnswer(audioSdp,true),{code:'unsupported-media-profile'});
});


test('explicit ICE services reach only the original native peer and stay out of wire intents',()=>withBrowser('early',async state=>{
    const iceServers=[{urls:['stun:127.0.0.1:3478','stuns:localhost:5349']},
        {urls:['turn:relay.example:3478?transport=udp','turns:[::1]:5349?transport=tcp'],username:'private-turn-user',credential:'private-turn-secret'}];
    const voice=await open(state,{iceServers});
    assert.deepEqual(state.peerConfiguration,{iceServers});
    assert.equal(state.offers.length,1);
    assert(!JSON.stringify(voice.savedOffer).includes('private-turn'));
    assert(!JSON.stringify(state.client.commands).includes('relay.example'));
    await voice.close();assert.equal(state.peerCloses,1);
}));

test('ICE configuration is snapshotted before pending capture permission',()=>withBrowser('early',async state=>{
    let finish;const pending=new Promise(resolve=>finish=resolve);
    navigator.mediaDevices.getUserMedia=()=>pending;
    const iceServers=[{urls:['turn:relay.example:3478'],username:'original-user',credential:'original-secret'}];
    const options={iceServers,iceTransportPolicy:'relay'};
    const opening=open(state,options);
    options.iceTransportPolicy='all';
    iceServers[0].urls[0]='turn:changed.example:3478';iceServers[0].credential='changed-secret';iceServers.push({urls:'stun:changed.example'});
    finish(state.stream);const voice=await opening;
    assert.deepEqual(state.peerConfiguration,{iceServers:[{urls:['turn:relay.example:3478'],username:'original-user',credential:'original-secret'}],iceTransportPolicy:'relay'});
    await voice.close();
}));

test('invalid or excessive ICE services refuse before capture and keep error details private',()=>withBrowser('early',async state=>{
    const invalid=[null,{},Array(2),[{urls:Array(2)}],Array(9).fill({urls:'stun:localhost'}),[{urls:[]}],
        [{urls:['stun:localhost','stun:localhost','stun:localhost','stun:localhost','stun:localhost']}],
        Array(5).fill({urls:Array(4).fill('stun:localhost')}),
        ...['https://private-secret.example','stun://localhost','stun:localhost:0','stun:localhost:65536','stun:localhost/path',
            'stun:user:private-secret@localhost','stun:localhost?transport=udp','turn:localhost?transport=other',
            'stun:[not-an-ip]','stun:localhost#private-secret'].map(urls=>({urls})).map(s=>[s]),
        [{urls:'turn:localhost'}],[{urls:'stun:localhost',username:4}],
        [{urls:'turn:localhost',username:'user',credential:{private:'secret'}}],
        [{urls:'stun:localhost',credential:'x'.repeat(2049)}],
        [{urls:'stun:localhost',privateSecret:'private-secret'}]];
    for(const iceServers of invalid)await assert.rejects(open(state,{iceServers}),error=>error.code==='invalid-request'&&!error.message.includes('private-secret'));
    assert.equal(state.microphoneRequests,0);assert.equal(state.peer,undefined);assert.equal(state.offers.length,0);
}));

test('relay-only policy requires explicit TURN and rejects invalid policy before capture',()=>withBrowser('early',async state=>{
    for(const options of [{iceTransportPolicy:'relay'},{iceTransportPolicy:'relay',iceServers:[]},
        {iceTransportPolicy:'relay',iceServers:[{urls:'stun:localhost'}]},
        ...[null,4,'none','private-policy-canary'].map(iceTransportPolicy=>({iceTransportPolicy}))]){
        await assert.rejects(open(state,options),error=>error.code==='invalid-request'&&!error.message.includes('canary'));
    }
    assert.equal(state.microphoneRequests,0);assert.equal(state.peer,undefined);assert.equal(state.offers.length,0);
}));

test('unverifiable relay policy refuses before capture',()=>withBrowser('early',async state=>{
    delete RTCPeerConnection.prototype.getConfiguration;
    await assert.rejects(open(state,{iceTransportPolicy:'relay',iceServers:[{urls:'turn:localhost:3478',username:'synthetic',credential:'synthetic'}]}),{code:'unsupported-media-profile'});
    assert.equal(state.microphoneRequests,0);assert.equal(state.offers.length,0);assert.equal(state.peer,undefined);
}));

test('ignored relay policy closes owned capture and peer before submitting an offer',()=>withBrowser('relay-ignored',async state=>{
    await assert.rejects(open(state,{iceTransportPolicy:'relay',iceServers:[{urls:'turn:localhost:3478',username:'synthetic',credential:'synthetic'}]}),{code:'unsupported-media-profile'});
    assert.equal(state.offers.length,0);assert.equal(state.peerCloses,1);assert.equal(state.trackStops,1);
}));


test('original video sender refuses unretained or excessive initialization bounds',async()=>{
    const {addBoundedVideoSender}=await import('../typescript/dist/browser_video_setup.js');
    const track={kind:'video'},stream={};
    const valid={maxFramerate:30,maxBitrate:1_000_000,scaleResolutionDownBy:1};
    let supplied;
    const peer=encodings=>({addTransceiver(actual,init){assert.equal(actual,track);supplied=init;return{sender:{track,getParameters:()=>({encodings})}};}});
    addBoundedVideoSender(peer([{...valid}]),track,stream);
    assert.equal(supplied.streams[0],stream);assert.equal(supplied.direction,'sendrecv');
    for(const encodings of [undefined,[],[{}],[{...valid,maxFramerate:60}],[{...valid,maxBitrate:2_000_000}],
        [{...valid,scaleResolutionDownBy:2}],[{...valid},{...valid}]])
        assert.throws(()=>addBoundedVideoSender(peer(encodings),track,stream),{code:'unsupported-media-profile'});
    assert.throws(()=>addBoundedVideoSender({addTransceiver(){return{sender:{track:{},getParameters:()=>({encodings:[valid]})}};}},track,stream),{code:'unsupported-media-profile'});
});
