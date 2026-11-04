import assert from 'node:assert/strict';
import {test} from 'node:test';
import {constrainVideoCapture} from '../typescript/dist/browser_video_setup.js';

test('manual canvas capture retains requestFrame semantics and rejects unproven zero-rate tracks',async()=>{
    const names=['CanvasCaptureMediaStreamTrack','HTMLCanvasElement'];
    const original=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
    class Canvas {}
    class CanvasTrack {
        readyState='live';canvas=new Canvas();settings={width:640,height:360,frameRate:0};
        requestFrame(){}
        async applyConstraints(value){assert.deepEqual(value,{width:{max:640},height:{max:360},frameRate:{max:30}});}
        getSettings(){return this.settings;}
    }
    Object.defineProperty(globalThis,'CanvasCaptureMediaStreamTrack',{configurable:true,value:CanvasTrack});
    Object.defineProperty(globalThis,'HTMLCanvasElement',{configurable:true,value:Canvas});
    try {
        const track=new CanvasTrack(),stream={getVideoTracks:()=>[track]};
        await constrainVideoCapture(stream,100);
        assert.equal(track.getSettings().frameRate,0);
        for(const change of [t=>t.readyState='ended',t=>t.canvas={},t=>t.requestFrame=null,
            t=>t.settings.width=1280,t=>t.settings.height=720,t=>t.settings.frameRate=31,
            t=>t.settings.frameRate=-1,t=>t.settings.frameRate=NaN]){
            const invalid=new CanvasTrack();change(invalid);
            await assert.rejects(constrainVideoCapture({getVideoTracks:()=>[invalid]},100),{code:'unsupported-media-profile'});
        }
        await assert.rejects(constrainVideoCapture({getVideoTracks:()=>[{
            readyState:'live',canvas:new Canvas(),requestFrame(){},
            applyConstraints:async()=>{},getSettings:()=>({width:640,height:360,frameRate:0}),
        }]},100),{code:'unsupported-media-profile'});
        delete globalThis.CanvasCaptureMediaStreamTrack;
        await assert.rejects(constrainVideoCapture(stream,100),{code:'unsupported-media-profile'});
    }finally{
        for(const [name,descriptor] of original)if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];
    }
});
