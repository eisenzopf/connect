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
        assert.deepEqual(value[0][0],{type:"inbound-rtp",kind:"audio",packetsLost:2,nackCount:4,
            concealedSamples:960,silentConcealedSamples:480,concealmentEvents:2,
            jitterBufferDelay:.1,jitterBufferEmittedCount:960,jitterBufferTargetDelay:.2,
            jitterBufferMinimumDelay:.05,totalDecodeTime:.01,totalProcessingDelay:.03,
            totalInterFrameDelay:1,totalSquaredInterFrameDelay:.04,freezeCount:1,totalFreezesDuration:.4});
        assert.equal(JSON.stringify(value).includes("private-secret"),false);
    }finally{
        globalThis.RTCPeerConnection=original;
        delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;delete globalThis.connectPeerClockDiagnostics;
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
        delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;delete globalThis.connectPeerClockDiagnostics;
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
        for(const name of ['connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'])delete globalThis[name];
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
        for(const name of ['connectPeerDiagnostics','connectPeerMediaDiagnostics','connectPeerCodecDiagnostics','connectPeerClockDiagnostics'])delete globalThis[name];
    }
});
