import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import {markerFrames,ordinalSync} from '../video/marker_calibration.mjs';
const actual=JSON.parse(fs.readFileSync(new URL('./av_skew_roundoff_observed.json',import.meta.url),'utf8'));
const schedule=markerFrames().map(frame=>frame/30).filter(at=>at>=1&&at<60);
const percentile=offset=>ordinalSync(schedule,schedule.map((at,i)=>at+(i%10===0?offset:.03)),60);

test('actual Mac Firefox cadence arithmetic boundary retains original edges and pairs',()=>{
    const before=structuredClone(actual),result=ordinalSync(actual.video,actual.audio,60);
    const raw=actual.raw_pairs.map(pair=>Math.abs(pair.signed_video_minus_audio_ms)).sort((a,b)=>a-b);
    assert.equal(raw[Math.ceil(raw.length*.95)-1],120.000000000001);
    assert.equal(result.status,'passed');assert.equal(result.p95_abs_skew_ms,120);
    assert.equal(result.quantile_roundoff.raw_p95_abs_skew_ms,actual.raw_p95_ms);
    assert.equal(result.quantile_roundoff.p95_normalized,true);
    assert.deepEqual(result.marker_pairs,actual.raw_pairs);
    assert.deepEqual(result.video_edge_seconds,actual.video);assert.deepEqual(result.audio_edge_seconds,actual.audio);
    assert.deepEqual(actual,before);assert.deepEqual(result.limits,{median_abs_skew_ms:80,p95_abs_skew_ms:120});
});
test('p95 roundoff cannot admit meaningful excesses or the other actual sync failures',()=>{
    assert.equal(percentile(.12).status,'passed');
    for(const offset of [.12000001,.123,.1241]){
        const result=percentile(offset);assert.equal(result.status,'failed');assert(result.p95_abs_skew_ms>120);
        assert.equal(result.quantile_roundoff.p95_normalized,false);
    }
});
test('median boundary preserves80ms and rejects80.00000001 and80.00001ms',()=>{
    assert.equal(ordinalSync(schedule,schedule.map(at=>at+.08),60).status,'passed');
    for(const offset of [.08000000001,.08000001]){
        const result=ordinalSync(schedule,schedule.map(at=>at+offset),60);
        assert.equal(result.status,'failed');assert(result.median_abs_skew_ms>80);
        assert.equal(result.quantile_roundoff.median_normalized,false);
    }
});
test('numeric normalization adds no ordinal proof or absolute-clock allowance',()=>{
    assert.equal(ordinalSync([1,2],[1,2],60).status,'failed');
    assert.equal(ordinalSync(schedule,schedule.map(()=>NaN),60).status,'failed');
    const result=ordinalSync(schedule.map(at=>at+1000),schedule.map(at=>at+1000+.12),60);
    assert.equal(result.quantile_roundoff.p95_error_bound_ms,0);
    assert.equal(result.quantile_roundoff.p95_normalized,false);
});
