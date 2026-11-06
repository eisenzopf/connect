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

test('original public native outputs before completed-root stay observable while true private outputs never count',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const {nativeIdentity,nativeBoundary,bindNativeStreams}=await import('../../sdk/tests/native_activation_fixture.mjs');
    const plan=browserMatrixPlan(),old=globalThis.document;
    class Renderer {audio(){return true;}video(){return true;}}
    globalThis.document={createElement:()=>({getContext:()=>({})})};
    const connection=id=>({id,streams:{byId:id=>({id})},render:async()=>({close(){},finished:Promise.resolve()})});
    const state={connections:plan.routes.map(route=>connection(route.connection))};
    const observer=browserMatrixObservation(Renderer,plan),renderer=new Renderer(),started=performance.now()-2000;
    const frame={sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp:1,copyTo:output=>output.set(tone(1100))};
    const originalNtp=(BigInt(Math.round((performance.timeOrigin+started)*1000))*(1n<<32n))/1000000n+(2208988800n<<32n);
    const native=(seq,rtp,identity)=>({profile:'connect/native-decoded-sequence/1',identity,datagramSequences:[seq],ssrc:7,rtpTimestamp:rtp,sourceClock:{ntp:originalNtp.toString(),rtpTimestamp:0}});
    try{
        await observer.initial(state,{timeoutMs:500});observer.start({originMs:started});
        const binding={connectionId:plan.routes[0].connection,streamId:plan.routes[0].audio};
        renderer.audio(binding,frame,native(0,96000,nativeIdentity(1)));
        await observer.attach(0,connection('target'),{audio:'target-audio',video:'target-video'},{timeoutMs:500});
        renderer.audio({connectionId:'target',streamId:'target-audio'},{...frame,timestamp:2},native(3,0,nativeIdentity(2,2)));
        // Genuine public original datagram arrives while the durable root reply is delayed.
        renderer.audio({connectionId:'target',streamId:'target-audio'},{...frame,timestamp:3},native(4,96960,nativeIdentity(2,2)));
        const streams=bindNativeStreams([{strm_id:'target-audio',kind:'audio',direction:'sendrecv',stream_local_id:2},{strm_id:'target-video',kind:'video',direction:'sendonly',stream_local_id:3}],2);
        renderer.nativeActivationBoundary=nativeBoundary({recordId:'record',operationId:'move',sourceConnectionId:binding.connectionId,targetConnectionId:'target'},streams,2,3,{'target-audio':4,'target-video':4});
        observer.activate(0,'target');
        const report=observer.report();assert.equal(report.format,'connect/browser-matrix-decoded-media/2');
        assert.equal(report.routes[0].audio.decoded_seconds,.04);assert.equal(report.routes[0].audio.original_source_quantum_overlaps,0);
        // Retained physical histories never earn extra coverage for the same original source quantum.
        renderer.audio(binding,{...frame,timestamp:4},native(1,96960,nativeIdentity(1)));
        assert.equal(observer.report().routes[0].audio.original_source_quantum_overlaps,1);
    }finally{await observer.close();globalThis.document=old;}
});

test('fixture render canvases retain backing stores until their original borrower joins',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const old=globalThis.document,canvases=[];let finish,closed=0;
    class Renderer {audio(){}video(){}}
    globalThis.document={createElement:()=>{
        const canvas={getContext:()=>({})};canvases.push(canvas);return canvas;
    }};
    const originalFinished=new Promise(resolve=>{finish=resolve;});
    const connection={id:'owned-output',streams:{byId:id=>({id})},async render(input){
        input.video[0].canvas.width=640;input.video[0].canvas.height=360;
        return {close(){closed++;},finished:originalFinished};
    }};
    const observer=browserMatrixObservation(Renderer,browserMatrixPlan());
    try{
        await observer.attach(0,connection,{audio:'audio',video:'video'},{});
        const closing=observer.close();await Promise.resolve();assert.equal(closed,1);
        assert.deepEqual(canvases.map(canvas=>[canvas.width,canvas.height]),[[128,72],[640,360]]);
        finish();assert.equal((await closing).joined,true);
        assert.ok(canvases.every(canvas=>canvas.width===0&&canvas.height===0));
    }finally{finish();await observer.close();globalThis.document=old;}
});
