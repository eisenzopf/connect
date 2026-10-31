import assert from 'node:assert/strict';
import test from 'node:test';
import {renderedMarkerClock} from './marker_calibration.mjs';

test('late render callbacks retain the signed difference from both presentation clocks',()=>{
    assert.deepEqual(renderedMarkerClock(1000,{expectedDisplayTime:940,presentationTime:930,presentedFrames:30}),
        {callback_seconds:1,callback_minus_expected_display_ms:60,callback_minus_presentation_ms:70,presented_frames:30});
});
test('a callback before expected display retains its negative difference',()=>{
    assert.equal(renderedMarkerClock(1000,{expectedDisplayTime:1016}).callback_minus_expected_display_ms,-16);
});
test('missing or invalid clocks remain unmeasured and unrelated metadata cannot escape',()=>{
    const result=renderedMarkerClock(1000,{expectedDisplayTime:NaN,presentationTime:-1,presentedFrames:Infinity,
        privateTrack:'private',transportAddress:'private'});
    assert.deepEqual(result,{callback_seconds:1,callback_minus_expected_display_ms:null,
        callback_minus_presentation_ms:null,presented_frames:null});
    assert.equal(renderedMarkerClock(1000,{expectedDisplayTime:3600001}).callback_minus_expected_display_ms,null);
    assert.equal(renderedMarkerClock(1000,{}).callback_minus_presentation_ms,null);
});
test('invalid callback clocks cannot manufacture a render observation',()=>{
    for(const value of [-1,NaN,Infinity,3600001])assert.throws(()=>renderedMarkerClock(value),TypeError);
});
