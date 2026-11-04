import {test} from 'node:test';
import assert from 'node:assert/strict';
import {DecodedRoute,pcmIdentity,browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
const tone=(hz,amplitude=.05,offset=0)=>Float32Array.from({length:960},(_,i)=>amplitude*Math.sin(2*Math.PI*hz*(i+offset)/48000));
test('decoded identity refuses silence and another participant rather than trusting expected frequency',()=>{
    assert.equal(pcmIdentity(tone(660),1).identified,true);
    assert.equal(pcmIdentity(tone(440),1).identified,false);
    assert.equal(pcmIdentity(new Float32Array(960),1).identified,false);
});
function observed({gap=false,duplicate=false}={}){
    const route=new DecodedRoute(0,1);route.start(1000);const pulses=markerFrames().map(frame=>frame/30);
    const high=at=>pulses.some(start=>at>=start&&at<start+.1);
    for(let i=0;i<3100;i++){
        const at=i*.02;if(gap&&at>=15&&at<15.5)continue;
        const pcm=tone(660,high(at)?.7:.05,i*960),time=1000+at*1000+10;
        route.audioFrame(i<750?'original':'target',i*20000,time,pcm);
        if(duplicate&&i===1000)route.audioFrame('target',i*20000,time+.1,pcm);
    }
    for(let i=0;i<1860;i++){
        const at=i/30,pulse=high(at),gray=Array(64).fill(pulse?255:100);
        if(!pulse){gray[0]=200;gray[1]=40;gray[8]=40;gray[9]=200;}
        route.videoFrame(i<450?'original':'target',Math.round(at*1e6),1000+at*1000+15,gray,new Uint8Array([i%256]),640,360);
    }
    return route.report();
}
test('independent content timeline uses the fixed 60-second window across original and replacement owners',()=>{
    const report=observed();assert.equal(report.passed,true);assert.equal(report.audio.identified_seconds,60);
    assert.equal(report.video.minimum_fps,30);assert.ok(report.sync.paired>=42);
});
test('missing received audio and repeated decoded timestamp cannot be hidden by extra video or frame totals',()=>{
    const gap=observed({gap:true});assert.equal(gap.passed,false);assert.ok(gap.audio.maximum_gap_ms>=490);
    assert.ok(gap.audio.identified_seconds<59.9);
    const duplicate=observed({duplicate:true});assert.equal(duplicate.passed,false);assert.equal(duplicate.audio.duplicate_frames,1);
});

test('initial duplex audio observes its own port and a prepared target stays excluded until original movement commit',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const plan=browserMatrixPlan(),old=globalThis.document;
    class Renderer {audio(){return true;}video(){return true;}}
    globalThis.document={createElement:()=>({getContext:()=>({})})};
    const connection=id=>({id,streams:{byId:id=>({id})},render:async()=>({close(){},finished:Promise.resolve()})});
    const state={connections:plan.routes.map(route=>connection(route.connection))};
    const observer=browserMatrixObservation(Renderer,plan),renderer=new Renderer();
    const frame={sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp:1,
        copyTo:output=>output.set(tone(1100))};
    try{
        await observer.initial(state,{timeoutMs:500});observer.start({originMs:performance.now()-2000});
        const binding={connectionId:plan.routes[0].connection,streamId:plan.routes[0].audio};
        renderer.audio(binding,frame);
        await observer.attach(0,connection('target'),{audio:'target-audio',video:'target-video'},{timeoutMs:500});
        renderer.audio({connectionId:'target',streamId:'target-audio'},frame);
        assert.equal(observer.report().routes[0].audio.decoded_seconds,.02);
        observer.activate(0,'target');renderer.audio(binding,{...frame,timestamp:2});
        renderer.audio({connectionId:'target',streamId:'target-audio'},{...frame,timestamp:3});
        assert.equal(observer.report().routes[0].audio.decoded_seconds,.04);
    }finally{await observer.close();globalThis.document=old;}
});
