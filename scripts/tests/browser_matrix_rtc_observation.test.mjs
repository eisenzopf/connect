import {test} from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {rtcDecodedAudioWorklet,browserMatrixRtcObservation} from '../lib/browser_matrix_rtc_observation.mjs';

function processor(rate=48000){
    const messages=[];
    const context={sampleRate:rate,currentFrame:0,AudioWorkletProcessor:class{
        constructor(){this.port={postMessage:value=>messages.push(value)};}
    },registerProcessor(_name,type){context.Processor=type;}};
    vm.runInNewContext(rtcDecodedAudioWorklet,context);
    return {context,messages,owner:new context.Processor()};
}
test('RTC observer copies received PCM exactly and emits only complete original 20ms windows',()=>{
    const {context,messages,owner}=processor();
    const pcm=Float32Array.from({length:2048},(_,i)=>Math.sin(i*.031)*.17+Math.cos(i*.083)*.04);
    for(let i=0;i<pcm.length;i+=128){context.currentFrame=i;assert.equal(owner.process([[pcm.subarray(i,i+128)]]),true);}
    assert.equal(messages.length,2);
    assert.deepEqual(Array.from(messages[0].samples),Array.from(pcm.slice(0,960)));
    assert.deepEqual(Array.from(messages[1].samples),Array.from(pcm.slice(960,1920)));
    assert.equal(messages[0].timestamp,0);assert.equal(messages[1].timestamp,20000);
});
test('RTC observer never fills missing audio or combines samples across a discontinuity',()=>{
    const {context,messages,owner}=processor();
    owner.process([[new Float32Array(128).fill(1)]]);
    context.currentFrame=128;owner.process([[]]);
    context.currentFrame=256;owner.process([[new Float32Array(128).fill(2)]]);
    context.currentFrame=4096;owner.process([[new Float32Array(960).fill(3)]]);
    assert.equal(messages.length,1);assert.ok(Array.from(messages[0].samples).every(value=>value===3));
    assert.equal(messages[0].timestamp,Math.round(4096*1000000/48000));
});
test('RTC observer rejects unavailable format and stops its original processor',()=>{
    const wrong=processor(44100);assert.equal(wrong.owner.process([[new Float32Array(128)]]),false);
    assert.equal(wrong.messages[0].type,'error');
    const original=processor();original.owner.port.onmessage({data:{type:'stop'}});
    assert.equal(original.owner.process([[new Float32Array(960).fill(1)]]),false);
    assert.deepEqual(original.messages.map(value=>value.type),['stopped']);
});
test('only the actual RTC renderer is tapped and frame metadata stays bound to its original canvas',async()=>{
    const keys=['AudioContext','AudioWorkletNode','CanvasRenderingContext2D','HTMLVideoElement'];
    const saved=Object.fromEntries(keys.map(key=>[key,globalThis[key]]));
    const nodes=[],contexts=[],audio=[],video=[],errors=[];
    class AudioContext {
        state='running';destination={};audioWorklet={addModule:async()=>{}};
        constructor(){contexts.push(this);}
        createMediaStreamSource(){return {connect(){},disconnect(){}};}
        createGain(){return {gain:{value:1},connect(){},disconnect(){}};}
    }
    class AudioWorkletNode {
        port={postMessage(){},close(){this.closed=true;}};
        constructor(){nodes.push(this);}
        connect(){}disconnect(){this.disconnected=true;}
    }
    class CanvasRenderingContext2D {constructor(canvas){this.canvas=canvas;}drawImage(){}}
    class HTMLVideoElement {
        videoWidth=640;videoHeight=360;
        requestVideoFrameCallback(callback){this.callback=callback;return 1;}
    }
    class Rtc {
        input={connectionId:'original'};
        render(){const context=new AudioContext();context.createMediaStreamSource({});return Promise.resolve({});}
    }
    Object.assign(globalThis,{AudioContext,AudioWorkletNode,CanvasRenderingContext2D,HTMLVideoElement});
    const originalRender=Rtc.prototype.render,originalDraw=CanvasRenderingContext2D.prototype.drawImage;
    const observer=browserMatrixRtcObservation(Rtc,{audio:(...args)=>audio.push(args),video:(...args)=>video.push(args),
        invalid:(...args)=>errors.push(args)});
    try{
        // A preparation readiness sink is not the public renderer and must not
        // be counted as another received audio route.
        new AudioContext().createMediaStreamSource({});
        await new Rtc().render();await Promise.resolve();
        assert.equal(nodes.length,1);
        nodes[0].port.onmessage({data:{type:'pcm',samples:new Float32Array(960).fill(.125),timestamp:42}});
        assert.equal(audio.length,1);assert.equal(audio[0][0],'original');assert.equal(audio[0][1],42);
        const canvas={},other={},context=new CanvasRenderingContext2D(canvas),otherContext=new CanvasRenderingContext2D(other);
        observer.attach('original',canvas);
        const element=new HTMLVideoElement();
        element.requestVideoFrameCallback(()=>{context.drawImage(element,0,0);otherContext.drawImage(element,0,0);});
        element.callback(100,{mediaTime:1.25});
        assert.equal(video.length,1);assert.equal(video[0][0],'original');assert.equal(video[0][1],1250000);
        assert.equal(video[0][4],640);assert.equal(video[0][5],360);
        context.drawImage(element,0,0);assert.equal(video.length,1,'painting outside actual video callback is not decoded-frame evidence');
        assert.deepEqual(errors,[]);
    }finally{
        await observer.close();
        assert.equal(Rtc.prototype.render,originalRender);assert.equal(CanvasRenderingContext2D.prototype.drawImage,originalDraw);
        assert.ok(nodes.every(node=>node.disconnected&&node.port.closed));
        for(const key of keys)if(saved[key]===undefined)delete globalThis[key];else globalThis[key]=saved[key];
    }
});
