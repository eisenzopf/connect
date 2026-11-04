import {test} from 'node:test';
import assert from 'node:assert/strict';
import {pipelineLatency,pipelineEnvelopeLatency} from '../lib/browser_matrix_pipeline.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
test('capture pipeline attributes independently decoded marker delays to equal source ordinals',()=>{
 const original=markerFrames().map(frame=>frame/30).filter(at=>at>=1&&at<60);
 const receive=original.map(at=>at+.125).filter((_,i)=>i!==17);
 const result=pipelineLatency(original,receive);assert.equal(result.status,'measured');
 assert.equal(result.paired,original.length-1);assert.ok(Math.abs(result.signed_median_ms-125)<1e-8);
 assert.ok(Math.abs(pipelineLatency(receive,original).signed_median_ms+125)<1e-8);
});
test('capture pipeline refuses a periodic or sparse edge list as timing proof',()=>{
 assert.equal(pipelineLatency([1,2,3],[1.1,2.1,3.1]).status,'missing-proof');
 assert.notEqual(pipelineLatency(Array.from({length:60},(_,i)=>i),Array.from({length:60},(_,i)=>i+.1)).status,'measured');
});

test('capture pipeline cannot qualify timing from invalid audio envelopes',()=>{
 const original=markerFrames().map(frame=>frame/30).filter(at=>at>=1&&at<60);
 const received=original.map(at=>at+.1),valid={status:'observer-valid'};
 assert.equal(pipelineEnvelopeLatency(original,received,valid,valid).status,'measured');
 for(const invalid of [undefined,{status:'observer-invalid'}]){
  const result=pipelineEnvelopeLatency(original,received,invalid,valid);
  assert.equal(result.status,'observer-invalid');assert.ok(result.paired>=42);
  assert.equal(pipelineEnvelopeLatency(original,received,valid,invalid).status,'observer-invalid');
 }
});
