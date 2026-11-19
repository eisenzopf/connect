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
    globalThis.RTCPeerConnection=Peer;const install=options.serialized?new Function('return ('+installPeerDiagnostics.toString()+')')():installPeerDiagnostics;install({peerBound:16,rtcSourceFrameWitness:options.enabled??true});
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

test('serialized RTP diagnostics bind only the exact tagged native MID and retain other video rows as unknown',async()=>{
    const f=terminalFixture({serialized:true});try{
        f.tag();
        const preview={id:'separate-private-preview',kind:'video',readyState:'live'};
        const extra={mid:'2',sender:{},receiver:{track:preview,transport:f.peer.transport}};f.peer.txs.push(extra);
        const rows=[
            {id:'camera',type:'outbound-rtp',kind:'video',mid:'0',ssrc:11,packetsSent:8},
            {id:'public',type:'inbound-rtp',kind:'video',mid:'1',trackIdentifier:f.peer.receiveTrack.id,ssrc:22,packetsReceived:9},
            {id:'preview',type:'inbound-rtp',kind:'video',mid:'2',trackIdentifier:preview.id,ssrc:33,packetsReceived:10},
            {id:'unbound',type:'inbound-rtp',kind:'video',ssrc:44,packetsReceived:11},
            {id:'rtx',type:'inbound-rtp',kind:'video',mid:'1',trackIdentifier:f.peer.receiveTrack.id,ssrc:55,packetsReceived:12,codecId:'rtx-codec'},
        ];
        f.peer.getStats=async()=>new Map([...rows.map(row=>[row.id,row]),['rtx-codec',{type:'codec',mimeType:'video/rtx',payloadType:121,clockRate:90000,sdpFmtpLine:'apt=120;rtx-time=200'}]]);
        const read=async()=>{const result=(await globalThis.connectPeerMediaDiagnostics())[0];assert.equal(result.length,5,'Unknown/contradictory/RTX rows must never be dropped');assert.deepEqual(result.map(row=>row.ssrc),[11,22,33,44,55]);return result;};
        const original=await read();
        assert.equal(original[0].original_track.stream_id,'source-video');
        assert.equal(original[1].original_track.stream_id,'receive-video');assert.equal(original[1].original_track.mid,'1');
        assert.equal(original[2].original_track,null);assert.equal(original[2].native_mid,'2');assert.equal(original[2].native_track_identifier,preview.id);
        assert.equal(original[3].original_track,null);assert.equal(original[3].native_mid,null);
        assert.equal(original[4].codec.mime_type,'video/rtx');assert.equal(original[4].codec.associated_payload_type,120);assert.equal(original[4].original_track.stream_id,'receive-video');assert.equal(original[4].native_rtp_binding.codec_dictionary_type,'codec');
        assert.equal(globalThis.connectPeerStreamDescriptor('original-connection','receive',{track:preview}),null);
        assert.equal(globalThis.connectPeerStreamDescriptor('original-connection','receive',{track:f.peer.receiveTrack}).track_id,f.peer.receiveTrack.id);
        for(const patch of [{mid:undefined},{mid:'2'},{mid:'invalid mid'},{trackIdentifier:'wrong-original-track'},{trackIdentifier:null}]){
            const saved={...rows[1]};Object.assign(rows[1],patch);const changed=(await read())[1];
            if('trackIdentifier'in patch){assert.equal(changed.original_track_refusal,'native-track-identifier-contradiction');assert.equal(changed.original_track.track_id,f.peer.receiveTrack.id);assert.equal(changed.native_rtp_binding.complete,false);}
            else assert.equal(changed.original_track,null);
            Object.assign(rows[1],saved);
        }
        extra.mid='1';assert.equal((await read())[1].original_track,null,'Ambiguous native MID cannot be attributed');extra.mid='2';
        assert.equal((await read())[1].original_track.stream_id,'receive-video','Unknown rows do not retag or replace original pointers');
    }finally{f.peer.close();f.restore();}
});

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


test('failed native selected pair retains bounded actual shape without another API or sensitive candidate text',()=>{
 const cases=[
  [peer=>{peer.transport.iceTransport=undefined;},d=>{assert.equal(d.ice_state,'absent');assert.equal(d.pair_read,false);},0],
  [peer=>{peer.ice.state='failed';},d=>{assert.equal(d.ice_state,'failed');assert.equal(d.pair_read,false);},0],
  [peer=>{peer.ice.state='failed';Object.defineProperty(peer.ice,'getSelectedCandidatePair',{get(){throw Error('PRIVATE-MUST-NOT-READ');}});},d=>assert.equal(d.method_kind,'not-read'),0],
  [peer=>{peer.pair.local.address='PRIVATE-LOCAL.local';Object.defineProperty(peer.pair,'remote',{get(){throw Error('PRIVATE-MUST-NOT-READ');}});Object.defineProperty(peer.pair.local,'port',{get(){throw Error('PRIVATE-MUST-NOT-READ');}});},d=>{assert.equal(d.local.address_kind,'mdns');assert.equal(d.local.port_kind,'not-read');assert.equal(d.remote.read,false);},2],
  [peer=>{peer.ice.getSelectedCandidatePair=undefined;},d=>{assert.equal(d.method_kind,'undefined');assert.equal(d.pair_read,false);},0],
  [peer=>{peer.pair=null;},d=>{assert.equal(d.pair_present,false);assert.equal(d.pair_kind,'null');},2],
  [peer=>{peer.pair.local.address='PRIVATE-UNIQUE-HOST.local';},d=>{assert.equal(d.local.address_kind,'mdns');assert.equal('address'in d.local,false);},2],
  [peer=>{peer.pair.remote.protocol='tcp';},d=>{assert.equal(d.remote.protocol_kind,'tcp');assert.equal(d.remote.port,40000);},2],
  [peer=>{peer.pair.local.port='PRIVATE-PORT';},d=>{assert.equal(d.local.port_kind,'invalid');assert.equal('port'in d.local,false);},2],
  [peer=>{peer.pair.remote.protocol='PRIVATE-PROTOCOL';},d=>assert.equal(d.remote.protocol_kind,'other'),2],
  [peer=>{peer.pair.remote={address:null,port:40000,protocol:'udp',type:'prflx',candidate:''};
   peer.remoteDescription.sdp=peer.remoteDescription.sdp.replace('a=mid:0','a=candidate:1 1 UDP 123 127.0.0.1 40000 typ host ufrag PRIVATE-UFRAG\r\na=mid:0');},d=>{
    assert.equal(d.remote.candidate_type,'prflx');assert.equal(d.remote.candidate_empty,true);assert.equal(d.remote.port,40000);assert.equal(d.remote.protocol_kind,'udp');
    assert.deepEqual(d.remote_description_candidates.candidates,[{mid:'0',component:1,address:'127.0.0.1',port:40000,protocol:'udp',type:'host'}]);
    assert.equal(d.remote_description_candidates.complete,true);},2],
  [peer=>{peer.pair.remote={address:null,port:0,protocol:'PRIVATE-PROTO',type:'PRIVATE-TYPE',candidate:'PRIVATE-CANDIDATE'};
   peer.remoteDescription.sdp=peer.remoteDescription.sdp.replace('a=mid:0','a=mid:0\r\na=candidate:1 1 UDP 123 PRIVATE-HOST.local 40000 typ host');},d=>{
    assert.equal(d.remote.candidate_type,'other');assert.equal(d.remote.candidate_empty,false);assert.equal(d.remote.port_kind,'invalid');assert.equal('port'in d.remote,false);
    assert.equal(d.remote.protocol_kind,'other');assert.equal(d.remote_description_candidates.complete,false);assert.equal(d.remote_description_candidates.candidates.length,0);},2],
  [peer=>{peer.pair.remote={address:null};for(const name of ['type','candidate','port','protocol'])Object.defineProperty(peer.pair.remote,name,{get(){throw Error('PRIVATE-GETTER');}});},d=>{
    assert.equal(d.remote.candidate_type,'threw');assert.equal(d.remote.candidate_kind,'threw');assert.equal(d.remote.candidate_empty,null);
    assert.equal(d.remote.port_kind,'threw');assert.equal(d.remote.protocol_kind,'threw');},2],
  [peer=>{peer.pair.remote.address=null;peer.remoteDescription.sdp=peer.remoteDescription.sdp.replace('a=mid:0','a=mid:0\r\n'+Array.from({length:33},(_,i)=>'a=candidate:1 1 UDP 123 127.0.0.1 '+(40000+i)+' typ host').join('\r\n'));},d=>{
    assert.equal(d.remote_description_candidates.candidates.length,32);assert.equal(d.remote_description_candidates.omitted,1);assert.equal(d.remote_description_candidates.complete,false);},2],
  [peer=>{peer.pair.local.address='::ffff:192.168.1.215';},d=>{assert.equal(d.local.address_kind,'numeric-v6');assert.equal(d.local.address,'::ffff:192.168.1.215');},2],
 ];
 for(const [change,check,pairCalls]of cases){const f=terminalFixture();try{
  f.tag();change(f.peer);const snapshot=globalThis.connectPeerPublicBinding('original-connection');
  assert.equal(snapshot.complete,true);assert.equal(snapshot.bindings.send.complete,false);assert.equal(snapshot.bindings.receive.complete,false);
  for(const role of Object.values(snapshot.bindings)){
   assert.equal(role.reason,'selected-pair-unavailable');const detail=role.native_path_diagnostic;
   assert.equal(detail.profile,'connect/rtc-native-selected-pair-shape/1');check(detail);assert(Object.isFrozen(detail));
   const encoded=JSON.stringify(detail);for(const word of ['PRIVATE','candidate:','DO-NOT-COPY','ufrag','sdp','password','error'])assert.equal(encoded.includes(word),false);
  }
  assert.equal(f.counts.pair,pairCalls);assert.equal(f.counts.stats,0);assert.equal(f.counts.sync,0);assert.equal(f.counts.close,0);
  assert.equal(globalThis.connectPeerPublicBinding('original-connection'),snapshot);assert.equal(f.counts.pair,pairCalls);
  assert(new TextEncoder().encode(JSON.stringify(snapshot)).byteLength<=32768);
  assert.equal(f.peer.close(),'original-native-close-return');
 }finally{f.restore();}}
});

test('native path shape stays absent on healthy roles default mode and original getter exceptions',()=>{
 const healthy=terminalFixture();try{healthy.tag();for(const name of ['type','candidate'])Object.defineProperty(healthy.peer.pair.remote,name,{get(){throw Error('PRIVATE-HEALTHY-READ');}});const row=globalThis.connectPeerPublicBinding('original-connection');
  assert.equal(row.bindings.send.complete,true);assert.equal(row.bindings.receive.complete,true);
  assert.equal('native_path_diagnostic'in row.bindings.send,false);assert.equal(healthy.counts.pair,2);assert.equal(healthy.counts.stats,0);
  healthy.peer.close();
 }finally{healthy.restore();}
 const disabled=terminalFixture({enabled:false});try{disabled.peer.close();assert.equal(disabled.counts.pair,0);assert.equal('terminal_binding'in disabled.report(),false);}finally{disabled.restore();}
 const thrown=terminalFixture();try{thrown.tag();thrown.peer.pairError=Error('PRIVATE-GETTER-ERROR');
  const row=globalThis.connectPeerPublicBinding('original-connection');assert.equal(thrown.counts.pair,2);
  assert.equal(row.bindings.send.reason,'native-binding-unavailable');assert.equal('native_path_diagnostic'in row.bindings.send,false);
  assert.equal(JSON.stringify(row).includes('PRIVATE'),false);assert.equal(thrown.peer.close(),'original-native-close-return');
 }finally{thrown.restore();}
});


// The production observer sees the ORIGINAL native promise/transport/event
// graph below. Controlled trusted-event calls are test fixtures, not authority.
function selectionFixture({twoEarly=false,late=false,unsupported=false,serialized=false}={}){
 const f=terminalFixture({serialized}),peer=f.peer,local=peer.localDescription,remote=peer.remoteDescription,pair=peer.pair;
 const listeners=new Map(),extraListeners=new Map();let resolveLocal,rejectLocal,reads=0;
 const original=new Promise((ok,fail)=>{resolveLocal=ok;rejectLocal=fail;});
 const nativePrototype=Object.getPrototypeOf(Object.getPrototypeOf(peer));
 nativePrototype.setLocalDescription=function(){return original;};
 const attach=(ice,store)=>{ice.onselectedcandidatepairchange=null;ice.onstatechange=null;
  ice.addEventListener=(kind,fn,options)=>{const rows=store.get(kind)??[];rows.push({fn,capture:options?.capture===true});store.set(kind,rows);};
  ice.removeEventListener=(kind,fn)=>{store.set(kind,(store.get(kind)??[]).filter(row=>row.fn!==fn));};};
 attach(peer.ice,listeners);if(unsupported)delete peer.ice.onselectedcandidatepairchange;
 peer.connectionState='new';peer.iceConnectionState='new';peer.signalingState='have-local-offer';peer.ice.state='new';peer.pair=null;
 peer.remoteDescription=late?remote:null;
 const extra={state:'new',getSelectedCandidatePair:()=>null};attach(extra,extraListeners);
 if(twoEarly)peer.txs.push({mid:'2',sender:{transport:{state:'new',iceTransport:extra}},receiver:{track:{kind:'audio'}}});
 let counter=1,change=null,blocked=null;
 const map=()=>{const rows=[
  ['c',{id:'c',type:'codec',mimeType:'video/VP8',payloadType:120,clockRate:90000}],
  ['t',{id:'t',type:'transport',dtlsState:'connected',selectedCandidatePairId:'p',selectedCandidatePairChanges:counter}],
  ['p',{id:'p',type:'candidate-pair',transportId:'t',localCandidateId:'l',remoteCandidateId:'r',state:'succeeded',nominated:true}],
  ['l',{id:'l',type:'local-candidate',transportId:'t',address:'127.0.0.1',port:50000,protocol:'udp'}],
  ['r',{id:'r',type:'remote-candidate',transportId:'t',address:'127.0.0.2',port:40000,protocol:'udp'}],
  ['send',{id:'send',type:'outbound-rtp',kind:'video',ssrc:11,codecId:'c',transportId:'t',packetsSent:7}],
  ['receive',{id:'receive',type:'inbound-rtp',kind:'video',ssrc:22,codecId:'c',transportId:'t',packetsReceived:9}],
 ];const report=new Map(rows);change?.(report);return report;};
 peer.getStats=()=>{reads++;return blocked??Promise.resolve(map());};
 const emit=(type,trusted=true)=>{for(const row of [...(listeners.get(type)??[])].sort((a,b)=>Number(b.capture)-Number(a.capture)))row.fn({type,isTrusted:trusted,currentTarget:peer.ice});};
 return {...f,original,emit,extra,listeners,arm:async()=>{const returned=peer.setLocalDescription(local);assert.equal(returned,original);resolveLocal();await returned;},reject:rejectLocal,
  select(){if(twoEarly)peer.txs.pop();peer.remoteDescription=remote;peer.connectionState='connected';peer.iceConnectionState='connected';peer.signalingState='stable';peer.ice.state='connected';peer.pair={...pair,remote:{address:null,port:40000,protocol:'udp',type:'prflx',candidate:''}};f.tag();emit('selectedcandidatepairchange');emit('statechange');},
  report:map,setCounter:value=>{counter=value;},setChange:value=>{change=value;},setBlocked:value=>{blocked=value;},reads:()=>reads,
  listenerCount:()=>[...listeners.values(),...extraListeners.values()].reduce((n,v)=>n+v.length,0),
  cleanup(){peer.close();assert.equal(this.listenerCount(),0);f.restore();}};
}

test('early original transport selection epoch binds typed redaction to the same one real Stats read',async()=>{
 const f=selectionFixture({twoEarly:true,serialized:true});try{await f.arm();assert.equal(f.listenerCount(),4);f.select();
  const rows=(await globalThis.connectPeerMediaDiagnostics())[0];assert.equal(f.reads(),1);assert.equal(rows.length,2);
  const epoch=rows[0].selected_path.transport_epoch;assert.deepEqual(epoch,{profile:'connect/rtc-native-selection-epoch/1',transport_id:'p0t1',armed_before_selection:true,initial_selection_count:0,selection_count:1,state_version:2});
  assert.deepEqual(rows[1].selected_path.transport_epoch,epoch);assert.equal(rows[0].selected_path.selected_candidate_pair_changes,1);
  assert.equal(rows[0].selected_path.remote.address,'127.0.0.2','Only REAL original Stats contains this address');
  const snapshot=globalThis.connectPeerPublicBinding('original-connection');assert.equal(snapshot.bindings.send.complete,true);assert.equal(snapshot.bindings.receive.complete,true);
  for(const role of Object.values(snapshot.bindings)){assert.equal(role.selected_udp_pair.kind,'prflx-address-redacted');assert.equal(role.selected_udp_pair.remote.address,null);assert.deepEqual(role.selected_udp_pair.transport_epoch,epoch);}
  const saved=JSON.stringify(snapshot);f.setCounter(2);f.emit('selectedcandidatepairchange');assert.equal(JSON.stringify(globalThis.connectPeerPublicBinding('original-connection')),saved,'Original public capture cannot be refreshed');
  assert.equal(f.peer.close(),'original-native-close-return');assert.equal(f.listenerCount(),0);assert.equal(globalThis.connectPeerDiagnostics().peers[0].live_owner_retained,false);assert.equal(globalThis.connectPeerDiagnostics().peers[0].terminal_binding.complete,true);
 }finally{f.cleanup();}
});

test('selection metadata refuses late unsupported synthetic and overflowing epochs without changing native promise outcomes',async()=>{
 for(const options of [{late:true},{unsupported:true},{}]){const f=selectionFixture(options);try{await f.arm();f.select();
  if(!options.late&&!options.unsupported)f.emit('selectedcandidatepairchange',false);
  const rows=(await globalThis.connectPeerMediaDiagnostics())[0];assert(rows.every(row=>!row.selected_path.transport_epoch));
  assert.equal(globalThis.connectPeerPublicBinding('original-connection').bindings.send.complete,false);
 }finally{f.cleanup();}}
 const bounded=selectionFixture();try{await bounded.arm();bounded.select();for(let n=0;n<130;n++)bounded.emit('statechange');
  assert((await globalThis.connectPeerMediaDiagnostics())[0].every(row=>!row.selected_path.transport_epoch));
  assert.equal(globalThis.connectPeerPublicBinding('original-connection').bindings.receive.complete,false);
 }finally{bounded.cleanup();}
 const rejected=selectionFixture();try{const original=rejected.peer.setLocalDescription(rejected.peer.localDescription),error=Error('exact original rejection');assert.equal(original,rejected.original);rejected.reject(error);await assert.rejects(original,value=>value===error);assert.equal(rejected.listenerCount(),0);}finally{rejected.cleanup();}
 const late=selectionFixture();try{const pending=late.peer.setLocalDescription(late.peer.localDescription);assert.equal(pending,late.original);late.peer.close();await late.arm();assert.equal(late.listenerCount(),0,'Late original resolution never rearms a closed peer');}finally{late.cleanup();}
});

test('the original Stats request refuses pending counter state and tuple changes including same tuple selection ABA',async()=>{
 for(const mutation of ['selection','state','tuple','counter']){const f=selectionFixture();try{await f.arm();f.select();
  let release;f.setBlocked(new Promise(ok=>{release=ok;}));const pending=globalThis.connectPeerMediaDiagnostics();assert.equal(f.reads(),1);
  if(mutation==='selection'){f.emit('selectedcandidatepairchange');f.emit('selectedcandidatepairchange');f.setCounter(3);}
  if(mutation==='state')f.emit('statechange');
  if(mutation==='tuple')f.peer.pair.remote.port=40001;
  if(mutation==='counter')f.setCounter(2);
  release(f.report());const rows=(await pending)[0];assert(rows.every(row=>!row.selected_path.transport_epoch),mutation);assert.equal(rows[0].packetsSent,7);
 }finally{f.cleanup();}}
});

test('typed privacy metadata never escapes masked Stats ambiguous transports or contradictory original RTP references',async()=>{
 const changes=[r=>{delete r.get('t').selectedCandidatePairChanges;},r=>{r.get('r').address=null;},r=>{r.get('p').nominated=false;},r=>{r.get('receive').transportId='foreign';},r=>{r.set('t2',{...r.get('t'),id:'t2'});},r=>{r.get('l').port++;}];
 for(const change of changes){const f=selectionFixture();try{await f.arm();f.select();f.setChange(change);const rows=(await globalThis.connectPeerMediaDiagnostics())[0];assert(rows.every(row=>!row.selected_path.transport_epoch));assert.equal(f.reads(),1);}finally{f.cleanup();}}
 const changed=selectionFixture();try{await changed.arm();changed.select();changed.peer.txs[1].receiver.transport={state:'connected',iceTransport:{...changed.peer.ice}};
  assert((await globalThis.connectPeerMediaDiagnostics())[0].every(row=>!row.selected_path.transport_epoch));assert.equal(globalThis.connectPeerPublicBinding('original-connection').bindings.send.complete,false);
 }finally{changed.cleanup();}
});

test('capture phase epoch handler precedes reentrant ordinary callbacks and default mode preserves original promise',async()=>{
 const f=selectionFixture();let reentrant;const ordinary=()=>{if(f.peer.connectionState==='connected')reentrant=globalThis.connectPeerPublicBinding('original-connection');};try{
  f.peer.ice.addEventListener('selectedcandidatepairchange',ordinary);
  await f.arm();f.select();assert.equal(reentrant.bindings.send.selected_udp_pair.transport_epoch.selection_count,1);
  assert.equal(reentrant.bindings.send.selected_udp_pair.transport_epoch.state_version,1,'FIRST trusted selection callback is already counted before ordinary callback');
 }finally{f.peer.ice.removeEventListener('selectedcandidatepairchange',ordinary);f.cleanup();}
 const disabled=terminalFixture({enabled:false});let resolve;const promise=new Promise(ok=>{resolve=ok;});try{const native=Object.getPrototypeOf(Object.getPrototypeOf(disabled.peer));native.setLocalDescription=()=>promise;assert.equal(disabled.peer.setLocalDescription(),promise);resolve();await promise;assert.equal('transport_epoch'in globalThis.connectPeerDiagnostics().peers[0],false);disabled.peer.close();}finally{disabled.restore();}
});


function retirementReceiverReport(peer,stamp){
 const transport='original-transport',pair='original-pair';
 return new Map([
  ['rtp',{id:'rtp',type:'inbound-rtp',kind:'video',trackIdentifier:peer.receiveTrack.id,mid:'1',ssrc:22,timestamp:stamp,packetsReceived:4,framesDecoded:2,codecId:'codec',transportId:transport}],
  ['codec',{id:'codec',type:'codec',payloadType:120,mimeType:'video/VP8',clockRate:90000}],
  [transport,{id:transport,type:'transport',selectedCandidatePairId:pair}],
  [pair,{id:pair,type:'candidate-pair',transportId:transport,nominated:true,state:'succeeded',localCandidateId:'local',remoteCandidateId:'remote'}],
  ['local',{id:'local',type:'local-candidate',transportId:transport,address:'::1',port:50000,protocol:'udp'}],
  ['remote',{id:'remote',type:'remote-candidate',transportId:transport,address:'::1',port:40000,protocol:'udp'}]]);
}
const finishReceiverObservation=async()=>{await Promise.resolve();await Promise.resolve();await Promise.resolve();};

test('one original receiver statistics request survives tracked peer removal without delaying or changing native close',async()=>{
 const f=terminalFixture({serialized:true});let release,calls=0;const original=new Promise(resolve=>{release=resolve;});
 try{
  f.tag();f.peer.txs[1].receiver.getStats=()=>{calls++;assert.equal(f.peer.signalingState,'stable');return original;};
  assert.equal(f.peer.close(),'original-native-close-return');assert.equal(calls,1);assert.equal(f.counts.close,1);
  const pending=f.report().receiver_retirement_statistics;assert.equal(pending.state,'pending');assert.equal(f.report().live_owner_retained,false);
  assert.equal(globalThis.connectPeerStreamDescriptor('original-connection','receive'),null,'The closed original native owner has been removed');
  assert.equal(globalThis.connectPeerRetireCycle().released,false,'An unsettled original query is not a joined owner');
  const stamp=performance.timeOrigin+pending.requested_at_ms,report=retirementReceiverReport(f.peer,stamp);
  delete report.get('original-transport').id;delete report.get('original-pair').id;delete report.get('original-pair').transportId;delete report.get('local').transportId;delete report.get('remote').transportId;
  release(report);await original;await finishReceiverObservation();
  const actual=f.report().receiver_retirement_statistics;assert.equal(actual.state,'fulfilled');assert.equal(actual.complete,true);
  assert.equal(actual.rows[0].stats_timestamp_ms,stamp);assert.equal(actual.rows[0].selected_path.local.address,'::1');assert.equal(actual.rows[0].selected_path.remote.address,'::1');
  assert.equal(actual.rows[0].native_track_identifier,f.peer.receiveTrack.id);assert.equal(actual.rows[0].native_mid,'1');assert(Object.values(actual.rows[0].native_statistics_reference_fields).every(field=>field.present===false&&field.value===null),'Optional native reference fields remain actually absent; graph linkage uses the original report references');
  assert.equal('transport_epoch'in actual.rows[0].selected_path,false,'No freshly closed or inherited selection epoch is stamped on an async report');
  assert(actual.requested_at_ms<=actual.native_closed_at_ms&&actual.native_closed_at_ms<=actual.completed_at_ms);
  assert.equal(actual.native_close_returned,true);assert.equal(actual.native_closed,true);
  assert.equal(pending.state,'pending','An earlier exported snapshot cannot mutate after settlement');
  report.get('rtp').ssrc=999;assert.equal(f.report().receiver_retirement_statistics.rows[0].ssrc,22);
  f.peer.close();assert.equal(calls,1);assert.deepEqual(f.report().receiver_retirement_statistics,actual,'A later close cannot replace the original close observation');
  assert.equal(globalThis.connectPeerRetireCycle().released,true);
 }finally{release?.(new Map());await finishReceiverObservation();f.restore();}
});

test('rejected partial competing and changed native receiver requests remain explicit failures with original cleanup',async()=>{
 for(const mode of ['reject','partial','competing','foreign-track','changed-during-request','contradictory-reference']){
  const f=terminalFixture();let calls=0;
  try{
   f.tag();f.peer.txs[1].receiver.getStats=()=>{calls++;if(mode==='reject')return Promise.reject(Error('PRIVATE-QUERY-ERROR'));
    const report=retirementReceiverReport(f.peer,performance.timeOrigin+performance.now());
    if(mode==='partial')report.delete('codec');if(mode==='competing')report.set('other',{...report.get('rtp'),ssrc:44});
    if(mode==='foreign-track')report.get('rtp').trackIdentifier='foreign';if(mode==='contradictory-reference')report.get('local').transportId='foreign-transport';
    if(mode==='changed-during-request')f.peer.txs[1].receiver.track={...f.peer.receiveTrack};return Promise.resolve(report);};
   assert.equal(f.peer.close(),'original-native-close-return');await finishReceiverObservation();
   const actual=f.report().receiver_retirement_statistics;assert.equal(calls,1);assert.equal(actual.complete,false);assert(actual.reason);
   assert.equal(actual.state,mode==='reject'?'rejected':'fulfilled');assert.equal(JSON.stringify(actual).includes('PRIVATE-QUERY-ERROR'),false);
   assert.equal(actual.native_close_returned,true);assert.equal(actual.native_closed,true);assert.equal(globalThis.connectPeerRetireCycle().released,true);
  }finally{f.restore();}
 }
});

test('default and untagged diagnostics do not create a receiver request and pending retirement remains uncertain',async()=>{
 for(const enabled of [false,true]){const f=terminalFixture({enabled});let calls=0;
  try{f.peer.txs[1].receiver.getStats=()=>{calls++;return new Promise(()=>{});};f.peer.close();assert.equal(calls,0);
   if(!enabled)assert.equal('receiver_retirement_statistics'in f.report(),false);else assert.equal(f.report().receiver_retirement_statistics.state,'not-requested');
  }finally{f.restore();}}
 const f=terminalFixture();let release;
 try{f.tag();f.peer.txs[1].receiver.getStats=()=>new Promise(resolve=>{release=resolve;});f.peer.close();
  assert.equal(f.report().receiver_retirement_statistics.state,'pending');assert.equal(globalThis.connectPeerRetireCycle().released,false);
  release(retirementReceiverReport(f.peer,performance.timeOrigin+performance.now()));await finishReceiverObservation();assert.equal(globalThis.connectPeerRetireCycle().released,true);
 }finally{release?.(new Map());await finishReceiverObservation();f.restore();}
});

test('a failed original native close is not upgraded by a later close or successful receiver statistics result',async()=>{
 const f=terminalFixture();let calls=0;
 try{f.tag();f.peer.txs[1].receiver.getStats=()=>{calls++;return Promise.resolve(retirementReceiverReport(f.peer,performance.timeOrigin+performance.now()));};
  const original=Error('original-native-close');f.peer.closeError=original;assert.throws(()=>f.peer.close(),error=>error===original);
  delete f.peer.closeError;f.peer.close();await finishReceiverObservation();const actual=f.report().receiver_retirement_statistics;
  assert.equal(calls,1);assert.equal(actual.native_close_returned,false);assert.equal(actual.native_closed,false);
  assert.equal(globalThis.connectPeerRetireCycle().released,true,'All original work settled, but this measurement cannot claim a confirmed close');
 }finally{delete f.peer.closeError;f.peer.close();await finishReceiverObservation();f.restore();}
});
