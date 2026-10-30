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
                pliCount:Infinity,firCount:-1,retransmittedPacketsSent:"private-secret",
            }]));
        }
    }
    try{
        globalThis.RTCPeerConnection=Peer;installPeerDiagnostics();
        for(let index=0;index<5;index++)new globalThis.RTCPeerConnection();
        const value=await globalThis.connectPeerMediaDiagnostics();
        assert.equal(value.length,4);assert(value.every(rows=>rows.length===8));
        assert.deepEqual(value[0][0],{type:"inbound-rtp",kind:"audio",packetsLost:2,nackCount:4,
            concealedSamples:960,silentConcealedSamples:480,concealmentEvents:2,
            jitterBufferDelay:.1,jitterBufferEmittedCount:960});
        assert.equal(JSON.stringify(value).includes("private-secret"),false);
    }finally{
        globalThis.RTCPeerConnection=original;
        delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;
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
        delete globalThis.connectPeerDiagnostics;delete globalThis.connectPeerMediaDiagnostics;delete globalThis.connectPeerCodecDiagnostics;
    }
});
