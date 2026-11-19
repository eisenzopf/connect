import {test} from 'node:test';
import assert from 'node:assert/strict';
import {pcmIdentity,strictPcmIdentity,PcmSampleLedger,pcmSampleCoverage} from '../lib/browser_matrix_pcm.mjs';
import {transitionCoverage} from '../lib/media_transition_coverage.mjs';
const tones=[440,660,880,1100];
const tone=(hz,phase=0,start=832,end=960,amplitude=.15)=>Float32Array.from({length:960},(_,i)=>i>=start&&i<end?amplitude*Math.sin(2*Math.PI*hz*(i-start)/48000+phase):0);
const clock=(owner='original',epoch=1)=>({profile:'connect/rtc-decoded-pcm-clock/1',owner,epoch_id:epoch});
const push=(ledger,samples,{owner='original',epoch=1,timestamp=0,at=0,expected=1,publicInterval=0,...options}={})=>ledger.push({owner,clock:clock(owner,epoch),timestamp,at,identity:strictPcmIdentity(samples,expected),publicInterval,...options});
const coverage=(ledger,{startMs=0,endMs=1000,cuts=[]}={})=>{
 const rows=ledger.rows.filter(row=>row.at>=startMs&&row.at<endMs),times=rows.filter(row=>row.identity.identified&&!row.duplicate).map(row=>row.at);
 return pcmSampleCoverage(rows,transitionCoverage(times,cuts,{startMs,endMs,frameMs:20,minimumGapMs:50}),{startMs,endMs});
};
test('all four source tones/all16phases retain full original960 samples, legacy refusal and8x+residual finite-support proof',()=>{
 for(let expected=0;expected<4;expected++)for(let phase=0;phase<16;phase++){
  const samples=tone(tones[expected],phase*Math.PI/8),original=samples.slice(),legacy=pcmIdentity(samples,expected),identity=strictPcmIdentity(samples,expected);
  assert.equal(legacy.source_identified,false);assert.deepEqual(samples,original);assert.deepEqual(identity.legacy,legacy);
  assert.equal(identity.source_identified,true);assert.equal(identity.finite_support.used,true);
  assert.ok(identity.powers[expected]>8*Math.max(identity.finite_support.residual_power,...identity.powers.filter((_,i)=>i!==expected),1e-12));
  assert.ok(identity.finite_support.minimum_relative_pivot>=1e-4);
  const ledger=new PcmSampleLedger();push(ledger,samples,{expected});
  const prefix=samples.findIndex(x=>x!==0);assert.equal(ledger.report().available_samples,960-prefix);
  assert.equal(ledger.report().unavailable_samples,prefix);assert.equal(identity.original_samples,960);
 }
});
test('every foreign partial source code remains refused',()=>{
 for(let expected=0;expected<4;expected++)for(let actual=0;actual<4;actual++)if(expected!==actual)
  assert.equal(strictPcmIdentity(tone(tones[actual]),expected).source_identified,false);
});
test('equal660/880 finite-support contributors remain ambiguous',()=>{
 const mixture=tone(660),other=tone(880);for(let i=0;i<960;i++)mixture[i]+=other[i];
 assert.equal(strictPcmIdentity(mixture,1).source_identified,false);
});
test('alias-prone5340Hz cannot be relabeled660Hz by finite-support fitting',()=>{
 assert.equal(strictPcmIdentity(tone(5340),1).source_identified,false);
});
test('original broadband noise remains unidentified after the finite-support residual check',()=>{
 let seed=93;const noise=Float32Array.from({length:960},(_,i)=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return i<832?0:(seed/2**32-.5)*.4;});
 assert.equal(strictPcmIdentity(noise,1).source_identified,false);
});
test('legacy-recognized660 with an interior erasure retains identity and debits ALL513originalzero samples',()=>{
 const samples=tone(660,0,128,832);samples.fill(0,320,576);
 const legacy=pcmIdentity(samples,1),identity=strictPcmIdentity(samples,1),ledger=new PcmSampleLedger();
 assert.equal(legacy.source_identified,true,'ActualG551 causal check: blanket refusal was an incorrect test oracle');
 assert.deepEqual(identity.legacy,legacy);assert.deepEqual(identity.powers,legacy.powers);
 assert.equal(identity.finite_support.used,false);assert.equal(identity.source_identified,true);
 const row=push(ledger,samples);assert.equal(row.available_samples,447);assert.equal(ledger.report().unavailable_samples,513);
 assert.deepEqual(row.unavailable_runs,[{start_sample:0,end_sample_exclusive:129},
  {start_sample:320,end_sample_exclusive:576},{start_sample:832,end_sample_exclusive:960}]);
});
test('initially unrecognized fullspan PCM with an interior erasure gains NO edge-support fallback',()=>{
 const samples=Float32Array.from({length:960},(_,i)=>i>=320&&i<576?0:(i%2?.15:-.15));
 const legacy=pcmIdentity(samples,1),identity=strictPcmIdentity(samples,1),ledger=new PcmSampleLedger();
 assert.equal(legacy.source_identified,false);assert.equal(identity.finite_support.used,false);
 assert.equal(identity.source_identified,false);push(ledger,samples);assert.equal(ledger.report().available_samples,0);
});
test('unsupported/rank-poor spans, unknown full blocks, literalzero and malformed PCM fail closed',()=>{
 for(const n of[0,8,16,20,32,63]){
  const identity=strictPcmIdentity(n?tone(660,.3,960-n):new Float32Array(960),1);
  assert.equal(identity.source_identified,false);assert.equal(identity.finite_support.used,false);
 }
 const unknown=tone(770,.3,0);assert.equal(strictPcmIdentity(unknown,1).source_identified,false);
 assert.equal(strictPcmIdentity(tone(440,.3,0,960,.0001),1).source_identified,false,'Quiet foreign tone never gains expected-source authority');
 const quiet=strictPcmIdentity(tone(660,.3,832,960,.0001),1);assert.equal(quiet.source_identified,true);assert.equal(quiet.identified,false);
 const invalid=tone(660);invalid[900]=NaN;assert.throws(()=>strictPcmIdentity(invalid,1),/Finite/);
 assert.throws(()=>strictPcmIdentity(new Float32Array(959),1),/Mono/);
 assert.throws(()=>strictPcmIdentity(tone(660),1,{competitors:[330,330]}),/Unique/);
});
test('legacy recognized full rows keep exact verdict, power array and energy-floor arithmetic',()=>{
 for(let expected=0;expected<4;expected++)for(const amplitude of[.15,.0005]){
  const samples=tone(tones[expected],.3,0,960,amplitude),legacy=pcmIdentity(samples,expected),strict=strictPcmIdentity(samples,expected);
  assert.deepEqual(strict.legacy,legacy);assert.deepEqual(strict.powers,legacy.powers);
  assert.equal(strict.source_identified,legacy.source_identified);assert.equal(strict.identified,legacy.identified);assert.equal(strict.finite_support.used,false);
 }
});
test('Native partial positive retains actual330/440 competitors,8x dominance and at least half total-energy explanation',()=>{
 const qualified=samples=>{const identity=strictPcmIdentity(samples,1,{competitors:[330]}),native=identity.extra_powers.find(row=>row.hz===330).power;
  return identity.identified&&identity.powers[1]>8*Math.max(native,identity.powers[0],1e-12)&&identity.expected_explained_energy_fraction>=.5;};
 for(let phase=0;phase<16;phase++)assert.equal(qualified(tone(660,phase*Math.PI/8)),true);
 for(const foreign of[330,440])assert.equal(qualified(tone(foreign)),false);
 const mixed=tone(660),native=tone(330,0,832,960,.1);for(let i=0;i<960;i++)mixed[i]+=native[i];
 assert.equal(qualified(mixed),false,'Real Native330 component is not omitted from the joint bank');
});
test('sixty-second-equivalent quantized waveform crossings retain only the honest initial unknown-neighbor debit',()=>{
 const ledger=new PcmSampleLedger();
 for(let block=0;block<5;block++){
  const samples=Float32Array.from({length:960},(_,i)=>Math.round(.15*32768*Math.sin(2*Math.PI*660*(block*960+i)/48000))/32768);
  push(ledger,samples,{timestamp:block*20000,at:block*20});
 }
 const report=ledger.report();assert.equal(report.exact_zero_samples*600/48,150);
 assert.equal(report.unavailable_samples,1,'Only the original initial crossing lacks its actual left neighbor');
 assert.equal(60*48000-report.unavailable_samples,2879999,'Periodic continuation never recreates an unknown start600times');
 assert.equal(report.physical_audibility_proved,false);
});
test('a real160sample interior dropout remains unavailable despite strong decoded source identity',()=>{
 const samples=tone(660,.3,0);samples.fill(0,320,480);const ledger=new PcmSampleLedger();
 const row=push(ledger,samples);assert.equal(row.identity.source_identified,true);
 assert.equal(row.available_samples,800);assert.equal(ledger.report().unavailable_samples,160);
 assert.ok(ledger.report().unavailable_samples*40/48>100,'No second availability allowance');
});
test('all80+80 zero samples are debited and only exact SAME coordinates merge their decoded gap duration',()=>{
 const a=tone(660,.3,0),b=tone(660,.3,0);a.fill(0,880);b.fill(0,0,80);
 const ledger=new PcmSampleLedger();const first=push(ledger,a);assert.equal(first.available_samples,880);
 push(ledger,b,{timestamp:20000,at:20});assert.equal(first.available_samples,880);assert.equal(ledger.report().available_samples,1760);
 assert.equal(coverage(ledger).maximum_decoded_zero_run_samples,160);
 for(const options of[{owner:'foreign'},{epoch:2},{timestamp:40000},{publicInterval:1}]){
  const isolated=new PcmSampleLedger();push(isolated,a);push(isolated,b,{timestamp:20000,at:20,...options});
  assert.equal(isolated.report().available_samples,1760,'No boundary exempts a genuine zero run');
  assert.equal(coverage(isolated).maximum_decoded_zero_run_samples,80,'No joining across an unproved coordinate/public boundary');
 }
});
test('duplicate, retired clock, overlap, malformed geometry and late same-owner epoch cannot gain sample credit',()=>{
 const samples=tone(660,.3,0),ledger=new PcmSampleLedger();push(ledger,samples);
 const duplicate=push(ledger,samples,{at:20});assert.equal(duplicate.duplicate,true);assert.equal(duplicate.available_samples,0);
 assert.throws(()=>push(ledger,samples,{timestamp:1000,at:30}),/Overlapping/);
 const epochs=new PcmSampleLedger();push(epochs,samples);push(epochs,samples,{epoch:2,timestamp:20000,at:20});
 assert.throws(()=>push(epochs,samples,{timestamp:40000,at:40}),/Retired/);
});
test('original callback-start anchoring debits a leading partial support in duration and onset without changing timestamps',()=>{
 const ledger=new PcmSampleLedger();push(ledger,tone(660,.3),{timestamp:0,at:100});
 const report=coverage(ledger,{endMs:120});assert.equal(ledger.rows[0].at,100);
 assert.equal(report.first_available_ms,100+832/48);assert.equal(report.identified_seconds,128/48000);
 assert.equal(report.new_handoff_allowance_ms,0);assert.equal(report.original_callback_interval_anchoring,true);
});
test('callback bursts cannot compress a genuine original-clock zero run for gap accounting',()=>{
 const ledger=new PcmSampleLedger();for(let i=0;i<15;i++)push(ledger,new Float32Array(960),{timestamp:i*20000,at:100});
 const report=coverage(ledger,{endMs:120});assert.equal(report.maximum_decoded_zero_run_samples,15*960);
 assert.ok(report.maximum_clean_gap_ms>=300);assert.equal(ledger.report().bad_identity_rows,15,'Literalzero is still hard-unidentified, never a budget waiver');
});
test('measured handoff allowance is the original hole only, not enlarged by zero prefixes or operation timing',()=>{
 const ledger=new PcmSampleLedger();
 for(const at of[0,20,40,500,520,540,560,580])push(ledger,at===500?tone(660,.3):tone(660,.3,0),{timestamp:at*1000,at});
 const report=coverage(ledger,{endMs:600,cuts:[{id:'validated',atMs:200}]});
 assert.equal(report.transition_missing_ms,440);assert.equal(report.original_transition_coverage.transitions[0].resumed_sample_ms,500);
 assert.equal(report.unavailable_zero_samples,832);assert.equal(report.new_handoff_allowance_ms,0);
 assert.equal(report.identified_seconds,(8*960-832)/48000);
});

test('all4tones/all16phases at the quantized usable-energy floor preserve observed one-sample sign crossings',()=>{
 const quantized=(hz,phase,amplitude,offset)=>Float32Array.from({length:960},(_,i)=>Math.round(amplitude*32768*Math.sin(2*Math.PI*hz*(offset+i)/48000+phase))/32768);
 const energy=samples=>samples.reduce((n,x)=>n+x*x,0)/960;
 for(const hz of tones)for(let phase=0;phase<16;phase++){
  const angle=phase*Math.PI/8,minEnergy=amplitude=>Math.min(...Array.from({length:5},(_,block)=>energy(quantized(hz,angle,amplitude,block*960))));
  let low=.001,high=.002;for(let i=0;i<24;i++){const middle=(low+high)/2;if(minEnergy(middle)>1e-6)high=middle;else low=middle;}
  assert.ok(minEnergy(high)>1e-6);assert.ok(minEnergy(high)<1.001e-6);
  const ledger=new PcmSampleLedger(),expected=tones.indexOf(hz);
  push(ledger,quantized(hz,angle,high,-960),{expected,timestamp:0,at:0}); // Actual earlier original row, not a modeled neighbor.
  for(let block=0;block<5;block++){
   const samples=quantized(hz,angle,high,block*960),row=push(ledger,samples,{expected,timestamp:(block+1)*20000,at:(block+1)*20});
   assert.equal(row.identity.source_identified,true);assert.equal(row.identity.identified,true);
   assert.ok(row.identity.exact_zero_runs.every(run=>run.end_sample_exclusive-run.start_sample===1));
  }
  assert.equal(ledger.report(ledger.rows.slice(1)).unavailable_samples,0);
  assert.equal(ledger.report(ledger.rows.slice(1)).available_samples,5*960);
 }
});
test('127sample and repeated2sample real dropouts receive no threshold exemption',()=>{
 for(const n of[127,2]){
  const samples=tone(660,.3,0);samples.fill(0,320,320+n);const ledger=new PcmSampleLedger(),row=push(ledger,samples);
  assert.equal(row.identity.source_identified,true);assert.equal(row.available_samples,960-n);
  assert.equal(ledger.report().unavailable_samples,n);
  assert.ok(n*(n===2?3000:40)/48>100,'Repeated short losses exceed unchanged100ms budget');
 }
});
test('repeated same-sign isolated sample holes debit every missing sample rather than hiding seconds',()=>{
 const samples=tone(660,.3,0),holes=[];
 for(let i=8;i<952;i+=8)if(samples[i-1]*samples[i+1]>0){samples[i]=0;holes.push(i);}
 const ledger=new PcmSampleLedger(),row=push(ledger,samples);assert.equal(row.identity.source_identified,true);
 assert.equal(row.available_samples,960-holes.length);assert.equal(ledger.report().unavailable_samples,holes.length);
 assert.ok(holes.length*3000/48000>1,'Old blanket singlezero exemption could hide seconds');
});
test('unknown trailing1 is debited then refunded ONLY by actual same-coordinate opposite-sign next neighbor',()=>{
 const a=tone(660,.3,0);a[959]=0;assert.ok(a[958]>0);
 const b=tone(660,-Math.PI/2,0);assert.ok(b[0]<0);
 const ledger=new PcmSampleLedger(),old=push(ledger,a);assert.equal(old.available_samples,959);assert.equal(old.unavailable_runs.length,1);
 push(ledger,b,{timestamp:20000,at:20});assert.equal(old.available_samples,960);assert.deepEqual(old.unavailable_runs,[]);
 assert.equal(ledger.report().available_samples,1920,'Same old row and owner total both refund exactly1');
 for(const options of[{owner:'foreign'},{epoch:2},{timestamp:40000},{publicInterval:1},{timestamp:0}]){
  const isolated=new PcmSampleLedger(),original=push(isolated,a);push(isolated,b,{timestamp:20000,at:20,...options});
  assert.equal(original.available_samples,959);assert.equal(original.unavailable_runs.length,1);
 }
 const sameSign=new PcmSampleLedger(),original=push(sameSign,a);push(sameSign,tone(660,Math.PI/2,0),{timestamp:20000,at:20});
 assert.equal(original.available_samples,959,'Same-sign isolated hole remains missing');
});
test('leading1 crossing requires an actual original same-coordinate previous nonzero neighbor',()=>{
 const a=tone(660,.3,0);if(a[959]>0)for(let i=0;i<960;i++)a[i]=-a[i];assert.ok(a[959]<0);
 const b=tone(660,.3,0);b[0]=0;assert.ok(b[1]>0);
 const unknown=new PcmSampleLedger();assert.equal(push(unknown,b).available_samples,959,'No guessed previous sign');
 const ledger=new PcmSampleLedger();push(ledger,a);assert.equal(push(ledger,b,{timestamp:20000,at:20}).available_samples,960);
 const foreign=new PcmSampleLedger();push(foreign,a);assert.equal(push(foreign,b,{owner:'foreign',timestamp:20000,at:20}).available_samples,959);
});
