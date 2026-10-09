import assert from 'node:assert/strict';
import { test } from 'node:test';
import { BrowserVoiceSession } from '../typescript/dist/browser.js';

// Exercise handshake ordering without a microphone, network or provider call.
async function withBrowser(mode, run) {
    const names = ['navigator', 'MediaStream', 'RTCPeerConnection', 'WebSocket'];
    const original = new Map(names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
    const state = { offers: [], trackStopped: false, peerClosed: false, socketClosed: false, microphoneRequests: 0 };
    const stream = { getTracks: () => [{ kind: 'audio', readyState: 'live', stop: () => { state.trackStopped = true; } }] };
    state.stream = stream;
    class Peer extends EventTarget {
        iceGatheringState = 'complete';
        constructor() { super(); if (mode === 'peer-constructor') throw new Error('fixture peer failure'); }
        addTrack() { if (mode === 'track-add') throw new Error('fixture track failure'); }
        createDataChannel() {}
        async createOffer() { await Promise.resolve(); return { type: 'offer', sdp: 'fixture-offer' }; }
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
            if (mode === 'late') setTimeout(() => this.open(), 5);
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
            if (signal.type === 'bye') { this.message({ type: 'bye' }); return; }
            state.offers.push(signal);
            this.message({ type: 'answer', sdp: 'fixture-answer', connection_id: 'fixture', request_id: signal.request_id });
            this.message({ type: 'ready', request_id: signal.request_id });
        }
        close() { this.readyState = 3; state.socketClosed = true; }
    }
    const replacements = { navigator: { mediaDevices: { getUserMedia: async () => { state.microphoneRequests++; return stream; } } }, MediaStream: class {}, RTCPeerConnection: Peer, WebSocket: Socket };
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
