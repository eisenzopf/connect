import assert from 'node:assert/strict';
import { test } from 'node:test';
import { videoDiagnostics } from '../typescript/dist/browser_video_stats.js';

test('public encoder observations omit unavailable data and private RTC fields', async () => {
    const source = { kind: 'video', type: 'outbound-rtp', framesEncoded: 90,
        keyFramesEncoded: 3, frameWidth: 640, frameHeight: 360,
        packetsSent: 200, bytesSent: 120000, timestamp: 100,
        totalEncodeTime: 0.4, totalPacketSendDelay: 0.2, framesPerSecond: 30,
        qualityLimitationReason: 'cpu', trackIdentifier: 'private-device-canary',
        ssrc: 'private-device-canary', payload: 'private-device-canary' };
    const peer = { getStats: async () => new Map([['source', source]]) };
    const [observed] = await videoDiagnostics(peer);
    assert.equal(observed.encodeSeconds, 0.4);
    assert.equal(observed.packetSendDelaySeconds, 0.2);
    assert.equal(observed.qualityLimit, 'cpu');
    assert.equal(JSON.stringify(observed).includes('private-device-canary'), false);
    source.totalEncodeTime = undefined;
    source.totalPacketSendDelay = Infinity;
    source.framesPerSecond = -1;
    source.qualityLimitationReason = 'private-device-canary';
    const [missing] = await videoDiagnostics(peer);
    assert.equal(missing.encodeSeconds, undefined);
    assert.equal(missing.packetSendDelaySeconds, undefined);
    assert.equal(missing.framesPerSecond, undefined);
    assert.equal(missing.qualityLimit, undefined);
});

test('capture and congestion observations use exact RTC references and omit invalid/private values', async () => {
    const outbound={id:'out',kind:'video',type:'outbound-rtp',mediaSourceId:'capture',remoteId:'remote',transportId:'transport',
        targetBitrate:1e6,timestamp:100};
    const capture={id:'capture',type:'media-source',kind:'video',frames:300,framesPerSecond:30,trackIdentifier:'private-canary'};
    const remote={id:'remote',type:'remote-inbound-rtp',kind:'video',localId:'out',packetsLost:-1,fractionLost:.03,roundTripTime:.04,
        ssrc:'private-canary'};
    const pair={state:'succeeded',availableOutgoingBitrate:123456,address:'private-canary'};
    const report=new Map([['out',outbound],['capture',capture],['remote',remote],['transport',{selectedCandidatePairId:'pair'}],['pair',pair]]);
    const peer={getStats:async()=>report};
    const [actual]=await videoDiagnostics(peer);
    assert.equal(actual.capturedFrames,300);assert.equal(actual.capturedFramesPerSecond,30);
    assert.equal(actual.targetBitrate,1e6);assert.equal(actual.remotePacketsLost,-1);
    assert.equal(actual.remoteFractionLost,.03);assert.equal(actual.remoteRoundTripTimeSeconds,.04);
    assert.equal(actual.availableOutgoingBitrate,123456);
    assert.equal(JSON.stringify(actual).includes('private-canary'),false);
    capture.frames=NaN;capture.framesPerSecond=-1;outbound.targetBitrate=Infinity;
    remote.localId='other';pair.state='waiting';
    const [missing]=await videoDiagnostics(peer);
    for(const key of ['capturedFrames','capturedFramesPerSecond','targetBitrate','remotePacketsLost',
        'remoteFractionLost','remoteRoundTripTimeSeconds','availableOutgoingBitrate'])assert.equal(missing[key],undefined);
    remote.localId='out';remote.fractionLost=1.1;remote.roundTripTime=-1;remote.packetsLost=NaN;
    const [invalid]=await videoDiagnostics(peer);
    assert.equal(invalid.remoteFractionLost,undefined);assert.equal(invalid.remoteRoundTripTimeSeconds,undefined);
    assert.equal(invalid.remotePacketsLost,undefined);
});

test('worker snapshots deduplicate concurrent requests, retain pending age, and reject private fields', async () => {
    let calls=0;
    const counters={observedFrames:300,observedKeyframes:1,keyframeRequests:1,keyframeCompletions:0,
        keyframeFailures:0,keyframeRequestPending:true,pendingRequestSeconds:8};
    const worker={postMessage:(_data,[port])=>{calls++;setTimeout(()=>{port.postMessage(counters);port.close();},5);}};
    const peer={getStats:async()=>new Map([['out',{kind:'video',type:'outbound-rtp',timestamp:0}]])};
    const [a,b]=await Promise.all([videoDiagnostics(peer,worker),videoDiagnostics(peer,worker)]);
    assert.equal(calls,1);assert.deepEqual(a[0].worker,counters);assert.deepEqual(b[0].worker,counters);
    counters.payload='private-canary';
    assert.equal((await videoDiagnostics(peer,worker))[0].worker,undefined);
    delete counters.payload;counters.observedFrames=-1;
    assert.equal((await videoDiagnostics(peer,worker))[0].worker,undefined);
});

test('unresponsive or terminated workers cannot leave the diagnostics promise or owned ports open', async () => {
    const peer={getStats:async()=>new Map([['out',{kind:'video',type:'outbound-rtp',timestamp:0}]])};
    const began=performance.now();
    const [observed]=await videoDiagnostics(peer,{postMessage:()=>{}});
    assert.equal(observed.worker,undefined);
    assert.ok(performance.now()-began<1000);
    assert.equal((await videoDiagnostics(peer,{postMessage:()=>{throw Error('terminated');}}))[0].worker,undefined);
});

test('ICE diagnostics read the current public video transport when selected-pair stats are transiently absent', async () => {
    const current = {remote:{port:23456,protocol:'udp',type:'host',address:'private-canary',candidate:'private-canary'},
        local:{port:34567,address:'private-canary'}};
    let calls=0;
    const transport={iceTransport:{state:'connected',getSelectedCandidatePair:()=>{calls++;return current;}}};
    const row={kind:'video',type:'outbound-rtp',timestamp:1};
    const peer={getStats:async()=>new Map([['video',row]]),
        getSenders:()=>[{track:{kind:'audio'},transport:{iceTransport:{getSelectedCandidatePair:()=>{throw Error('Wrong track');}}}},
            {track:{kind:'video'},transport}],getReceivers:()=>[{track:{kind:'video'},transport}]};
    const [out]=await videoDiagnostics(peer);
    assert.equal(out.selectedRemotePort,23456);assert.equal(out.selectedLocalPort,34567);
    assert.equal(out.selectedCandidateType,'host');assert.equal(out.selectedTransport,'udp');
    assert.equal(out.selectedCandidatePairSource,'ice-transport');assert.equal(calls,1);
    assert.equal(JSON.stringify(out).includes('private-canary'),false);
    row.type='inbound-rtp';
    assert.equal((await videoDiagnostics(peer))[0].selectedRemotePort,23456);assert.equal(calls,2);
});

test('succeeded selected-pair stats keep precedence over an unrelated current transport observation', async () => {
    const report=new Map([['video',{kind:'video',type:'inbound-rtp',transportId:'transport',timestamp:1}],
        ['transport',{selectedCandidatePairId:'pair'}],['pair',{state:'succeeded',remoteCandidateId:'remote',localCandidateId:'local'}],
        ['remote',{port:45678,protocol:'udp',candidateType:'host'}],['local',{port:56789}]]);
    const peer={getStats:async()=>report,getReceivers:()=>{throw Error('Primary stats must not query fallback');}};
    const [observed]=await videoDiagnostics(peer);
    assert.equal(observed.selectedRemotePort,45678);assert.equal(observed.selectedLocalPort,56789);
    assert.equal(observed.selectedCandidatePairSource,'stats');
});

test('current ICE observations omit invalid metadata, absent pairs, closed tracks and ambiguous video transports', async () => {
    let current={remote:{port:Infinity,protocol:'private-canary',type:'private-canary',candidate:'private-canary'},local:{port:-1}};
    let calls=0;
    const ice={state:'connected',getSelectedCandidatePair:()=>{calls++;return current;}};
    const sender={track:{kind:'video'},transport:{iceTransport:ice}};
    let senders=[sender];
    const peer={getStats:async()=>new Map([['video',{kind:'video',type:'outbound-rtp',timestamp:1}]]),getSenders:()=>senders};
    const [invalid]=await videoDiagnostics(peer);
    for(const key of ['selectedRemotePort','selectedLocalPort','selectedCandidateType','selectedTransport'])assert.equal(invalid[key],undefined);
    assert.equal(JSON.stringify(invalid).includes('private-canary'),false);
    current=null;assert.equal((await videoDiagnostics(peer))[0].selectedCandidatePairSource,undefined);
    const before=calls;
    for(const state of ['new','closed','disconnected','failed']){
        ice.state=state;assert.equal((await videoDiagnostics(peer))[0].selectedRemotePort,undefined);
    }
    assert.equal(calls,before);
    ice.state='connected';senders=[sender,sender];
    assert.equal((await videoDiagnostics(peer))[0].selectedRemotePort,undefined);assert.equal(calls,before);
    senders=[sender];ice.getSelectedCandidatePair=()=>{throw Error('private-canary');};
    assert.equal((await videoDiagnostics(peer))[0].selectedRemotePort,undefined);
});
