import {test} from 'node:test';
import assert from 'node:assert/strict';
import {matrixAudioProfile} from '../lib/browser_matrix_sources.mjs';
import {pcmIdentity} from '../lib/browser_matrix_observation.mjs';
import {AudioEnvelope} from '../video/audio_envelope.mjs';
const tones=[440,660,880,1100];
function sweep(high){let bad=0,wrongAccepted=0,minimumRatio=Infinity;
 for(const [id,hz]of tones.entries())for(const falling of[false,true])for(const phase of Array.from({length:16},(_,i)=>i/16))for(let onset=0;onset<=960;onset+=8){
  const samples=Float32Array.from({length:960},(_,i)=>(falling?(i<onset?high:matrixAudioProfile.lowPeak):(i<onset?matrixAudioProfile.lowPeak:high))*Math.sin(2*Math.PI*(hz*i/48000+phase)));
  const actual=pcmIdentity(samples,id),wrong=pcmIdentity(samples,(id+1)%4);if(!actual.identified)bad++;if(wrong.identified)wrongAccepted++;
  minimumRatio=Math.min(minimumRatio,actual.powers[id]/Math.max(...actual.powers.filter((_,i)=>i!==id)));
 }
 return {bad,wrongAccepted,minimumRatio};
}
test('clean20ms source blocks remain independently identifiable across pulse offsets and phases',()=>{
 const original=sweep(.7);assert.ok(original.bad>0,'Original14x step reproduces ambiguity before codecs');
 const actual=sweep(matrixAudioProfile.highPeak);assert.equal(actual.bad,0);assert.equal(actual.wrongAccepted,0);assert.ok(actual.minimumRatio>8);
 assert.equal(pcmIdentity(new Float32Array(960),0).identified,false);
});
test('source envelope keeps both existing RMS decisions and fixed100ms high interval',()=>{
 const envelope=new AudioEnvelope();for(let i=0;i<40;i++){const at=i*.02;envelope.sample(at,(at>=.3&&at<.4?matrixAudioProfile.highPeak:matrixAudioProfile.lowPeak)/Math.sqrt(2));}
 const report=envelope.report();assert.equal(report.status,'observer-valid');assert.deepEqual(report.accepted_edge_seconds,[.3]);
 assert.equal(report.high_rms_above,.055);assert.equal(report.low_rms_at_most,.045);assert.equal(report.fixed_source_pulse_seconds,.1);
});

// Full original PCM enters the SAME exported measurement used by DecodedRoute.
// These are constructed source-estimator controls, not actual codec/cell proof.
test('all authored carrier phases preserve strict source identity and quiet availability',()=>{
 for(const [source,hz]of tones.entries())for(let phase=0;phase<16;phase++)for(const peak of[.0005,.05,.2]){
  const samples=Float32Array.from({length:960},(_,i)=>peak*Math.sin(2*Math.PI*(hz*i/48000+phase/16)));
  const actual=pcmIdentity(samples,source),detail=JSON.stringify({source,hz,phase,peak});
  assert.equal(actual.source_identified,true,detail);assert.equal(actual.identified,peak!==.0005,detail);
  for(let other=0;other<4;other++)if(other!==source){const wrong=pcmIdentity(samples,other);
   assert.equal(wrong.source_identified,false,detail+' foreign='+other);assert.equal(wrong.identified,false);}
 }
});

test('forbidden high frequency fold partners cannot identify any authored source',()=>{
 const observations=[];
 for(const [source,hz]of tones.entries()){
  const actualHz=6000-hz;let claims=0,usableClaims=0,blocks=0;
  for(let phase=0;phase<16;phase++)for(const peak of[.05,.2]){
   const samples=Float32Array.from({length:960},(_,i)=>peak*Math.sin(2*Math.PI*(actualHz*i/48000+phase/16)));
   const actual=pcmIdentity(samples,source);claims+=Number(actual.source_identified);usableClaims+=Number(actual.identified);blocks++;
  }
  observations.push({source,actualHz,forbiddenClaimHz:hz,blocks,claims,usableClaims});
 }
 // Evaluate ALL four exact aliases before the OLD-counter oracle. Keeping
 // energetic RMS and ratio8 does not authorize a 4.9--5.56kHz foreign carrier.
 assert.deepEqual(observations.map(row=>row.claims),[0,0,0,0],JSON.stringify({profile:'connect/pcm-fullrate-fold-counter/1',observations}));
 assert.deepEqual(observations.map(row=>row.usableClaims),[0,0,0,0]);
});

test('zero noise mixtures comparison floor and malformed PCM remain fail closed',()=>{
 const zero=new Float32Array(960),tinyNoise=Float32Array.from({length:960},(_,i)=>(((i*73)%127)-63)*1e-9);
 const mixture=Float32Array.from({length:960},(_,i)=>tones.reduce((sum,hz)=>sum+.05*Math.sin(2*Math.PI*hz*i/48000),0));
 for(let source=0;source<4;source++){
  const belowComparisonFloor=Float32Array.from({length:960},(_,i)=>1e-8*Math.sin(2*Math.PI*tones[source]*i/48000));
  for(const samples of[zero,tinyNoise,mixture,belowComparisonFloor]){const actual=pcmIdentity(samples,source);
   assert.equal(actual.source_identified,false,JSON.stringify({source}));assert.equal(actual.identified,false);}
 }
 for(const length of[0,959,961])assert.throws(()=>pcmIdentity(new Float32Array(length),0),/Mono 20ms source identity/);
 for(const expected of[-1,4,.5,null])assert.throws(()=>pcmIdentity(zero,expected),/Mono 20ms source identity/);
 for(const index of[0,1,7,8,959])for(const invalid of[NaN,Infinity,-Infinity]){
  const samples=new Float32Array(960);samples[index]=invalid;assert.throws(()=>pcmIdentity(samples,0),/Finite decoded PCM/);
 }
});
