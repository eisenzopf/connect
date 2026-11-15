import test from "node:test";
import assert from "node:assert/strict";
import {installPeerDiagnostics} from "./browser_failure.mjs";

test("RTP loss and repair counters remain bounded numeric facts without private stats fields",async()=>{
    const original=globalThis.RTCPeerConnection;
    class Peer{
        getTransceivers(){return [];}
        addEventListener(){}
        async getStats(){
            return new Map(Array.from({length:20},(_,index)=>["private-secret-"+index,{
                type:"inbound-rtp",kind:"audio",id:"private-secret",trackIdentifier:"private-secret",
                nackCount:4,concealedSamples:960,silentConcealedSamples:480,concealmentEvents:2,
                jitterBufferDelay:.1,jitterBufferEmittedCount:960,packetsLost:2,
                totalDecodeTime:.01,totalProcessingDelay:.03,totalInterFrameDelay:1,
                totalSquaredInterFrameDelay:.04,jitterBufferTargetDelay:.2,jitterBufferMinimumDelay:.05,
                freezeCount:1,totalFreezesDuration:.4,
                pliCount:Infinity,firCount:-1,retransmittedPacketsSent:"private-secret",
            }]));
        }
    }
    try{
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics();
        for(let index=0;index<5;index++)new globalThis.RTCPeerConnection();
        const value=await globalThis.connectPeerMediaDiagnostics();
        const clocks=await globalThis.connectPeerClockDiagnostics();
        assert.equal(clocks.length,4);assert(clocks.every(rows=>rows.length===8));
        assert.equal(value.length,4);assert(value.every(rows=>rows.length===8));
        const {audio_counter_provenance,...originalFields}=value[0][0];
        assert.deepEqual(audio_counter_provenance.concealedSamples,{present:true,available:true,value:960});
        assert.deepEqual(audio_counter_provenance.totalSamplesReceived,{present:false,available:false,value:null});
        assert.deepEqual(originalFields,{peer_index:0,connection_id:null,report_index:0,stats_timestamp_ms:null,type:"inbound-rtp",kind:"audio",packetsLost:2,nackCount:4,
            concealedSamples:960,silentConcealedSamples:480,concealmentEvents:2,
            jitterBufferDelay:.1,jitterBufferEmittedCount:960,jitterBufferTargetDelay:.2,
            jitterBufferMinimumDelay:.05,totalDecodeTime:.01,totalProcessingDelay:.03,
            totalInterFrameDelay:1,totalSquaredInterFrameDelay:.04,freezeCount:1,totalFreezesDuration:.4});
        assert.equal(JSON.stringify(value).includes("private-secret"),false);
    }finally{
        globalThis.RTCPeerConnection=original;
        delete globalThis.connectPeerTag;delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;delete globalThis.connectPeerClockDiagnostics;
    }
});

test("codec diagnostics preserve negotiation facts and exclude signaling secrets",()=>{
    const original=globalThis.RTCPeerConnection;
    class Peer{
        constructor(){
            this.localDescription={type:"offer",sdp:["v=0","m=audio 9 UDP/TLS/RTP/SAVPF 111","a=sendrecv",
                "a=ice-pwd:synthetic-secret","a=fingerprint:sha-256 synthetic-fingerprint",
                "a=candidate:synthetic-private-address","a=rtpmap:111 opus/48000/2",
                "a=rtcp-fb:111 transport-cc","a=rtcp-fb:111 nack",""].join("\r\n")};
            this.remoteDescription={type:"answer",sdp:["v=0","m=video 9 UDP/TLS/RTP/SAVPF 96","a=sendonly",
                "a=rtpmap:96 VP8/90000","a=rtcp-fb:96 nack pli","a=msid:synthetic-private-msid",""].join("\r\n")};
        }
        getTransceivers(){return [{direction:"sendrecv",currentDirection:"recvonly"}];}
        addEventListener(){}
    }
    try{
        globalThis.RTCPeerConnection=Peer;
        installPeerDiagnostics();new globalThis.RTCPeerConnection();
        const value=globalThis.connectPeerCodecDiagnostics(),encoded=JSON.stringify(value);
        for(const secret of ["synthetic-secret","synthetic-fingerprint","synthetic-private-address","synthetic-private-msid"])
            assert.equal(encoded.includes(secret),false);
        assert.deepEqual(value[0].local.media[0].codecs,[{payload_type:111,name:"opus",clock_rate:48000,channels:2}]);
        assert.equal(value[0].local.media[0].feedback[1].type,"nack");
        assert.equal(value[0].remote.media[0].direction,"sendonly");
        assert.deepEqual(value[0].remote.media[0].codecs,[{payload_type:96,name:"VP8",clock_rate:90000,channels:null}]);
    }finally{
        globalThis.RTCPeerConnection=original;
        delete globalThis.connectPeerTag;delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;delete globalThis.connectPeerClockDiagnostics;
    }
});

test('receiver clock diagnostics resolve kind privately and preserve signed clock facts without IDs or epochs',async()=>{
    const original=globalThis.RTCPeerConnection;
    class Peer{
        getTransceivers(){return [];}
        addEventListener(){}
        async getStats(){
            const now=performance.timeOrigin+performance.now();
            return new Map([
                ['private-local',{type:'inbound-rtp',kind:'audio',id:'private-local',timestamp:now,
                    estimatedPlayoutTimestamp:now+50,packetsReceived:100,totalSamplesReceived:960,
                    concealedSamples:480,silentConcealedSamples:0,concealmentEvents:1,
                    insertedSamplesForDeceleration:120,removedSamplesForAcceleration:240,
                    packetsDiscarded:NaN,packetsLost:-1,
                    jitterBufferEmittedCount:200,jitterBufferDelay:20,jitterBufferTargetDelay:16,
                    jitterBufferMinimumDelay:10,transportId:'private-transport'}],
                ['private-remote',{type:'remote-outbound-rtp',localId:'private-local',timestamp:now-10,
                    remoteTimestamp:now-40,reportsSent:5,ssrc:1234,remoteAddress:'private-address'}],
            ]);
        }
    }
    try{
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics();new globalThis.RTCPeerConnection();
        const [rows]=await globalThis.connectPeerClockDiagnostics();
        assert.deepEqual(rows[0],{type:'inbound-rtp',kind:'audio',estimated_playout_timestamp_present:true,
            estimated_playout_minus_stats_ms:50,packetsReceived:100,framesDecoded:null,totalSamplesReceived:960,
            concealedSamples:480,silentConcealedSamples:0,concealmentEvents:1,
            insertedSamplesForDeceleration:120,removedSamplesForAcceleration:240,packetsDiscarded:null,packetsLost:null,
            jitterBufferEmittedCount:200,jitterBufferDelay:20,jitterBufferTargetDelay:16,jitterBufferMinimumDelay:10,
            jitter_buffer_mean_ms:100,jitter_buffer_target_mean_ms:80,jitter_buffer_minimum_mean_ms:50});
        assert.equal(rows[1].kind,'audio');assert.equal(rows[1].reports_sent,5);
        assert.equal(rows[1].local_minus_remote_clock_ms,30);
        assert(rows[1].last_sr_received_age_ms>=10&&rows[1].last_sr_received_age_ms<1000);
        assert.equal(JSON.stringify(rows).includes('private-'),false);
        for(const key of ['id','ssrc','timestamp','remoteTimestamp','localId'])assert.equal(key in rows[1],false);
    }finally{
        globalThis.RTCPeerConnection=original;
        for(const name of ['connectPeerTag','connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'])delete globalThis[name];
    }
});

test('unsupported clocks, empty buffers and failed clock reads remain explicitly unmeasured',async()=>{
    const original=globalThis.RTCPeerConnection;
    class Peer{
        constructor(){this.fail=false;}
        getTransceivers(){return [];}
        addEventListener(){}
        async getStats(){
            if(this.fail)throw Error('private-error');
            return new Map([['private',{type:'inbound-rtp',kind:'video',timestamp:NaN,
                estimatedPlayoutTimestamp:Infinity,packetsReceived:-1,framesDecoded:NaN,
                jitterBufferEmittedCount:0,jitterBufferDelay:Infinity}]]);
        }
    }
    try{
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics();const peer=new globalThis.RTCPeerConnection();
        const [[row]]=await globalThis.connectPeerClockDiagnostics();
        assert.equal(row.estimated_playout_timestamp_present,false);assert.equal(row.estimated_playout_minus_stats_ms,null);
        assert.equal(row.packetsReceived,null);assert.equal(row.framesDecoded,null);assert.equal(row.jitter_buffer_mean_ms,null);
        peer.fail=true;assert.deepEqual(await globalThis.connectPeerClockDiagnostics(),[{unavailable:true}]);
    }finally{
        globalThis.RTCPeerConnection=original;
        for(const name of ['connectPeerTag','connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'])delete globalThis[name];
    }
});


test('one original RTC statistics poll distinguishes present zero from absent or invalid audio counters',async()=>{
    const original=globalThis.RTCPeerConnection;let polls=0;
    class Peer {
        getTransceivers(){return [];} addEventListener(){}
        async getStats(){polls++;return new Map([['original-audio',{type:'inbound-rtp',kind:'audio',
            timestamp:1700000000000,ssrc:77,concealedSamples:0,silentConcealedSamples:0,
            concealmentEvents:NaN,jitterBufferEmittedCount:960,totalSamplesDuration:0,
            insertedSamplesForDeceleration:undefined,removedSamplesForAcceleration:-1}]]);}
    }
    try {
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics();const peer=new globalThis.RTCPeerConnection();
        globalThis.connectPeerTag(peer,'original-owner');
        const rows=await globalThis.connectPeerMediaDiagnostics(),row=rows[0][0],p=row.audio_counter_provenance;
        assert.equal(polls,1);assert.equal(row.connection_id,'original-owner');assert.equal(row.ssrc,77);
        assert.deepEqual(p.ssrc,{present:true,available:true,value:77});
        assert.deepEqual(p.concealedSamples,{present:true,available:true,value:0});
        assert.deepEqual(p.totalSamplesDuration,{present:true,available:true,value:0});
        assert.deepEqual(p.totalSamplesReceived,{present:false,available:false,value:null});
        assert.deepEqual(p.concealmentEvents,{present:true,available:false,value:null});
        assert.deepEqual(p.insertedSamplesForDeceleration,{present:true,available:false,value:null});
        assert.deepEqual(p.removedSamplesForAcceleration,{present:true,available:false,value:null});
        assert.ok(Object.isFrozen(p)&&Object.isFrozen(p.concealedSamples));
        assert.equal(row.concealmentEvents,undefined,'Original top-level omission policy remains unchanged');
    } finally {
        globalThis.RTCPeerConnection=original;
        for(const name of ['connectPeerTag','connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'])delete globalThis[name];
    }
});


test('existing page initialization opts into bounded PCM explicitly before observation and defaults off',async()=>{
    const originalPeer=globalThis.RTCPeerConnection,originalFlag=Object.getOwnPropertyDescriptor(globalThis,'connectRtcAudioFacts');
    let peers=0,polls=0;
    class Peer{constructor(){peers++;}getTransceivers(){return [];}addEventListener(){}async getStats(){polls++;return new Map();}}
    const globals=['connectPeerTag','connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'];
    try{
        globalThis.RTCPeerConnection=Peer;
        Object.defineProperty(globalThis,'connectRtcAudioFacts',{value:true,configurable:true});
        installPeerDiagnostics();assert.equal(globalThis.connectRtcAudioFacts,false,'Ambient state cannot opt the default fixture in');
        globalThis.RTCPeerConnection=Peer;
        installPeerDiagnostics({peerBound:16,rtcAudioFacts:true});
        const {rtcAudioBoundaryDiagnostics}=await import('../lib/browser_matrix_rtc_audio_diagnostics.mjs');
        const capture=rtcAudioBoundaryDiagnostics({retainSuspectPcm:globalThis.connectRtcAudioFacts===true});
        assert.equal(capture.report().suspect_pcm.enabled,true);
        assert.equal(peers,0);assert.equal(polls,0,'The option adds no peer, graph or statistics poll');
        const before=globalThis.RTCPeerConnection;
        for(const rtcAudioFacts of [null,1,'true',{},[]])assert.throws(()=>installPeerDiagnostics({rtcAudioFacts}),TypeError);
        assert.equal(globalThis.RTCPeerConnection,before,'Invalid configuration fails before installing peer state');
        capture.close();assert.equal(capture.report().suspect_pcm.events.length,0);
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics({rtcAudioFacts:false});
        assert.equal(globalThis.connectRtcAudioFacts,false);
    }finally{
        globalThis.RTCPeerConnection=originalPeer;
        if(originalFlag)Object.defineProperty(globalThis,'connectRtcAudioFacts',originalFlag);else delete globalThis.connectRtcAudioFacts;
        for(const name of globals)delete globalThis[name];
    }
});

// Actual installPeerDiagnostics, same synchronous close; no helper copy.
function terminalFixture(options={}){
    const original=globalThis.RTCPeerConnection,flag=Object.getOwnPropertyDescriptor(globalThis,'connectRtcAudioFacts');
    const globals=['connectPeerTag','connectPeerStreamTag','connectPeerStreamDescriptor','connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics','connectPeerRetireCycle','connectPeerPublicBinding'];
    const saved=new Map(globals.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]));
    const counts={close:0,pair:0,sync:0,stats:0};
    class Peer{
        constructor(){
            this.connectionState='connected';this.iceConnectionState='connected';this.iceGatheringState='complete';this.signalingState='stable';
            this.sendTrack={id:'js-track-distinct-from-wire-msid',kind:'video',readyState:'ended'};
            this.receiveTrack={id:'received-original-track',kind:'video',readyState:'live'};
            this.sources=[{source:22,rtpTimestamp:0xfffffff0,timestamp:1794702000000}];
            this.pair={local:{address:'127.0.0.1',port:50000,protocol:'udp'},remote:{address:'::1',port:40000,protocol:'udp'}};
            this.ice={state:'connected',getSelectedCandidatePair:()=>{counts.pair++;if(this.pairError)throw this.pairError;return this.pair;}};
            this.transport={state:'connected',iceTransport:this.ice,addEventListener(){},removeEventListener(){}};
            this.txs=[{mid:'0',direction:'sendonly',currentDirection:'sendonly',sender:{track:this.sendTrack,transport:this.transport},receiver:{}},
                {mid:'1',direction:'recvonly',currentDirection:'recvonly',sender:{},receiver:{track:this.receiveTrack,transport:this.transport,getSynchronizationSources:()=>{counts.sync++;return this.sources;}}}];
            const section=(mid,tail)=>['m=video 9 UDP/TLS/RTP/SAVPF 120 124','a=mid:'+mid,'a=rtpmap:120 VP8/90000','a=rtpmap:124 rtx/90000',...tail].join('\r\n')+'\r\n';
            this.localDescription={type:'offer',sdp:'v=0\r\na=ice-pwd:DO-NOT-COPY\r\n'+section('0',['a=msid:wire-stream WIRE-MSID-NOT-JS-ID','a=ssrc-group:FID 11 12','a=ssrc:11 cname:DO-NOT-COPY','a=ssrc:12 cname:DO-NOT-COPY'])+section('1',[])};
            this.remoteDescription={type:'answer',sdp:'v=0\r\n'+section('0',[])+section('1',[])};
        }
        getTransceivers(){return this.txs;}getReceivers(){return this.txs.map(tx=>tx.receiver);}addEventListener(){}removeEventListener(){}
        getStats(){counts.stats++;throw Error('Terminal capture must never ask getStats');}
        close(){counts.close++;if(this.closeError)throw this.closeError;this.connectionState='closed';this.signalingState='closed';return 'original-native-close-return';}
    }
    globalThis.RTCPeerConnection=Peer;installPeerDiagnostics({peerBound:16,rtcSourceFrameWitness:options.enabled??true});
    const peer=new globalThis.RTCPeerConnection();
    const tag=(describe=true)=>{
        assert.equal(globalThis.connectPeerTag(peer,'original-connection'),true);
        assert.equal(globalThis.connectPeerStreamTag(peer,'original-connection','source-video','send',peer.txs[0]),true);
        assert.equal(globalThis.connectPeerStreamTag(peer,'original-connection','receive-video','receive',peer.receiveTrack),true);
        if(describe)for(const direction of ['send','receive'])assert(globalThis.connectPeerStreamDescriptor('original-connection',direction));
    };
    return {peer,counts,tag,report:()=>globalThis.connectPeerDiagnostics().peers[0],restore(){
        globalThis.RTCPeerConnection=original;if(flag)Object.defineProperty(globalThis,'connectRtcAudioFacts',flag);else delete globalThis.connectRtcAudioFacts;
        for(const [name,descriptor]of saved){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
    }};
}

test('terminal witness captures real tagged scalar bindings once before immediate native close including ended sender identity',()=>{
    const f=terminalFixture();try{
        f.tag();assert.deepEqual(globalThis.connectPeerStreamDescriptor('original-connection','send'),{connection_id:'original-connection',stream_id:'source-video',track_id:f.peer.sendTrack.id,transceiver_index:0,mid:'0'});
        assert.equal(f.peer.close(),'original-native-close-return');const row=f.report(),terminal=row.terminal_binding;
        assert.equal(terminal.complete,true);assert.equal(terminal.native_close_observed,true);assert.equal(row.live_owner_retained,false);
        assert(terminal.captured_at_ms<=terminal.captured_done_at_ms&&terminal.captured_done_at_ms<=terminal.native_closed_at_ms);
        assert.equal(terminal.bindings.send.track_state,'ended');assert.equal(terminal.bindings.send.primary_ssrc,11);assert.equal(terminal.bindings.send.rtx_ssrc,12);
        assert.deepEqual(terminal.bindings.receive.sync_source,{source:22,rtpTimestamp:0xfffffff0,timestamp:1794702000000,timestamp_clock:'native-uninterpreted'});
        assert.equal(terminal.bindings.send.selected_udp_pair.basis,'RTCIceTransport.getSelectedCandidatePair');
        const encoded=JSON.stringify(terminal);for(const word of ['DO-NOT-COPY','WIRE-MSID-NOT-JS-ID','nominated','succeeded','cipher','HWM','high_water','sdp'])assert.equal(encoded.includes(word),false);
        assert(Object.isFrozen(terminal)&&Object.isFrozen(terminal.bindings.receive.sync_source));
        const before={...f.counts};assert.equal(f.peer.close(),'original-native-close-return');assert.equal(f.report().terminal_binding,terminal);
        assert.equal(f.counts.pair,before.pair);assert.equal(f.counts.sync,before.sync);assert.equal(f.counts.stats,0);
        assert.equal(globalThis.connectPeerRetireCycle().released,true);
    }finally{f.restore();}
});

test('default diagnostics do not allocate terminal bindings or invoke terminal native APIs',()=>{
    const f=terminalFixture({enabled:false});try{
        f.peer.pairError=Error('Must not enter opt-in getter');f.peer.close();const row=f.report();
        assert.equal('terminal_binding'in row,false);assert.equal(f.counts.pair,0);assert.equal(f.counts.sync,0);assert.equal(f.counts.stats,0);
    }finally{f.restore();}
});

test('same string IDs cannot replace the actual tagged native track or transceiver pointers',()=>{
    for(const change of [peer=>{peer.txs[0].sender.track={...peer.sendTrack};},peer=>{peer.txs[0]={...peer.txs[0]};},peer=>{peer.txs[1].receiver={...peer.txs[1].receiver};}]){
        const f=terminalFixture();try{
            f.tag(false);change(f.peer);assert.equal(f.peer.close(),'original-native-close-return');const t=f.report().terminal_binding;
            assert.equal(t.complete,true);assert.equal(Object.values(t.bindings).some(role=>!role.complete),true);assert.equal(f.counts.stats,0);
        }finally{f.restore();}
    }
    const f=terminalFixture();try{
        globalThis.connectPeerTag(f.peer,'original-connection');f.peer.txs[0].sender.track=null;
        assert.equal(globalThis.connectPeerStreamTag(f.peer,'original-connection','source-video','send',f.peer.txs[0]),false);
        f.peer.txs[0].sender.track=f.peer.sendTrack;
        assert.equal(globalThis.connectPeerStreamDescriptor('original-connection','send'),null,'A later first poll cannot adopt an untagged track');
        // Only the explicit successful-capture hook can now pin the track.
        assert.equal(globalThis.connectPeerStreamTag(f.peer,'original-connection','source-video','send',f.peer.txs[0]),true);
        assert.equal(globalThis.connectPeerStreamDescriptor('original-connection','send').track_id,f.peer.sendTrack.id);
        f.peer.close();assert.equal(f.report().terminal_binding.bindings.send.complete,true);
    }finally{f.restore();}
});

test('unsupported or ambiguous native MID FID SIM and SSRC declarations stay incomplete without changing close',()=>{
    const changes=[peer=>{peer.txs[0].mid=null;},peer=>{peer.localDescription.sdp=peer.localDescription.sdp.replace('a=mid:1','a=mid:0');},
        peer=>{peer.localDescription.sdp=peer.localDescription.sdp.replace('a=ssrc-group:FID 11 12','a=ssrc-group:SIM 11 12');},
        peer=>{peer.localDescription.sdp=peer.localDescription.sdp.replace('a=ssrc:12 cname:DO-NOT-COPY\r\n','a=ssrc:12 cname:DO-NOT-COPY\r\na=ssrc:13 cname:DO-NOT-COPY\r\n');},
        peer=>{peer.localDescription.sdp=peer.localDescription.sdp.replace('a=ssrc:12 cname:DO-NOT-COPY\r\n','');},
        peer=>{peer.localDescription.sdp+='x'.repeat(65537);}];
    for(const change of changes){const f=terminalFixture();try{
        f.tag();change(f.peer);assert.equal(f.peer.close(),'original-native-close-return');assert.equal(f.report().terminal_binding.bindings.send.complete,false);
    }finally{f.restore();}}
});

test('selected pair errors nonnumeric endpoints and changed transport states never invent Stats nomination',()=>{
    for(const change of [peer=>{peer.pairError={terminalReason:'PRIVATE-NATIVE-ERROR'};},peer=>{peer.pair.local.address='hidden.local';},peer=>{peer.pair.remote.protocol='tcp';},peer=>{peer.ice.state='failed';},peer=>{peer.transport.state='closed';}]){
        const f=terminalFixture();try{
            f.tag();change(f.peer);assert.equal(f.peer.close(),'original-native-close-return');const t=f.report().terminal_binding;
            assert.equal(t.bindings.send.complete,false);assert.equal(t.bindings.receive.complete,false);
            const text=JSON.stringify(t);assert.equal(text.includes('PRIVATE-NATIVE-ERROR'),false);assert.equal(text.includes('nominated'),false);assert.equal(f.counts.stats,0);
        }finally{f.restore();}
    }
});

test('receiver missing duplicate or invalid synchronization sources are never guessed into a maximum',()=>{
    for(const sources of [[],[{source:22,rtpTimestamp:1,timestamp:100},{source:22,rtpTimestamp:0xffffffff,timestamp:101}],
        [{source:22,rtpTimestamp:2**32,timestamp:100}],[{source:22,rtpTimestamp:1,timestamp:NaN}]]){
        const f=terminalFixture();try{
            f.tag();f.peer.sources=sources;f.peer.close();const t=f.report().terminal_binding;
            assert.equal(t.bindings.send.complete,true);assert.equal(t.bindings.receive.complete,false);
        }finally{f.restore();}
    }
});

test('terminal diagnostics preserve exact original native close exceptions and never fabricate a close receipt',()=>{
    const f=terminalFixture();try{
        f.tag();const original=Error('ORIGINAL-NATIVE-CLOSE');f.peer.closeError=original;
        assert.throws(()=>f.peer.close(),error=>error===original);const terminal=f.report().terminal_binding;
        assert.equal(terminal.complete,false);assert.equal(terminal.native_close_observed,false);assert.equal(terminal.reason,'native-close-error');
        f.peer.closeError=null;assert.equal(f.peer.close(),'original-native-close-return');assert.equal(f.report().terminal_binding,terminal);assert.equal(f.counts.stats,0);
    }finally{f.restore();}
});

test('terminal scalar observations respect the original sixteen-peer bound and release all borrowed native owners',()=>{
    const f=terminalFixture();try{
        f.tag();const peers=[f.peer];for(let i=1;i<17;i++)peers.push(new globalThis.RTCPeerConnection());
        for(const peer of peers)assert.equal(peer.close(),'original-native-close-return');
        const report=globalThis.connectPeerDiagnostics();assert.equal(report.peers.length,16);assert.equal(report.omitted_peers,1);
        assert(report.peers.every(row=>row.live_owner_retained===false));assert.equal(f.counts.close,17);assert.equal(f.counts.stats,0);
        assert.equal(globalThis.connectPeerRetireCycle().retained_peer_owners,0);
    }finally{f.restore();}
});

test('validated public binding uses the same native capture once and remains distinct from terminal closure evidence',()=>{
    const f=terminalFixture();try{
        f.tag();const publicBinding=globalThis.connectPeerPublicBinding('original-connection');
        assert.equal(publicBinding.profile,'connect/rtc-native-public-binding/1');assert.equal(publicBinding.complete,true);
        assert.equal('native_close_observed'in publicBinding,false);assert.equal('native_closed_at_ms'in publicBinding,false);
        assert.equal(f.counts.close,0);assert.equal(f.counts.stats,0);
        const reads={...f.counts};assert.equal(globalThis.connectPeerPublicBinding('original-connection'),publicBinding);
        assert.equal(f.counts.pair,reads.pair);assert.equal(f.counts.sync,reads.sync);
        assert(Object.isFrozen(publicBinding));assert.equal(f.report().public_binding,publicBinding);
        assert.equal(f.peer.close(),'original-native-close-return');const terminal=f.report().terminal_binding;
        assert.equal(terminal.profile,'connect/rtc-native-terminal-binding/1');assert.equal(terminal.native_close_observed,true);
        assert(terminal.captured_at_ms>=publicBinding.captured_done_at_ms);
        assert.equal(globalThis.connectPeerPublicBinding('original-connection'),publicBinding,'A later caller gets the exact old capture, never a new publication timestamp');
        assert(new TextEncoder().encode(JSON.stringify(terminal)).byteLength<=32768);
        assert(new TextEncoder().encode(JSON.stringify(publicBinding)).byteLength<=32768);
    }finally{f.restore();}
});

test('public native capture is opt-in unavailable on ambiguous owner and never retries missing first metadata',()=>{
    const disabled=terminalFixture({enabled:false});try{assert.equal(typeof globalThis.connectPeerPublicBinding,'undefined');disabled.peer.close();}finally{disabled.restore();}
    const f=terminalFixture();try{
        f.tag();assert.equal(globalThis.connectPeerPublicBinding('foreign-owner'),null);
        f.peer.pairError=Error('PRIVATE-PUBLIC-NATIVE-ERROR');const unavailable=globalThis.connectPeerPublicBinding('original-connection');
        assert.equal(unavailable.bindings.send.complete,false);assert.equal(JSON.stringify(unavailable).includes('PRIVATE-PUBLIC-NATIVE-ERROR'),false);
        f.peer.pairError=null;assert.equal(globalThis.connectPeerPublicBinding('original-connection'),unavailable,'A second invocation cannot adopt fresh metadata into the old publication');
        const other=new globalThis.RTCPeerConnection();globalThis.connectPeerTag(other,'original-connection');
        assert.equal(globalThis.connectPeerPublicBinding('original-connection'),null,'Same string owner across two real peers is ambiguous');
        assert.equal(f.peer.close(),'original-native-close-return');assert.equal(other.close(),'original-native-close-return');
    }finally{f.restore();}
});
