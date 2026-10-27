import assert from 'node:assert/strict';
import test from 'node:test';
import {audioSampleClock} from '../video/audio_sample_clock.mjs';

test('100ms samples follow elapsed time across native timer cadences',()=>{
    for(const tick of [10,12,13,16]){
        const due=audioSampleClock(),samples=[];
        for(let observed=0;observed<60000;observed+=tick)if(due(observed))samples.push(observed);
        assert.equal(samples.length,600);
        assert.ok(samples.every((at,index)=>at>=index*100&&at<index*100+tick));
    }
});

test('late callbacks produce one real sample and expose the actual gap',()=>{
    const due=audioSampleClock();
    assert.equal(due(0),true);assert.equal(due(10),false);
    assert.equal(due(450),true);assert.equal(due(451),false);
    assert.equal(due(549),false);assert.equal(due(550),true);
});

test('invalid and backwards observation times never create samples',()=>{
    const due=audioSampleClock();
    for(const at of [-1,NaN,Infinity])assert.equal(due(at),false);
    assert.equal(due(1000),true);
    for(const at of [0,1000,1050,1099])assert.equal(due(at),false);
    assert.equal(due(1100),true);
});
