import assert from 'node:assert/strict';
import { test } from 'node:test';
import { BrowserVoiceSession } from '../typescript/dist/browser.js';
import { validateVideoAnswer } from '../typescript/dist/browser_capture.js';
import { constrainVideoCapture } from '../typescript/dist/browser_video_setup.js';

// Exercise handshake ordering without a microphone, network or provider call.
async function withBrowser(mode, run) {
    const names = ['navigator', 'MediaStream', 'RTCPeerConnection', 'WebSocket'];
    const original = new Map(names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
    const state = { offers: [], trackStopped: false, peerClosed: false, socketClosed: false, microphoneRequests: 0 };
    const track = Object.assign(new EventTarget(), { kind: 'audio', readyState: 'live', stop: () => { state.trackStopped = true; } });
    const stream = { getTracks: () => [track] };
    state.track = track;
    state.stream = stream;
    class Peer extends EventTarget {
        iceGatheringState = 'complete';
        constructor() { super(); state.peer = this; if (mode === 'peer-constructor') throw new Error('fixture peer failure'); }
        addTrack() { if (mode === 'track-add') throw new Error('fixture track failure'); }
        createDataChannel() {}
        async createOffer() { if (mode === 'offer-stall') return new Promise(() => {}); await Promise.resolve(); return { type: 'offer', sdp: 'fixture-offer' }; }
        async setLocalDescription(offer) { this.localDescription = offer; }
        async setRemoteDescription(answer) { state.answer = answer; }
        close() { state.peerClosed = true; }
    }
    class Socket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        readyState = 0;
        constructor() {
            super();
            if (mode === 'socket-constructor') throw new Error('fixture socket failure');
            state.socket = this;
            if (mode === 'early') queueMicrotask(() => this.open());
            if (mode === 'late' || mode === 'late-send-error') setTimeout(() => this.open(), 5);
            if (mode === 'closed') queueMicrotask(() => { this.readyState = 3; });
        }
        open() { this.readyState = 1; this.onopen?.(); }
        message(value) {
            const event = new MessageEvent('message', { data: JSON.stringify(value) });
            this.onmessage?.(event);
            this.dispatchEvent(event);
        }
        send(raw) {
            assert.equal(this.readyState, 1);
            const signal = JSON.parse(raw);
            if (mode === 'late-send-error' && signal.type === 'offer-ready') throw Error('fixture send failure');
            if (signal.type === 'bye') { this.message({ type: 'bye' }); return; }
            state.offers.push(signal);
            this.message({ type: 'answer', sdp: 'fixture-answer', connection_id: 'fixture', request_id: signal.request_id });
            this.message({ type: 'ready', request_id: signal.request_id });
        }
        close() { this.readyState = 3; state.socketClosed = true; }
    }
    const replacements = { navigator: { mediaDevices: { getUserMedia: async constraints => { state.constraints = constraints; state.microphoneRequests++; return stream; } } }, MediaStream: class {}, RTCPeerConnection: Peer, WebSocket: Socket };
    for (const name of names) Object.defineProperty(globalThis, name, { configurable: true, value: replacements[name] });
    try { await run(state); }
    finally {
        for (const name of names) {
            const descriptor = original.get(name);
            if (descriptor) Object.defineProperty(globalThis, name, descriptor);
            else delete globalThis[name];
        }
    }
}

for (const mode of ['early', 'late']) {
    test(`browser sends its offer once when the socket opens ${mode}`, async () => {
        await withBrowser(mode, async state => {
            const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100);
            assert.equal(state.offers.length, 1);
            assert.equal(state.offers[0].type, 'offer-ready');
            assert.deepEqual(state.answer, { type: 'answer', sdp: 'fixture-answer' });
            await voice.close();
            assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
        });
    });
}

test('browser fails promptly when signaling closed during offer preparation', async () => {
    await withBrowser('closed', async state => {
        await assert.rejects(() => BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100), error => error.code === 'webrtc-failed');
        assert.equal(state.offers.length, 0);
        assert.ok(state.trackStopped && state.peerClosed);
    });
});

test('browser closes a still-connecting socket after signaling timeout', async () => {
    await withBrowser('stalled', async state => {
        await assert.rejects(() => BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 10), error => error.code === 'timeout');
        assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
    });
});

test('server hangup releases browser microphone and notifies the application', async () => {
    await withBrowser('early', async state => {
        const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100);
        state.socket.message({ type: 'bye', connection_id: 'unrelated' });
        assert.equal(state.trackStopped, false);
        state.socket.message({ type: 'bye', connection_id: 'fixture' });
        await voice.ended;
        assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
    });
});

test('application audio uses the same signaling and is released without requesting a microphone', async () => {
    await withBrowser('early', async state => {
        const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100, { inputStream: state.stream });
        assert.equal(state.microphoneRequests, 0);
        assert.equal(state.offers.length, 1);
        await voice.close();
        assert.ok(state.trackStopped);
    });
});

test('application audio rejects empty, ended and video streams before signaling', async () => {
    await withBrowser('early', async state => {
        for (const tracks of [[], [{ kind: 'audio', readyState: 'ended' }], [{ kind: 'video', readyState: 'live' }]]) {
            await assert.rejects(() => BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100, { inputStream: { getTracks: () => tracks } }), error => error.code === 'invalid-request');
        }
        assert.equal(state.microphoneRequests, 0);
        assert.equal(state.offers.length, 0);
    });
});

for (const mode of ['peer-constructor', 'track-add', 'socket-constructor']) {
    test(`browser releases acquired audio after ${mode} failure`, async () => {
        await withBrowser(mode, async state => {
            await assert.rejects(() => BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100), error => error.code === 'webrtc-failed');
            assert.equal(state.microphoneRequests, 1);
            assert.ok(state.trackStopped);
            if (mode !== 'peer-constructor') assert.ok(state.peerClosed);
            assert.equal(state.offers.length, 0);
        });
    });
}

test('hangup send race still releases all local resources and shares close completion', async () => {
    await withBrowser('early', async state => {
        const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100);
        let terminated = false, closes = 0;
        voice.videoWorker = { terminate() { terminated = true; } };
        state.socket.send = () => { throw new Error('fixture signaling closed during send'); };
        state.socket.close = function() {
            closes++; this.readyState = 3; state.socketClosed = true;
            this.onclose?.();
        };
        const first = voice.close(), second = voice.close();
        assert.equal(first, second);
        await first;
        await voice.ended;
        assert.equal(closes, 1);
        assert.ok(terminated && state.trackStopped && state.peerClosed && state.socketClosed);
    });
});

test('video answer rejects rejected, partial, nonduplex and nonVP8 profiles', () => {
    const answer = 'v=0\r\na=sendrecv\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n';
    assert.doesNotThrow(() => validateVideoAnswer(answer));
    const control = 'm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sctp-port:5000\r\n';
    assert.doesNotThrow(() => validateVideoAnswer(answer + control));
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


test('browser session selection is paired, bounded and immutable before capture', async () => {
    const { browserOfferSelection } = await import('../typescript/dist/browser_binding.js');
    assert.equal(browserOfferSelection({},false),undefined);
    const input={conversationId:'conversation',sessionId:'session'};
    const selected=browserOfferSelection(input,true);input.sessionId='changed';
    assert.deepEqual(selected,{conversation_id:'conversation',session_id:'session'});
    for(const value of [{conversationId:'c'}, {sessionId:'s'}, {conversationId:'c',sessionId:'has space'},
        {conversationId:'c',sessionId:'\u0000'}, {conversationId:'c',sessionId:'é'.repeat(65)}])
        assert.throws(()=>browserOfferSelection(value,true),error=>error.code==='invalid-request');
    assert.throws(()=>browserOfferSelection({conversationId:'c',sessionId:'s'},false));
});

test('invalid explicit video selection fails before requesting microphone or camera', async () => {
    await withBrowser('early',async state=>{
        await assert.rejects(BrowserVoiceSession.open('ws://127.0.0.1:3018','fixture',100,{video:true,conversationId:'c'}), error=>error.code==='invalid-request');
        assert.equal(state.microphoneRequests,0);
        assert.equal(state.offers.length,0);
    });
});


test('a stalled offer obeys the whole participation deadline and releases capture', async () => {
    await withBrowser('offer-stall', async state => {
        await assert.rejects(BrowserVoiceSession.open('ws://127.0.0.1:3018', 'fixture', 20), error => error.code === 'timeout');
        assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
        assert.equal(state.offers.length, 0);
    });
});

test('caller option mutation cannot change the selected media profile during capture', async () => {
    await withBrowser('early', async state => {
        const options = {};
        const pending = BrowserVoiceSession.open('ws://127.0.0.1:3018', 'fixture', 100, options);
        options.video = true;
        const voice = await pending;
        assert.equal(state.offers.length, 1);
        assert.equal(state.offers[0].conversation_id, undefined);
        await voice.close();
        assert.ok(state.trackStopped && state.peerClosed);
    });
});


test('asynchronous offer send failure settles promptly and releases owned browser resources', async () => {
    await withBrowser('late-send-error', async state => {
        await assert.rejects(BrowserVoiceSession.open('ws://127.0.0.1:3018', 'fixture', 100), error=>error.code==='webrtc-failed');
        assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
    });
});

for (const failure of ['capture-ended', 'ice-failed']) {
    test(`browser ${failure} releases capture and signaling exactly once`, async () => {
        await withBrowser('early', async state => {
            const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100);
            if (failure === 'capture-ended') state.track.dispatchEvent(new Event('ended'));
            else {
                state.peer.iceConnectionState = 'failed';
                state.peer.dispatchEvent(new Event('iceconnectionstatechange'));
            }
            await voice.ended;
            assert.ok(state.trackStopped && state.peerClosed && state.socketClosed);
            assert.equal(await voice.close(), undefined);
        });
    });
}

test('capture uses selected local device and rejects incompatible selectors before signaling', async () => {
    await withBrowser('early', async state => {
        const voice = await BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100, {audioDeviceId: 'fixture-microphone'});
        assert.deepEqual(state.constraints.audio, {deviceId: {exact: 'fixture-microphone'}});
        await voice.close();
        for (const options of [{audioDeviceId:''}, {audioDeviceId:'private-device-canary\u0000'},
            {videoDeviceId:'camera'}, {audioDeviceId:'mic',inputStream:state.stream}]) {
            await assert.rejects(() => BrowserVoiceSession.open('ws://127.0.0.1:8190', 'fixture', 100, options),
                error=>error.code==='invalid-request'&&!error.message.includes('private-device-canary'));
        }
        assert.equal(state.microphoneRequests, 1);
    });
});

test('video capture awaits finish before sender creation so queued bypass cannot detach the transform', async () => {
    await withBrowser('early', async state => {
        const names=['RTCRtpSender','RTCRtpScriptTransform','Worker','RTCPeerConnection','WebSocket'];
        const originals=new Map(names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
        const PeerBase=globalThis.RTCPeerConnection,SocketBase=globalThis.WebSocket;
        const video=Object.assign(new EventTarget(),{kind:'video',readyState:'live',
            stop:()=>{state.videoStopped=true;},getSettings:()=>({width:640,height:360,frameRate:30}),
            applyConstraints:()=>new Promise(resolve=>setTimeout(()=>{state.constraintsFinished=true;resolve();},5))});
        const stream={getTracks:()=>[state.track,video],getVideoTracks:()=>[video]};
        class Sender {
            static getCapabilities(){return{codecs:[{mimeType:'video/VP8'}]};}
            constructor(track){this.track=track;this.transform=null;}
            getParameters(){return{encodings:[{}]};} async setParameters(){}
        }
        class Peer extends PeerBase {
            senders=[];
            constructor(){super();state.peerCreations=(state.peerCreations??0)+1;}
            addTrack(track){
                const sender=new Sender(track);this.senders.push(sender);
                if(track.kind==='video')queueMicrotask(()=>{state.bypassed=!sender.transform;});
                return sender;
            }
            getTransceivers(){return this.senders.map(sender=>({sender,setCodecPreferences(){}}));}
            getSenders(){return this.senders;}
        }
        class Socket extends SocketBase {
            send(raw){const signal=JSON.parse(raw);if(signal.type==='bye'){super.send(raw);return;}
                state.offers.push(signal);
                this.message({type:'answer',connection_id:'fixture',request_id:signal.request_id,
                    sdp:'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n'});
                this.message({type:'ready',request_id:signal.request_id});}
        }
        const replacements={RTCRtpSender:Sender,RTCRtpScriptTransform:class {},RTCPeerConnection:Peer,WebSocket:Socket,
            Worker:class {terminate(){state.workerStopped=true;}}};
        for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:replacements[name]});
        try {
            const session=await BrowserVoiceSession.open('ws://127.0.0.1:8190','fixture',200,{video:true,inputStream:stream});
            assert.equal(state.bypassed,false);assert.equal(state.offers.length,1);
            await session.close();assert.ok(state.trackStopped&&state.videoStopped&&state.workerStopped);
            state.trackStopped=false;state.videoStopped=false;
            state.constraintsFinished=false;video.applyConstraints=async()=>{throw Error('controlled constraints refusal');};
            await assert.rejects(BrowserVoiceSession.open('ws://127.0.0.1:8190','fixture',200,{video:true,inputStream:stream}));
            assert.equal(state.constraintsFinished,false);assert.equal(state.peerCreations,1);assert.ok(state.trackStopped&&state.videoStopped);
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
