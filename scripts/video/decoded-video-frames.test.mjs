import test from "node:test";
import assert from "node:assert/strict";
import {observeDecodedVideoFrames} from "./decoded_video_frames.mjs";

test("every borrowed frame closes; only the owned clone stops and pending reads join",async()=>{
    const original=globalThis.MediaStreamTrackProcessor;
    let enqueue,configuration,stops=0,originalStops=0,closed=0;
    const clone={stop:()=>stops++},track={clone:()=>clone,stop:()=>originalStops++};
    globalThis.MediaStreamTrackProcessor=class {
        constructor(options){configuration=options;this.readable=new ReadableStream({start(c){enqueue=c;}});}
    };
    try{
        let observed=0;
        const observer=observeDecodedVideoFrames(track,()=>observed++);
        enqueue.enqueue({close:()=>closed++});enqueue.enqueue({close:()=>closed++});
        await new Promise(resolve=>setImmediate(resolve));
        assert.deepEqual(configuration,{track:clone,maxBufferSize:2});
        assert.equal(observed,2);assert.equal(closed,2);
        const result=await observer.close();assert.deepEqual(result,{frames:2,failed:false,joined:true});
        await observer.close();assert.equal(stops,1);assert.equal(originalStops,0);
    }finally{globalThis.MediaStreamTrackProcessor=original;}
});

test("a pixel-observer failure closes its frame and clone without retaining raw errors",async()=>{
    const original=globalThis.MediaStreamTrackProcessor;let stops=0,closed=0;
    globalThis.MediaStreamTrackProcessor=class {
        readable=new ReadableStream({start(c){c.enqueue({close:()=>closed++});}});
    };
    try{
        const observer=observeDecodedVideoFrames({clone:()=>({stop:()=>stops++})},()=>{throw Error("private frame detail");});
        await new Promise(resolve=>setImmediate(resolve));
        const result=await observer.close();assert.deepEqual(result,{frames:0,failed:true,joined:true});
        assert.equal(closed,1);assert.equal(stops,1);assert.equal(JSON.stringify(result).includes("private"),false);
    }finally{globalThis.MediaStreamTrackProcessor=original;}
});

test("unsupported or failed construction cannot take or leak the original track",()=>{
    const original=globalThis.MediaStreamTrackProcessor;let clones=0,stops=0;
    const track={clone:()=>{clones++;return{stop:()=>stops++};}};
    try{
        globalThis.MediaStreamTrackProcessor=undefined;
        assert.throws(()=>observeDecodedVideoFrames(track,()=>{}),TypeError);assert.equal(clones,0);
        globalThis.MediaStreamTrackProcessor=class{constructor(){throw Error("private startup detail");}};
        assert.throws(()=>observeDecodedVideoFrames(track,()=>{}));assert.equal(clones,1);assert.equal(stops,1);
    }finally{globalThis.MediaStreamTrackProcessor=original;}
});
