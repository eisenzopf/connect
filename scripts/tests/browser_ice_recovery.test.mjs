import assert from 'node:assert/strict';
import test from 'node:test';
import {observeFreshIceMedia} from '../video/browser_ice_restart.mjs';

async function receiver({tone=660,rms=.03,video=true,fresh=true,late=false}) {
    const saved=new Map(['window','performance','setTimeout'].map(key=>[key,Object.getOwnPropertyDescriptor(globalThis,key)]));
    let clock=0;
    const observation={audioSamples:[-300,-200,-100].map(at=>({unix_seconds:at/1000,rms:.03,frequency_hz:660})),changingTimes:[]};
    Object.defineProperty(globalThis,'window',{configurable:true,value:{videoFixture:{observation}}});
    Object.defineProperty(globalThis,'performance',{configurable:true,value:{timeOrigin:0,now:()=>clock}});
    Object.defineProperty(globalThis,'setTimeout',{configurable:true,value:callback=>{
        clock+=25;
        if(clock%100===0&&(!late||clock>=2000)) {
            if(fresh)observation.audioSamples.push({unix_seconds:clock/1000,rms,frequency_hz:tone});
            if(video)observation.changingTimes.push(clock/1000);
        }
        callback();
    }});
    try{return await observeFreshIceMedia({expectedHz:660});}
    finally{for(const [key,descriptor] of saved)descriptor?Object.defineProperty(globalThis,key,descriptor):delete globalThis[key];}
}

test('fresh decoded tone and changing video prove recovery inside the original two-second bound',async()=>{
    const result=await receiver({});
    assert.equal(result.status,'passed');assert.equal(result.fresh_audio_samples,3);
    assert.equal(result.seconds,.3);assert.equal(result.limit_seconds,2);
    assert.ok(result.fresh_changing_frames>=1);assert.ok(result.maximum_audio_gap_seconds<=.2);
});
for(const [name,input] of [
    ['stale pre-restart audio',{fresh:false}],
    ['a different decoded source tone',{tone:440}],
    ['near-silent decoded audio',{rms:.0009}],
    ['unchanging decoded video',{video:false}],
    ['media arriving at or after the recovery deadline',{late:true}],
])test(`${name} cannot qualify ICE recovery`,async()=>{
    await assert.rejects(receiver(input),/within two seconds/);
});
