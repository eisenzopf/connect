import {test} from 'node:test';
import assert from 'node:assert/strict';
import {matrixAudioProfile} from '../lib/browser_matrix_sources.mjs';
import {pcmIdentity} from '../lib/browser_matrix_observation.mjs';
import {AudioEnvelope} from '../video/audio_envelope.mjs';
const tones=[440,660,880,1100];
function sweep(high){let bad=0,wrongAccepted=0,minimumRatio=Infinity;
 for(const [id,hz]of tones.entries())for(const falling of[false,true])for(const phase of[0,.25,.5,.75])for(let onset=0;onset<960;onset+=8){
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
