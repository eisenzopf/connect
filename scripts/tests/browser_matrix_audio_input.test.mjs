import {test} from 'node:test';
import assert from 'node:assert/strict';
import {MatrixAudioInput} from '../lib/browser_matrix_audio_input.mjs';

function low(input,owner='original',base=0,arrival=0){
    for(let i=0;i<=10;i++)input.sample(owner,base+i*20000,arrival+i*.02,.035);
}
test('queued distinct original PCM can share an arrival time without retiming its marker',()=>{
    const input=new MatrixAudioInput();low(input);
    input.sample('original',220000,.24,.035);
    input.sample('original',240000,.24,.4);
    assert.deepEqual(input.edges,[.24]);
    assert.equal(input.report().status,'observer-valid');
    assert.equal(input.report().equal_resolution_arrivals,1);
    assert.equal(input.report().owners[0].frames,13);
});
test('duplicate PCM and backwards arrival remain invalid even with a plausible marker',()=>{
    for(const [timestamp,arrival] of [[200000,.3],[220000,.19],[210000,.3]]){
        const input=new MatrixAudioInput();low(input);input.sample('original',timestamp,arrival,.4);
        assert.equal(input.report().status,'observer-invalid');assert.deepEqual(input.edges,[]);
    }
});
test('replacement cannot borrow old owner rearm or create a marker from its first high frame',()=>{
    const input=new MatrixAudioInput();low(input);input.sample('replacement',900000,.3,.4);
    assert.deepEqual(input.edges,[]);low(input,'replacement',920000,.32);
    input.sample('replacement',1140000,.54,.4);
    assert.deepEqual(input.edges,[.54]);assert.equal(input.report().owners.length,2);
});
test('missing source intervals and oversized physical-owner sets remain visible',()=>{
    const input=new MatrixAudioInput();low(input);input.sample('original',220000,.22,.4);
    input.sample('original',300000,.3,.4);
    assert.equal(input.report().status,'observer-invalid');
    assert.equal(input.report().owners[0].missing_capture_us,60000);
    for(const owner of ['second','third','fourth'])input.sample(owner,0,.4,.035);
    assert.equal(input.report().owners.length,3);assert.ok(input.report().invalid.length);
});
