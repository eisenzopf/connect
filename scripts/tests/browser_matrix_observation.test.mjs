import {test} from 'node:test';
import assert from 'node:assert/strict';
import {DecodedRoute,pcmIdentity,browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
import {transitionCoverage,cleanBinDurations} from '../lib/media_transition_coverage.mjs';

async function rtcRenderFixture(run){
    const keys=['document','AudioContext','AudioWorkletNode','CanvasRenderingContext2D','HTMLVideoElement','connectPeerTag'];
    const saved=Object.fromEntries(keys.map(key=>[key,globalThis[key]]));
    const nodes=[],renders=[],closedOwners=[],tags=[];
    let release;
    const rendering=new Promise(resolve=>{release=resolve;});
    class AudioContext {
        state='running';destination={};audioWorklet={addModule:async()=>{}};
        createMediaStreamSource(){return {connect(){},disconnect(){}};}
        createGain(){return {gain:{value:1},connect(){},disconnect(){}};}
    }
    class AudioWorkletNode {
        port={postMessage(){},close(){}};
        constructor(){nodes.push(this);}
        connect(){}disconnect(){}
    }
    class CanvasRenderingContext2D {
        constructor(canvas){this.canvas=canvas;}
        drawImage(){}
    }
    class HTMLVideoElement {requestVideoFrameCallback(){return 1;}}
    class Rtc {
        constructor(id,{wait=false,failure}={}){
            this.input={connectionId:id};this.peer={id};this.wait=wait;this.failure=failure;
        }
        capture(){return 'original-capture';}
        async render(input,options){
            renders.push({id:this.input.connectionId,options});
            if(this.failure)throw this.failure;
            new AudioContext().createMediaStreamSource({});
            if(this.wait)await rendering;
            const id=this.input.connectionId;
            let ended=false,finish,retire;
            const finished=new Promise(resolve=>{finish=resolve;});
            const retirement=new Promise(resolve=>{retire=resolve;});
            return {finished,retirement,close(){
                if(ended)return;ended=true;closedOwners.push(id);finish();
                retire(Object.freeze({connectionId:id,scope:'renderer-local',audioGraph:'closed',outputsDetached:true}));
            }};
        }
    }
    class Renderer {audio(){}video(){}}
    const canvases=[];
    Object.assign(globalThis,{AudioContext,AudioWorkletNode,CanvasRenderingContext2D,HTMLVideoElement,
        connectPeerTag:(peer,id)=>{tags.push({peer,id});return true;},
        document:{createElement(){const canvas={getContext:()=>({drawImage(){},getImageData:()=>({data:new Uint8ClampedArray(128*72*4).fill(100)})})};canvases.push(canvas);return canvas;}}});
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const observer=browserMatrixObservation(Renderer,browserMatrixPlan(),undefined,Rtc);
    const connection=(id,options)=>{
        const rtc=new Rtc(id,options);
        return {id,rtc,streams:{byId:id=>({id})},render:(input,request)=>rtc.render(input,request)};
    };
    const flush=async()=>{for(let i=0;i<8;i++)await Promise.resolve();};
    const audio=(node,timestamp)=>node.port.onmessage({data:{type:'pcm',samples:tone(1100),timestamp}});
    try{await run({observer,connection,nodes,renders,closedOwners,tags,canvases,release,flush,audio,Renderer});}
    finally{
        release();await observer.close();
        for(const key of keys)if(saved[key]===undefined)delete globalThis[key];else globalThis[key]=saved[key];
    }
}

test('RTC public rendering waits for validated activation and observes its first output before startup resolves',()=>rtcRenderFixture(async f=>{
    const old=f.connection('old'),target=f.connection('target');
    await f.observer.attach(0,old,{audio:'old-audio',video:'old-video',active:true},{timeoutMs:500});
    f.observer.start({originMs:performance.now()-2000});await f.flush();
    f.audio(f.nodes[0],0);
    await f.observer.attach(0,target,{audio:'target-audio',video:'target-video'},{timeoutMs:400});
    assert.deepEqual(f.renders.map(row=>row.id),['old'],'Private candidate must not create public playback');
    assert.equal(target.rtc.capture(),'original-capture');
    assert.ok(f.tags.some(row=>row.id==='target'&&row.peer===target.rtc.peer),'Capture diagnostics retain exact private peer identity');
    target.rtc.wait=true;
    const options={timeoutMs:123},activation=f.observer.activate(0,'target',options);
    await f.flush();
    assert.equal(f.renders.at(-1).options,options,'Use the original remaining movement budget');
    f.audio(f.nodes[1],20000);
    f.audio(f.nodes[0],40000);
    assert.equal(f.observer.report().routes[0].audio.decoded_seconds,.04,
        'First actual public target output counts while startup resolves; retired old output is excluded');
    assert.throws(()=>f.observer.activate(0,'target',options),'One renderer activation per original binding');
    f.release();await activation;
}));

test('an unactivated RTC candidate releases surfaces without claiming a renderer or starting playback',()=>rtcRenderFixture(async f=>{
    await f.observer.attach(0,f.connection('private-only'),{audio:'audio',video:'video'},{timeoutMs:500});
    const result=await f.observer.close();
    assert.deepEqual(f.renders,[]);assert.deepEqual(f.closedOwners,[]);
    assert.equal(result.owners,0);assert.equal(result.retained_surfaces,0);assert.equal(result.joined,true);
    assert.ok(f.canvases.every(canvas=>canvas.width===0&&canvas.height===0));
    assert.throws(()=>f.observer.activate(0,'private-only',{}));
}));

test('RTC observer close joins an in-flight original render activation before releasing its borrower',()=>rtcRenderFixture(async f=>{
    await f.observer.attach(0,f.connection('target',{wait:true}),{audio:'audio',video:'video'},{timeoutMs:500});
    const activation=f.observer.activate(0,'target',{timeoutMs:100});
    await f.flush();
    const closing=f.observer.close();let joined=false;closing.then(()=>{joined=true;});
    await f.flush();assert.equal(joined,false);assert.deepEqual(f.closedOwners,[]);
    f.release();await activation;
    const result=await closing;
    assert.equal(result.joined,true);assert.deepEqual(f.closedOwners,['target']);
    assert.equal(result.retained_surfaces,0);
}));

test('failed public RTC render activation remains a failure with unconfirmed borrower cleanup',()=>rtcRenderFixture(async f=>{
    const failure=Object.assign(Error('Original render failure'),{kind:'media'});
    await f.observer.attach(0,f.connection('target',{failure}),{audio:'audio',video:'video'},{timeoutMs:500});
    await assert.rejects(f.observer.activate(0,'target',{timeoutMs:100}),error=>error===failure);
    assert.ok(f.observer.report().routes[0].invalid.includes('Original public RTC render activation failed'));
    const result=await f.observer.close();
    assert.equal(result.joined,false,'A rejected render supplies no original retirement receipt');
    assert.equal(result.retained_surfaces,2);
    assert.equal(result.measurement_working_state.retired,false);
    assert.equal(result.measurement_working_state.retained.invalid_entries,1);
}));
const tone=(hz,amplitude=.05,offset=0)=>Float32Array.from({length:960},(_,i)=>amplitude*Math.sin(2*Math.PI*hz*(i+offset)/48000));
test('decoded identity refuses silence and another participant rather than trusting expected frequency',()=>{
    assert.equal(pcmIdentity(tone(660),1).identified,true);
    assert.equal(pcmIdentity(tone(440),1).identified,false);
    assert.equal(pcmIdentity(new Float32Array(960),1).identified,false);
});
function observed({gap=false,duplicate=false,interruptionMs=0,cutMs=null,videoGap=false,wrongSource=false}={}){
    const route=new DecodedRoute(0,1);route.start(1000);const pulses=markerFrames().map(frame=>frame/30);
    if(cutMs!==null){const cut={id:'original-validated-target',atMs:cutMs};route.audioCuts.push(cut);route.videoCuts.push(cut);}
    const gapSeconds=(gap?500:interruptionMs)/1000;
    const high=at=>pulses.some(start=>at>=start&&at<start+.1);
    for(let i=0;i<3100;i++){
        const at=i*.02;if(at>=15&&at<15+gapSeconds)continue;
        const pcm=tone(wrongSource&&i===1000?440:660,high(at)?.7:.05,i*960),time=1000+at*1000+10;
        route.audioFrame(i<750?'original':'target',i*20000,time,pcm);
        if(duplicate&&i===1000)route.audioFrame('target',i*20000,time+.1,pcm);
    }
    for(let i=0;i<1860;i++){
        const at=i/30,pulse=high(at),gray=Array(64).fill(pulse?255:100);
        if(videoGap&&at>=15&&at<15+gapSeconds)continue;
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

test('contract-bounded measured handoff gaps preserve raw loss and clean quality separately',()=>{
    for(const interruptionMs of [100,400]){
        const report=observed({interruptionMs,cutMs:15050,videoGap:true});
        assert.equal(report.passed,true,JSON.stringify({audio:report.audio.passed,video:report.video.passed,sync:report.sync.status,envelope:report.sync.audio_envelope.status}));
        assert.ok(report.audio.identified_seconds<60);
        if(interruptionMs===400){
            assert.equal(report.audio.transition_coverage.transitions.length,1);
            assert.ok(report.audio.maximum_gap_ms>200);
            assert.ok(report.video.minimum_fps<24);
            assert.equal(report.sync.audio_envelope.onset_state_crossed_transition,false);
        }
    }
});

test('handoff accounting refuses over-one-second gaps and equal gaps away from original cuts',()=>{
    for(const options of [{interruptionMs:1100,cutMs:15050},{interruptionMs:400},
        {interruptionMs:400,cutMs:16000}]){
        const report=observed({...options,videoGap:true});
        assert.equal(report.passed,false);
        assert.equal(report.audio.transition_coverage.transitions.length,0);
    }
});

test('handoff coverage grants no exception for wrong source or duplicate original outputs',()=>{
    for(const failure of [{wrongSource:true},{duplicate:true}]){
        const report=observed({interruptionMs:400,cutMs:15050,videoGap:true,...failure});
        assert.equal(report.passed,false);
        assert.equal(report.audio.transition_coverage.transitions.length,1);
    }
});

test('transition accounting requires bounded ordered actual cuts and excludes ambiguous or edge gaps',()=>{
    const options={startMs:0,endMs:2000,frameMs:20,minimumGapMs:200};
    for(const cuts of [[{id:'a',atMs:800},{id:'a',atMs:900}],
        [{id:'a',atMs:900},{id:'b',atMs:800}],[{id:'a',atMs:NaN}]])
        assert.throws(()=>transitionCoverage([20,40,440],cuts,options));
    const ambiguous=transitionCoverage([20,40,440],[{id:'a',atMs:100},{id:'b',atMs:200}],options);
    assert.equal(ambiguous.transitions.length,0);
    const edge=transitionCoverage([400,420],[{id:'a',atMs:200}],options);
    assert.equal(edge.transitions.length,0);
    const known=transitionCoverage([20,40,440],[{id:'a',atMs:200}],options);
    assert.equal(known.transition_missing_ms,380);
    assert.deepEqual(cleanBinDurations(known,0,2),[620,1000]);
});

test('original public native outputs before completed-root stay observable while true private outputs never count',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const {nativeIdentity,nativeBoundary,bindNativeStreams}=await import('../../sdk/tests/native_activation_fixture.mjs');
    const plan=browserMatrixPlan(),old=globalThis.document;
    class Renderer {audio(){return true;}video(){return true;}}
    globalThis.document={createElement:()=>({getContext:()=>({})})};
    const connection=id=>({id,streams:{byId:id=>({id})},render:async()=>({close(){},finished:Promise.resolve(),
        retirement:Promise.resolve(Object.freeze({connectionId:id,scope:'renderer-local',audioGraph:'not-created',outputsDetached:true}))})});
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
        return {close(){closed++;},finished:originalFinished,retirement:originalFinished.then(()=>
            Object.freeze({connectionId:'owned-output',scope:'renderer-local',audioGraph:'closed',outputsDetached:true}))};
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


test('real original graph close refusal retains that borrower while other proven owners release backing',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const previous=globalThis.document,canvases=[],closed=[];
    class Renderer {audio(){}video(){}}
    globalThis.document={createElement:()=>{
        const canvas={getContext:()=>({})};canvases.push(canvas);return canvas;
    }};
    const originalFailure=Object.assign(Error('Private detail must not be exported'),
        {name:'OriginalRendererFailure',code:'closed',kind:'transport-uncertain',certainty:'read-only'});
    const connection=(id,fail)=>({id,streams:{byId:id=>({id})},async render(input){
        input.video[0].canvas.width=640;input.video[0].canvas.height=360;
        let finish,reject,retire,refuseRetirement;const finished=new Promise((resolve,refuse)=>{finish=resolve;reject=refuse;});
        const retirement=new Promise((resolve,refuse)=>{retire=resolve;refuseRetirement=refuse;});
        return {close(){closed.push(id);if(fail){reject(originalFailure);
            refuseRetirement(Object.assign(Error('Private graph close detail'),{name:'OriginalGraphCloseFailure',code:'graph-close-refused'}));}
            else{finish();retire(Object.freeze({connectionId:id,scope:'renderer-local',audioGraph:'closed',outputsDetached:true}));}},finished,retirement};
    }});
    const observer=browserMatrixObservation(Renderer,browserMatrixPlan());
    try{
        await observer.attach(0,connection('failed-original',true),{audio:'audio',video:'video'},{});
        await observer.attach(4,connection('joined-original',false),{audio:'audio',video:'video'},{});
        const closing=observer.close(),result=await closing;
        assert.equal(observer.close(),closing,'One original close owns the same join');
        assert.deepEqual(closed,['failed-original','joined-original'],'A rejection cannot skip another original renderer');
        assert.equal(result.joined,false);assert.equal(result.playback_passed,false);assert.equal(result.owners,2);
        assert.deepEqual(result.owner_outcomes,[
            {owner_index:0,route_index:0,connection_id:'failed-original',status:'rejected',stage:'original-render-finished',
                name:'OriginalRendererFailure',code:'closed',kind:'transport-uncertain',certainty:'read-only'},
            {owner_index:1,route_index:4,connection_id:'joined-original',status:'fulfilled',stage:'original-render-finished'}]);
        assert.equal(JSON.stringify(result).includes(originalFailure.message),false);
        assert.equal(result.retirement_outcomes[0].name,'OriginalGraphCloseFailure');
        assert.equal(result.retirement_outcomes[0].code,'graph-close-refused');
        assert.equal(result.retirement_outcomes[0].stage,'original-render-retirement');
        assert.equal(result.retirement_outcomes[1].status,'fulfilled');
        assert.equal(result.retained_surfaces,2);
        assert.deepEqual(canvases.map(canvas=>[canvas.width,canvas.height]),[[128,72],[640,360],[0,0],[0,0]],
            'A failed original graph close cannot release its backing; other proven borrowers still release');
    }finally{await observer.close();globalThis.document=previous;}
});


test('semantic playback failure remains failed while the exact delayed original retirement proves local cleanup',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const previous=globalThis.document,canvases=[];let retire,rejectFinished;
    class Renderer {audio(){}video(){}}
    globalThis.document={createElement:()=>{const canvas={getContext:()=>({})};canvases.push(canvas);return canvas;}};
    const failure=Object.assign(Error('Original playback refusal'),{name:'PlaybackFailure',kind:'transport-uncertain',certainty:'read-only'});
    const originalRetirement=new Promise(resolve=>{retire=resolve;});
    const handle={finished:new Promise((_,reject)=>{rejectFinished=reject;}),retirement:originalRetirement,
        close(){rejectFinished(failure);}};
    const receipt=Object.freeze({connectionId:'original',scope:'renderer-local',audioGraph:'closed',outputsDetached:true});
    const connection={id:'original',streams:{byId:id=>({id})},async render(input){
        input.video[0].canvas.width=640;input.video[0].canvas.height=360;return handle;
    }};
    const observer=browserMatrixObservation(Renderer,browserMatrixPlan());
    try{
        await observer.attach(0,connection,{audio:'audio',video:'video'},{});
        // An identical-looking replacement promise cannot substitute for the
        // exact original promise the observer retained from its render handle.
        handle.retirement=Promise.resolve(receipt);
        const closing=observer.close();let settled=false;closing.then(()=>{settled=true;});
        for(let i=0;i<8;i++)await Promise.resolve();
        assert.equal(settled,false,'Pending original cleanup cannot become a fake join');
        assert.deepEqual(canvases.map(canvas=>[canvas.width,canvas.height]),[[128,72],[640,360]]);
        retire(receipt);const result=await closing;
        assert.equal(result.joined,true,'Actual graph/output cleanup is independent of semantic playback failure');
        assert.equal(result.playback_passed,false,'The original finished rejection cannot become a playback or cell pass');
        assert.equal(result.owner_outcomes[0].name,'PlaybackFailure');
        assert.equal(result.owner_outcomes[0].kind,'transport-uncertain');
        assert.equal(result.owner_outcomes[0].certainty,'read-only');
        assert.equal(result.owner_outcomes[0].status,'rejected');
        assert.equal(result.retirement_outcomes[0].status,'fulfilled');
        assert.equal(result.retained_surfaces,0);
        assert.ok(canvases.every(canvas=>canvas.width===0&&canvas.height===0));
    }finally{retire(receipt);await observer.close();globalThis.document=previous;}
});

test('foreign, mutable and missing retirement receipts cannot prove the original borrower cleanup',async()=>{
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    for(const variant of ['foreign','mutable','missing']){
        const previous=globalThis.document,canvases=[];
        class Renderer {audio(){}video(){}}
        globalThis.document={createElement:()=>{const canvas={getContext:()=>({})};canvases.push(canvas);return canvas;}};
        const receipt={connectionId:variant==='foreign'?'another-owner':'original',scope:'renderer-local',audioGraph:'closed',outputsDetached:true};
        const connection={id:'original',streams:{byId:id=>({id})},async render(input){
            input.video[0].canvas.width=640;input.video[0].canvas.height=360;
            return {close(){},finished:Promise.resolve(),
                ...(variant==='missing'?{}:{retirement:Promise.resolve(variant==='mutable'?receipt:Object.freeze(receipt))})};
        }};
        const observer=browserMatrixObservation(Renderer,browserMatrixPlan());
        try{
            await observer.attach(0,connection,{audio:'audio',video:'video'},{});
            const result=await observer.close();
            assert.equal(result.playback_passed,true,'A fulfilled playback promise is not local cleanup evidence');
            assert.equal(result.joined,false,variant);assert.equal(result.retirement_outcomes[0].status,'rejected',variant);
            assert.equal(result.retained_surfaces,2,variant);
            assert.deepEqual(canvases.map(canvas=>[canvas.width,canvas.height]),[[128,72],[640,360]],variant);
        }finally{await observer.close();globalThis.document=previous;}
    }
});

test('joined observer retirement releases measurement working buffers without changing captured failure evidence',async()=>{
    for(const extraCallbacks of [false,true])await rtcRenderFixture(async f=>{
        await f.observer.attach(0,f.connection('old'),{audio:'audio',video:'video',active:true},{timeoutMs:500});
        f.observer.start({originMs:performance.now()-2000});await f.flush();
        f.audio(f.nodes[0],0);
        f.nodes[0].port.onmessage({data:{type:'pcm',samples:new Float32Array(960),timestamp:20000}});
        const renderer=new f.Renderer();
        renderer.video({connectionId:'old',streamId:'video'},
            {timestamp:30000,displayWidth:640,displayHeight:360});
        renderer.audio({connectionId:'old',streamId:'audio'},
            {sampleRate:44100,numberOfChannels:1,numberOfFrames:960,timestamp:40000});
        const report=f.observer.report(),captured=structuredClone(report);
        assert.equal(report.passed,false);assert.ok(report.routes[0].invalid.length);
        assert.equal(report.routes[0].audio_boundary_diagnostics.events.length,1);
        if(extraCallbacks){
            f.audio(f.nodes[0],60000);
            renderer.video({connectionId:'old',streamId:'video'},
                {timestamp:70000,displayWidth:640,displayHeight:360});
            renderer.audio({connectionId:'old',streamId:'audio'},
                {sampleRate:44100,numberOfChannels:1,numberOfFrames:960,timestamp:80000});
        }
        // Deliberately do not call report() again before close. A cached live
        // snapshot must not hide callbacks after that original intermediate proof.
        const closed=await f.observer.close(),working=closed.measurement_working_state;
        assert.equal(closed.joined,true);assert.ok(working,'Original joined observer must report actual working-state retirement');
        assert.equal(working.retired,true);
        assert.equal(working.before.audio_rows,extraCallbacks?3:2);assert.equal(working.before.video_rows,extraCallbacks?2:1);
        assert.equal(working.before.duplicate_keys,extraCallbacks?5:3);assert.ok(working.before.last_pixel_bytes>0);
        assert.ok(Object.values(working.retained).every(value=>value===0));
        assert.deepEqual(report,captured,'Later callbacks and retirement preserve the intermediate failure evidence');
        const final=f.observer.report();
        assert.equal(final.routes[0].audio.decoded_seconds,extraCallbacks?.06:.04);
        assert.equal(final.routes[0].video.decoded_frames,extraCallbacks?2:1);
        assert.equal(final.routes[0].invalid.length,extraCallbacks?2:1);
        if(!extraCallbacks)assert.deepEqual(final,captured,'Final captured quality proof is preserved when no extra outputs arrive');
        assert.deepEqual(f.closedOwners,['old']);
    });
});
