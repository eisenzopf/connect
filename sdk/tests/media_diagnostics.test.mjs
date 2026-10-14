import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { decodeResponse } from '../typescript/dist/protocol.js';
import { validMediaDiagnostics } from '../typescript/dist/media_diagnostics.js';
const fixture=JSON.parse(readFileSync(new URL('../../contracts/fixtures/media-diagnostics-v1.json', import.meta.url),'utf8'));
const congestion={"congestion_samples": 2, "maximum_sampled_connection_lost_packets": 11, "maximum_sampled_connection_congestion_events": 9, "maximum_sampled_connection_sent_packets": 496, "minimum_sampled_rtt_us": 47000, "maximum_sampled_rtt_us": 54000, "minimum_sampled_cwnd_bytes": 6000, "maximum_sampled_cwnd_bytes": 12000};
const decode=media=>decodeResponse({profile:'connect-control/1',result:{Ok:{kind:'health',data:{installation_id:'fixture',schema:17,profile:'connect-control/1',readiness:{state:'ready',components:[],media}}}}},'health');
test('health validates bounded diagnostics without treating bucket percentiles as exact latency',()=>{
    assert.equal(validMediaDiagnostics(fixture),true);
    assert.deepEqual(decode(fixture).data.readiness.media,fixture);
    const current=structuredClone(fixture);Object.assign(current.quic,congestion);
    assert.deepEqual(decode(current).data.readiness.media,current);
    const noQuic=structuredClone(fixture);delete noQuic.quic;
    assert.equal(validMediaDiagnostics(noQuic),true);
    delete noQuic.webrtc_video_input;
    noQuic.queues[0].max_forwarded_frame_age_ms=null;
    noQuic.queues[0].frame_age_boundary=null;
    assert.equal(validMediaDiagnostics(noQuic),true);
});
test('malformed counters, unbounded samples and extra sensitive diagnostic fields fail closed',()=>{
    const variants=[];
    for (const counter of [true,-1,0.5,Number.MAX_SAFE_INTEGER+1,null]) {
        const copy=structuredClone(fixture);copy.quic.too_large=counter;variants.push(copy);
    }
    for(const change of [
        value=>{value.queues[0].sampled_high_water_packets=65;},
        value=>{value.queues=Array(129).fill(value.queues[0]);},
        value=>{value.queues[0].frame_age_boundary=null;},
        value=>{value.forwarding[0].p50_us=256;},
        value=>{value.quic.remote_address='private-diagnostic-secret';},
        value=>{value.queues[0].payload='private-diagnostic-secret';},
        value=>{value.credential='private-diagnostic-secret';},
        value=>{value.webrtc_video_input.source_packets=true;},
        value=>{value.webrtc_video_input.payload='private-diagnostic-secret';},
    ]) {const copy=structuredClone(fixture);change(copy);variants.push(copy);}
    for (const [key,bad] of [['congestion_samples',true],['minimum_sampled_rtt_us',90000],['maximum_sampled_connection_lost_packets',-1],['minimum_sampled_cwnd_bytes',null]]) {
        const copy=structuredClone(fixture);Object.assign(copy.quic,congestion);copy.quic[key]=bad;variants.push(copy);
    }
    const partial=structuredClone(fixture);partial.quic.congestion_samples=1;variants.push(partial);
    for(const value of variants) {
        assert.equal(validMediaDiagnostics(value),false);
        assert.throws(()=>decode(value), error=>error.code==='protocol'&&!error.message.includes('private-diagnostic-secret'));
    }
});
