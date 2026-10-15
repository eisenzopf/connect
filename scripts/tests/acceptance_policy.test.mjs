import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import {ordinalSync} from "../video/marker_calibration.mjs";
import {audioVideoAcceptance} from "../video/acceptance_policy.mjs";
import {audioVideoSyncPassed} from "../video/audio_envelope.mjs";

const fixtures=JSON.parse(fs.readFileSync(new URL("./av_acceptance_cases.json",import.meta.url),"utf8"));
const observation=row=>{
    const variant=fixtures.variants[row.variant];
    const sync=ordinalSync(variant.video_edges,variant.audio_edges,row.seconds);
    for(const [path,value] of row.changes){
        const keys=path.split(".");let target=sync;
        for(const key of keys.slice(0,-1))target=target[key];
        target[keys.at(-1)]=structuredClone(value);
    }
    return sync;
};
for(const row of fixtures.cases)test(row.name,()=>{
    const sync=observation(row),before=structuredClone(sync);
    assert.deepEqual(audioVideoAcceptance(sync,row.scenario,row.seconds),row.expected);
    assert.deepEqual(sync,before,"acceptance must not rewrite raw observations");
});

test("nonfinite, oversized and nonnumeric raw input never becomes impaired acceptance",()=>{
    const base=fixtures.cases[0];
    for(const value of [NaN,Infinity,"0",true]){
        const sync=observation(base);sync.video_edge_seconds[0]=value;
        assert.equal(audioVideoAcceptance(sync,"impairment",60).status,"failed");
    }
    const sync=observation(base);sync.video_edge_seconds=Array.from({length:513},(_,i)=>i);sync.video_markers=513;
    assert.equal(audioVideoAcceptance(sync,"impairment",60).reason,"invalid-observation");
    for(const seconds of [NaN,Infinity,true,"60",121])assert.equal(audioVideoAcceptance(observation(base),"impairment",seconds).reason,"invalid-duration");
});

test("cross-language comparison tolerance cannot relax strict skew thresholds",()=>{
    const variant=fixtures.variants.aligned;
    const sync=ordinalSync(variant.video_edges,variant.audio_edges.map(at=>at+.08000000001),60);
    sync.median_abs_skew_ms=80;
    assert.equal(audioVideoAcceptance(sync,"clean",60).status,"failed");
});

test("legacy strict helper still rejects degraded impaired-quality evidence",()=>{
    const row=fixtures.cases.find(row=>row.name==="impairment offset"),sync=observation(row);
    sync.audio_envelope={status:"observer-valid",invalid:[],overflow:false,invalid_input:false};
    assert.equal(audioVideoAcceptance(sync,"impairment",60).status,"passed");
    assert.equal(audioVideoSyncPassed(sync,60),false);
});

test("actual browser verdict retains every non-AV hard gate under degraded acceptance",()=>{
    const source=fs.readFileSync(new URL("../video/browser-qualify.mjs",import.meta.url),"utf8");
    const start=source.indexOf("const passed=observations.every(");
    const stop=source.indexOf(";\n    console.log",start);
    assert(start>=0&&stop>start);
    const verdict=new Function("observations","spec","pages","teardown","ended","pairedTermination",
        `return ${source.slice(start+"const passed=".length,stop)};`);
    const sync=observation(fixtures.cases.find(row=>row.name==="impairment offset"));
    sync.audio_envelope={status:"observer-valid",invalid:[],overflow:false,invalid_input:false};
    const base={worker_pipeline:{status:"passed"},motion_profile:{status:"passed"},audio_profile:{status:"passed"},
        source_identity:{status:"passed"},stats_available:true,decoded:300,changing:300,elapsed_seconds:60,
        codec:"video/VP8",width:640,height:360,audio_rms:.02,audio_frequency_hz:440,expected_audio_frequency_hz:440,
        av_sync:sync,av_acceptance:audioVideoAcceptance(sync,"impairment",60),recovery:[{status:"passed"}]};
    const run=(o,teardown=.1,ended=[true],paired={status:"passed"})=>verdict([o],{profileMode:"impairment",seconds:60},[{}],teardown,ended,paired);
    assert.equal(run(base),true);
    for(const mutate of [o=>o.worker_pipeline.status="failed",o=>o.motion_profile.status="failed",
        o=>o.audio_profile.status="failed",o=>o.source_identity.status="failed",o=>o.stats_available=false,
        o=>o.decoded=0,o=>o.changing=0,o=>o.width=320,o=>o.codec="video/H264",o=>o.audio_rms=0,
        o=>o.audio_frequency_hz=880,o=>o.av_sync.audio_envelope.status="observer-invalid",
        o=>o.av_acceptance.status="failed",o=>o.recovery[0].status="failed"]){
        const changed=structuredClone(base);mutate(changed);assert.equal(run(changed),false);
    }
    assert.equal(run(base,5),false);
    assert.equal(run(base,.1,[false]),false);
    assert.equal(run(base,.1,[true],{status:"failed"}),false);
});
