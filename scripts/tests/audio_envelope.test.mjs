/** Fixed physical envelope guards; calibration and AV limits remain unchanged. */
import assert from 'node:assert/strict';import test from 'node:test';
import {AudioEnvelope as EnvelopeEdges,combineAudioEnvelopeSync,audioVideoSyncPassed} from '../video/audio_envelope.mjs';
import {markerFrames,calibrate,ordinalSync} from '../video/marker_calibration.mjs';
function sampled(pulses,seconds,modify=()=>null){const d=new EnvelopeEdges();for(let i=0;i<=Math.round(seconds*100);i++){const t=i/100;d.sample(t,modify(t)??(pulses.some(p=>t+1e-8>=p&&t<p+.1-1e-8)?.49:.035));}return d;}
const schedule=markerFrames().map(f=>f/30).filter(t=>t<60);
test('every genuine minimum500ms cadence rise retains original first high timestamp',()=>{const p=Array.from({length:12},(_,i)=>.3+i*.5);assert.deepEqual(sampled(p,6.2).edges,p.map(t=>Math.round(t*100)/100));});
test('initial high crop is unproven; later low-rearmed pulse is proven',()=>{assert.deepEqual(sampled([0,.5,1],1.2).edges,[.5,1]);});
test('within100ms pulse dip and short trailing threshold bounce do not rearm',()=>{const d=sampled([.3,.8,1.3],1.5,t=>(t>=.34&&t<.36)? .03:(t>=.45&&t<.46)? .06:null);assert.deepEqual(d.edges,[.3,.8,1.3]);assert.equal(d.transitions.filter(x=>x.reason==='continuous low not proven').length,2);assert(d.transitions.some(x=>x.at===.45&&x.rms===.06&&!x.accepted));});
test('same nonperiodic schedule calibrates without altered150ms tolerance or80/120 limits',()=>{const d=sampled(schedule,60);const c=calibrate(d.edges);assert.equal(c.status,'calibrated');const r=ordinalSync(d.edges,d.edges,60);assert.equal(r.status,'passed');assert.deepEqual(r.limits,{median_abs_skew_ms:80,p95_abs_skew_ms:120});assert.equal(r.calibration_tolerance_seconds,.15);});
test('separate genuine extra pulse remains visible and fails original calibration',()=>{const p=[...schedule,15.3].sort((a,b)=>a-b);const d=sampled(p,60);assert(d.edges.includes(15.3));assert.equal(calibrate(d.edges).status,'missing-proof');});
test('loss is explicitly missing rather than reconstructed',()=>{const p=schedule.filter((_,i)=>i!==25),d=sampled(p,60),c=calibrate(d.edges);assert.equal(c.status,'calibrated');assert.equal(c.missing_internal_markers,1);assert.equal(d.edges.length,schedule.length-1);});
test('progressive clock drift and1second AV displacement still fail original proof',()=>{const original=sampled(schedule,60),drift=sampled(schedule.map((t,i)=>t+i*.01),61);assert.equal(calibrate(drift.edges).status,'missing-proof');const shifted=sampled(schedule.map(t=>t+1),61);assert.equal(ordinalSync(original.edges,shifted.edges,60).status,'failed');});
test('missing observer interval cannot establish continuous low; diagnostic retained',()=>{const d=new EnvelopeEdges();d.sample(0,.035);d.sample(.01,.035);d.sample(.3,.035);d.sample(.31,.49);assert.deepEqual(d.edges,[]);assert(d.transitions.some(x=>x.gap_seconds>.05));const invalid=new EnvelopeEdges();invalid.sample(0,NaN);assert.equal(invalid.report().status,'observer-invalid');const bounded=new EnvelopeEdges();for(let i=0;i<2100;i++)bounded.sample(i/1000,i%2?.1:.01);assert.equal(bounded.transitions.length,2048);assert.equal(bounded.report().status,'observer-invalid');});

test('prolonged PLC high retains the original onset and reports its duration diagnostically',()=>{const d=sampled([.3,1.3,1.8],2,t=>t>=.3&&t<.9?.49:null);assert.deepEqual(d.edges,[.3,1.3,1.8]);assert.equal(d.report().status,'observer-valid');assert.equal(d.report().extended_envelopes.length,1);assert.equal(d.report().extended_envelopes[0].onset,.3);});
test('stuck high supplies no repeated pulses and cannot pass the unchanged AV proof',()=>{const d=sampled([],60,t=>t>=.3?.49:null);assert.deepEqual(d.edges,[.3]);assert.equal(d.report().status,'observer-valid');assert.equal(d.report().extended_envelopes.length,1);assert.equal(audioVideoSyncPassed(combineAudioEnvelopeSync(ordinalSync(d.edges,d.edges,60),d.report()),60),false);});
test('late spurious rebound without continuous low is retained but never accepted as a new onset',()=>{const d=sampled([.3,1.3],1.5,t=>(t>=.55&&t<.58)||(t>=.75&&t<.78)?.06:null);assert.deepEqual(d.edges,[.3,1.3]);assert.equal(d.report().status,'observer-valid');assert(d.transitions.some(x=>x.at===.55&&!x.accepted&&x.rms===.06));assert(d.transitions.some(x=>x.at===.75&&!x.accepted&&x.rms===.06));assert.equal(d.report().extended_envelopes.length,1);});
test('clean coded programme and short within-pulse bounce have unambiguous envelopes',()=>{assert.equal(sampled(schedule,60).report().status,"observer-valid");const d=sampled([.3,.8,1.3],1.5,t=>(t>=.34&&t<.36)? .03:(t>=.45&&t<.46)? .06:null);assert.equal(d.report().status,"observer-valid");});

test('browser verdict rejects invalid observer despite otherwise passing ordinal metrics',()=>{
 const clean=sampled(schedule,60),valid=ordinalSync(clean.edges,clean.edges,60);
 const envelope=clean.report();
 assert.equal(audioVideoSyncPassed(combineAudioEnvelopeSync(valid,envelope),60),true);
 const broken=sampled(schedule,60);broken.sample(60.01,NaN);const invalid=broken.report();
 const combined=combineAudioEnvelopeSync(valid,invalid);
 assert.equal(combined.status,'failed');
 assert.equal(combined.paired,valid.paired);
 assert.deepEqual(combined.marker_pairs,valid.marker_pairs);
 assert.equal(audioVideoSyncPassed(combined,60),false);
 assert.equal(audioVideoSyncPassed(combineAudioEnvelopeSync(valid,undefined),60),false);
 assert.equal(audioVideoSyncPassed(combineAudioEnvelopeSync({...valid,status:'failed'},envelope),60),false);
 assert.equal(audioVideoSyncPassed(combineAudioEnvelopeSync({...valid,median_abs_skew_ms:null},envelope),60),false);
});
