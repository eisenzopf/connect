import {test} from 'node:test';
import assert from 'node:assert/strict';
import {DecodedRoute,pcmIdentity,browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
import {transitionCoverage,cleanBinDurations} from '../lib/media_transition_coverage.mjs';

async function rtcRenderFixture(run){
    const keys=['document','AudioContext','MediaStreamAudioSourceNode','AudioWorkletNode','CanvasRenderingContext2D','HTMLVideoElement','connectPeerTag'];
    const saved=Object.fromEntries(keys.map(key=>[key,globalThis[key]]));
    const nodes=[],renders=[],closedOwners=[],tags=[];
    let release;
    const rendering=new Promise(resolve=>{release=resolve;});
    class MediaStreamAudioSourceNode {
        constructor(context){this.context=context;}
        connect(){}
        disconnect(){}
    }
    class AudioContext {
        state='running';destination={};audioWorklet={addModule:async()=>{}};
        createMediaStreamSource(){return new MediaStreamAudioSourceNode(this);}
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
            const context=new AudioContext();context.createMediaStreamSource({}).connect(context.destination);
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
    Object.assign(globalThis,{AudioContext,MediaStreamAudioSourceNode,AudioWorkletNode,CanvasRenderingContext2D,HTMLVideoElement,
        connectPeerTag:(peer,id)=>{tags.push({peer,id});return true;},
        document:{createElement(){const canvas={getContext:()=>({drawImage(){},getImageData:()=>({data:new Uint8ClampedArray(128*72*4).fill(100)})})};canvases.push(canvas);return canvas;}}});
    const {browserMatrixPlan}=await import('../lib/browser_matrix_admission.mjs');
    const plan=browserMatrixPlan();
    const observer=browserMatrixObservation(Renderer,plan,undefined,Rtc);
    const connection=(id,options)=>{
        const rtc=new Rtc(id,options);
        return {id,rtc,streams:{byId:id=>({id})},render:(input,request)=>rtc.render(input,request)};
    };
    const flush=async()=>{for(let i=0;i<8;i++)await Promise.resolve();};
    const audio=(node,timestamp)=>node.port.onmessage({data:{type:'pcm',samples:tone(1100),timestamp}});
    try{await run({observer,plan,connection,nodes,renders,closedOwners,tags,canvases,release,flush,audio,Renderer});}
    finally{
        release();await observer.close();
        for(const key of keys)if(saved[key]===undefined)delete globalThis[key];else globalThis[key]=saved[key];
    }
}

test('initial RTC render promises start before later offers and the programme barrier joins each same owner',()=>rtcRenderFixture(async f=>{
    const state={connections:f.plan.routes.map(route=>f.connection(route.connection,
        route.index===0?{wait:true}:undefined))};
    const options={timeoutMs:8000};
    f.observer.startInitialConnection(0,state.connections[0],options);
    await f.flush();
    assert.deepEqual(f.renders.map(row=>row.id),[state.connections[0].id]);
    assert.equal(f.renders[0].options,options,'Keep the original initial render budget');
    f.audio(f.nodes[0],0);
    assert.equal(f.observer.report().routes[0].audio.decoded_seconds,0,
        'Actual startup before programme origin grants no measured frames');
    for(const route of f.plan.routes.slice(1))
        f.observer.startInitialConnection(route.index,state.connections[route.index],options);
    await f.flush();
    assert.equal(f.renders.length,6,'A pending first play must not prevent later original receivers from starting');
    let joined=false;
    const barrier=f.observer.initial(state,options).then(()=>{joined=true;});
    await f.flush();
    assert.equal(joined,false,'The original all-six barrier still awaits pending play');
    f.release();await barrier;
    assert.deepEqual(f.renders.map(row=>row.id),state.connections.map(value=>value.id));
    assert.equal(f.nodes.length,6,'No second decoder graph at the final barrier');
    const closed=await f.observer.close();
    assert.equal(closed.joined,true);
    assert.equal(new Set(f.closedOwners).size,6,'Close every exact original renderer once');
}));

test('initial RTC render refusal retains earlier owners for cleanup and cannot be retried by the final barrier',()=>rtcRenderFixture(async f=>{
    const failure=Object.assign(Error('Original initial render refused'),{kind:'media'});
    const state={connections:f.plan.routes.map(route=>f.connection(route.connection,
        route.index===0?{wait:true}:route.index===1?{failure}:undefined))};
    f.observer.startInitialConnection(0,state.connections[0],{timeoutMs:8000});
    f.observer.startInitialConnection(1,state.connections[1],{timeoutMs:8000});
    await f.flush();
    assert.throws(()=>f.observer.startInitialConnection(2,state.connections[2],{timeoutMs:8000}),
        error=>error===failure,'An observed original refusal prevents later decoder startup');
    const barrier=f.observer.initial(state,{timeoutMs:8000});
    void barrier.catch(()=>{});
    assert.deepEqual(f.renders.map(row=>row.id),state.connections.slice(0,2).map(value=>value.id));
    let joined=false;
    const closing=f.observer.close().then(value=>{joined=true;return value;});
    await f.flush();
    assert.equal(joined,false,'Failure cleanup must join an earlier pending original render');
    f.release();
    await assert.rejects(barrier,error=>error===failure);
    const closed=await closing;
    assert.deepEqual(f.closedOwners,[state.connections[0].id],
        'Later refusal cannot skip an earlier actual render owner');
    assert.equal(closed.joined,false,'No retirement proof may be inferred for refused startup');
    assert.ok(closed.retained_surfaces>0);
}));

test('RTC public rendering waits for validated activation and observes its first output before startup resolves',()=>rtcRenderFixture(async f=>{
    const old=f.connection('old'),target=f.connection('target');
    await f.observer.attach(0,old,{audio:'old-audio',video:'old-video',active:true},{timeoutMs:500});
    f.observer.start({originMs:performance.now()-2000});await f.flush();
    f.audio(f.nodes[0],0);
    assert.equal(f.observer.report().quality_profile, 'connect/decoded-audio-quality/3');
    assert.equal(f.observer.report().metric_scope, 'measurable-decoded-quality-source-quanta-diagnostic');
    assert.equal(f.observer.report().original_source_sample_coverage_proved, false);
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
function observed({gap=false,duplicate=false,videoDuplicate=false,interruptionMs=0,cutMs=null,videoGap=false,wrongSource=false,rtcPresentation=false,audioArrivalStepMs=0,audioDelayMs=0,quantumAttribution=null}={}){
    const route=new DecodedRoute(0,1);route.start(1000);const pulses=markerFrames().map(frame=>frame/30);
    if(cutMs!==null){const cut={id:'original-validated-target',atMs:cutMs};route.audioCuts.push(cut);route.videoCuts.push(cut);}
    const gapSeconds=(gap?500:interruptionMs)/1000;
    const high=at=>pulses.some(start=>at>=start&&at<start+.1);
    for(let i=0;i<3100;i++){
        const at=i*.02;if(at>=15&&at<15+gapSeconds)continue;
        const pcm=tone(wrongSource&&i===1000?440:660,high(at)?.7:.05,i*960),time=1000+at*1000+10;
        const arrival = audioArrivalStepMs ? 1000 + Math.floor((time - 1000) / audioArrivalStepMs) * audioArrivalStepMs : time;
        let native = null;
        if (quantumAttribution) {
            const shifted = i - (quantumAttribution === 'overlap' && i >= 750 ? 1 : 0);
            const reportSecond = Math.floor(shifted / 50);
            const originUs = BigInt(Math.round((performance.timeOrigin + 1000) * 1000));
            const originNtp = ((originUs / 1000000n + 2208988800n) << 32n)
                + ((originUs % 1000000n) << 32n) / 1000000n;
            native = {profile:'connect/native-decoded-sequence/1', ssrc:7, rtpTimestamp:shifted*960,
                identity:{profile:'native-sequence-cursor/1', physical_incarnation:(i<750?'1':'2').padStart(32,'0'),
                    connection_generation:i<750?1:2, stream_generation:1}};
            if (quantumAttribution !== 'unbound') {
                const phase = quantumAttribution === 'ambiguous' && reportSecond % 2 ? (1n << 32n)/50n : 0n;
                native.sourceClock = {ntp:(originNtp + (BigInt(reportSecond) << 32n) + phase).toString(),
                    rtpTimestamp:reportSecond*48000};
            }
        }
        route.audioFrame(i<750?'original':'target',i*20000,arrival+audioDelayMs,pcm,null,native);
        if(duplicate&&i===1000)route.audioFrame('target',i*20000,time+.1,pcm,null,native);
    }
    for(let i=0;i<1860;i++){
        const at=i/30,pulse=high(at),gray=Array(64).fill(pulse?255:100);
        if(videoGap&&at>=15&&at<15+gapSeconds)continue;
        if(!pulse){gray[0]=200;gray[1]=40;gray[8]=40;gray[9]=200;}
        const presentation = rtcPresentation ? {profile:'connect/rtc-video-presentation/1',
            element_id:i<450?1:2, presentedFrames:i<450?i+1:i-449, mediaTime:0, rtpTimestamp:i*3000} : null;
        route.videoFrame(i<450?'original':'target',rtcPresentation?0:Math.round(at*1e6),
            1000+at*1000+15,gray,new Uint8Array([i%256]),640,360,null,presentation);
        if(videoDuplicate&&i===1000)route.videoFrame('target',Math.round(at*1e6),
            1000+at*1000+15.1,gray,new Uint8Array([i%256]),640,360);
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
    const frame={sampleRate:48000,numberOfChannels:1,numberOfFrames:960,timestamp:0,copyTo:output=>output.set(tone(1100))};
    const originalNtp=(BigInt(Math.round((performance.timeOrigin+started)*1000))*(1n<<32n))/1000000n+(2208988800n<<32n);
    const native=(seq,rtp,identity)=>({profile:'connect/native-decoded-sequence/1',identity,datagramSequences:[seq],ssrc:7,rtpTimestamp:rtp,sourceClock:{ntp:originalNtp.toString(),rtpTimestamp:0}});
    try{
        await observer.initial(state,{timeoutMs:500});observer.start({originMs:started});
        const binding={connectionId:plan.routes[0].connection,streamId:plan.routes[0].audio};
        renderer.audio(binding,frame,native(0,96000,nativeIdentity(1)));
        await observer.attach(0,connection('target'),{audio:'target-audio',video:'target-video'},{timeoutMs:500});
        renderer.audio({connectionId:'target',streamId:'target-audio'},{...frame,timestamp:0},native(3,0,nativeIdentity(2,2)));
        // Genuine public original datagram arrives while the durable root reply is delayed.
        renderer.audio({connectionId:'target',streamId:'target-audio'},{...frame,timestamp:20000},native(4,96960,nativeIdentity(2,2)));
        const streams=bindNativeStreams([{strm_id:'target-audio',kind:'audio',direction:'sendrecv',stream_local_id:2},{strm_id:'target-video',kind:'video',direction:'sendonly',stream_local_id:3}],2);
        renderer.nativeActivationBoundary=nativeBoundary({recordId:'record',operationId:'move',sourceConnectionId:binding.connectionId,targetConnectionId:'target'},streams,2,3,{'target-audio':4,'target-video':4});
        observer.activate(0,'target');
        const report=observer.report();assert.equal(report.format,'connect/browser-matrix-decoded-media/2');
        assert.equal(report.routes[0].audio.decoded_seconds,.04);assert.equal(report.routes[0].audio.original_source_quantum_overlaps,0);
        // Retained physical histories never earn extra coverage for the same original source quantum.
        renderer.audio(binding,{...frame,timestamp:20000},native(1,96960,nativeIdentity(1)));
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


test('RTC route credits actual presented frames despite constant media time and keeps strict identity bounds', () => {
    const report = observed({rtcPresentation:true});
    assert.equal(report.passed, true);
    assert.equal(report.video.minimum_fps, 30);
    assert.equal(report.video.duplicate_frames, 0);
    assert.equal(report.video.original_source_timestamp_overlaps, 0, 'RTC presentation metadata is not native source-clock provenance');
    assert.equal(report.video.presentation_identity_diagnostics.samples.length, 16);
    assert.equal(report.video.presentation_identity_diagnostics.samples[0].mediaTime, 0);
    const route = new DecodedRoute(0, 1);
    route.start(0);
    const gray = Array(64).fill(100);
    const frame = (owner, element_id, presentedFrames) => route.videoFrame(owner, 0, 2000,
        gray, new Uint8Array([presentedFrames || 0]), 640, 360, null,
        {profile:'connect/rtc-video-presentation/1', element_id, presentedFrames, mediaTime:0});
    frame('original', 1, 2);
    for (const value of [2, 1, 0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, undefined]) {
        assert.throws(() => frame('original', 1, value), /presentation/);
    }
    assert.throws(() => frame('another-owner', 1, 3), /presentation/);
    frame('original', 2, 1);
    frame('original', 2, 4);
    assert.equal(route.video.length, 3, 'New element identity is distinct, counter gaps never manufacture rows');
    assert.equal(route.video.every(row => row.native === null), true);
    assert.equal(route.report().video.presentation_identity_diagnostics.refusals.length, 10);
    const invalid = route.report();
    route.retireWorkingState();
    assert.equal(route.workingState().presentation_owners, 0);
    assert.equal(invalid.video.presentation_identity_diagnostics.refusals.length, 10, 'Retirement leaves captured raw diagnostics unchanged');
});


test('decoded PCM chronology accepts tied callback deliveries while retaining their original onset arrival rows', () => {
    const report = observed({audioArrivalStepMs:100});
    const envelope = report.sync.audio_envelope;
    assert.equal(envelope.status, 'observer-valid');
    assert.equal(envelope.profile, 'connect/decoded-pcm-chronology-envelope/1');
    assert.equal(envelope.original_unsegmented_callback_observer.status, 'observer-invalid');
    assert.ok(envelope.equal_resolution_arrivals > 0);
    assert.equal(envelope.maximum_sample_gap_seconds, .02);
    assert.equal(report.audio.maximum_pcm_sample_gap_ms, 20);
    assert.ok(envelope.accepted_onset_rows.length > 42);
    assert.deepEqual(envelope.accepted_edge_seconds,
        envelope.accepted_onset_rows.filter(row => row.callback_seconds >= 1).map(row => row.callback_seconds));
    assert.equal(report.audio.passed, true, 'Original callback coverage/count gates remain satisfied');
});

test('decoded PCM chronology preserves real frame gaps and rejects malformed overlapping or backward timestamps', () => {
    const route = new DecodedRoute(0, 1);
    route.start(0);
    for (let i = 0; i < 25; i++) {
        route.audioFrame('original', i * 20000, 1000 + i * 20, tone(660, i >= 15 ? .7 : .05));
    }
    route.audioFrame('original', 1000000, 1500, tone(660, .7));
    const report = route.report();
    assert.equal(report.sync.audio_envelope.status, 'observer-invalid');
    assert.ok(report.sync.audio_envelope.invalid.some(row => row.reason.includes('unobserved interval')));
    assert.equal(report.audio.maximum_pcm_sample_gap_ms, 520);
    assert.equal(report.audio.passed, false);
    route.audioFrame('original', 1000000, 1520, tone(660));
    assert.equal(route.report().audio.duplicate_frames, 1, 'Actual repeated PCM remains recorded and failed');
    for (const timestamp of [999999, 1000001, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1, undefined]) {
        assert.throws(() => route.audioFrame('original', timestamp, 1520, tone(660)), /PCM/);
    }
});

test('decoded onset validity never replaces delayed callback A/V skew with a source clock offset', () => {
    const report = observed({audioDelayMs:180});
    const envelope = report.sync.audio_envelope;
    assert.equal(envelope.status, 'observer-valid');
    assert.ok(envelope.accepted_onset_rows.length > 42);
    assert.equal(report.sync.status, 'failed');
    assert.ok(report.sync.median_abs_skew_ms > 150);
    assert.deepEqual(report.sync.audio_edge_seconds,
        envelope.accepted_onset_rows.filter(row => row.callback_seconds >= 1 && row.callback_seconds < 61).map(row => row.callback_seconds));
    assert.equal(report.passed, false);
});

test('decoded PCM epochs require actual declared identity and never rearm through stale epochs or a reset timestamp', async () => {
    const route = new DecodedRoute(0, 1);
    route.start(0);
    const clock = epoch_id => ({profile:'connect/rtc-decoded-pcm-clock/1', owner:'original', epoch_id});
    for (let i = 0; i < 20; i++) {
        route.audioFrame('original', i * 20000, 1000 + i * 20, tone(660, i >= 15 ? .7 : .05), null, null, clock(1));
    }
    assert.throws(() => route.audioFrame('original', 0, 1400, tone(660), null, null, clock(1)), /PCM/);
    route.audioFrame('original', 0, 1400, tone(660, .7), null, null, clock(2));
    assert.throws(() => route.audioFrame('original', 400000, 1420, tone(660), null, null, clock(1)), /stale.*PCM/);
    for (let i = 1; i < 25; i++) {
        route.audioFrame('original', i * 20000, 1400 + i * 20, tone(660, i >= 18 ? .7 : .05), null, null, clock(2));
    }
    const envelope = route.report().sync.audio_envelope;
    assert.equal(envelope.segments.length, 2);
    assert.equal(envelope.onset_state_crossed_owner_epoch, false);
    assert.equal(envelope.accepted_onset_rows.length, 2, 'New epoch high cannot borrow the prior low rearm');
    assert.equal(envelope.accepted_onset_rows[1].timestamp_us, 360000);
    assert.equal(envelope.accepted_onset_rows[1].callback_seconds, 1.76);
    assert.equal(envelope.accepted_onset_rows[1].clock.epoch_id, 2);
    const nativeRoute = new DecodedRoute(0, 1);
    nativeRoute.start(0);
    const native = stream_generation => ({ssrc:7, rtpTimestamp:0, identity:{profile:'native-sequence-cursor/1',
        physical_incarnation:'00000000000000000000000000000001', connection_generation:1, stream_generation}});
    nativeRoute.audioFrame('native', 0, 1000, tone(660), null, native(1));
    nativeRoute.audioFrame('native', 0, 1020, tone(660), null, native(2));
    assert.throws(() => nativeRoute.audioFrame('native', 20000, 1040, tone(660), null, native(1)), /stale.*PCM/);
    assert.deepEqual(nativeRoute.report().sync.audio_envelope.segments.map(row => row.clock.stream_generation), [1, 2]);
    const interleaved = new DecodedRoute(0, 1);
    interleaved.start(0);
    for (let i = 0; i < 31; i++) {
        for (const [owner, offset] of [['old', 0], ['new', 1]]) {
            interleaved.audioFrame(owner, i * 20000, 1000 + i * 20 + offset,
                tone(660, i >= 15 && i < 20 ? .7 : .05));
        }
    }
    const interleavedEnvelope = interleaved.report().sync.audio_envelope;
    assert.equal(interleavedEnvelope.segments.length, 2, 'Interleaved retained owners do not invent decoder epochs');
    assert.equal(interleavedEnvelope.status, 'observer-valid');
    assert.equal(interleavedEnvelope.accepted_onset_rows.length, 2);
    const {RtcpReceiveClock} = await import('../../sdk/typescript/dist/browser_rtcp.js');
    const receiveClock = new RtcpReceiveClock(() => 1000000);
    receiveClock.register(1, 48000);
    assert.equal(receiveClock.report(1, {ssrc:7, ntp:100n << 32n, rtpTimestamp:0,
        cname:'actual-clock', packets:1, octets:160}), true);
    const firstTime = receiveClock.timestamp(1, 7, 0);
    const secondTime = receiveClock.timestamp(1, 7, 960);
    assert.equal(secondTime.timestampUs - firstTime.timestampUs, 19999, 'Actual double-floor conversion is not a PCM overlap');
    const precise = new DecodedRoute(0, 1);
    precise.start(0);
    const rtp = value => ({...native(1), rtpTimestamp:value});
    precise.audioFrame('native', firstTime.timestampUs, 1000, tone(660), null, rtp(0));
    precise.audioFrame('native', secondTime.timestampUs, 1020, tone(660), null, rtp(960));
    // A later report may change the mapped timestamp while RTP position stays exact.
    precise.audioFrame('native', firstTime.timestampUs, 1040, tone(660), null, rtp(1920));
    let preciseReport = precise.report();
    assert.equal(preciseReport.audio.maximum_pcm_sample_gap_ms, 20);
    assert.equal(preciseReport.audio.duplicate_frames, 0, 'Mapped NTP duplicates never replace actual RTP identity');
    assert.equal(precise.audio[1].timestamp - precise.audio[0].timestamp, 19999);
    assert.equal(precise.audio[2].timestamp, precise.audio[0].timestamp, 'Original AudioData timestamps remain untouched');
    precise.audioFrame('native', 2000000, 1060, tone(660), null, rtp(1920));
    assert.equal(precise.report().audio.duplicate_frames, 1, 'Actual duplicate RTP remains recorded and failed');
    for (const value of [1919, 1921, 2879, -1, 1.5, NaN, 2 ** 32]) {
        assert.throws(() => precise.audioFrame('native', 2020000, 1080, tone(660), null, rtp(value)), /PCM/);
    }
    const wrapped = new DecodedRoute(0, 1);
    wrapped.start(0);
    wrapped.audioFrame('native', 1000000, 1000, tone(660), null, rtp(0xfffffc40));
    wrapped.audioFrame('native', 1019999, 1020, tone(660), null, rtp(0));
    assert.equal(wrapped.report().audio.maximum_pcm_sample_gap_ms, 20, 'One unsigned32 wrap is actual960-tick progress');
    wrapped.audioFrame('native', 1500000, 1040, tone(660), null, rtp(19200));
    assert.equal(wrapped.report().audio.maximum_pcm_sample_gap_ms, 400, 'Real RTP gaps are never rounded away');
    assert.equal(wrapped.report().audio.passed, false);
});


test('source quantum attribution is explicitly diagnostic while complete decoded media retains its quality checks', () => {
    for (const mode of ['unbound', 'overlap', 'ambiguous']) {
        const report = observed({quantumAttribution:mode});
        assert.equal(report.passed, true, mode);
        assert.equal(report.audio.passed, true, mode);
        assert.equal(report.audio.quality_profile, 'connect/decoded-audio-quality/3');
        assert.equal(report.audio.source_quantum_scope, 'diagnostic-only-source-clock-attribution');
        assert.equal(report.audio.metric_scope, 'measurable-decoded-quality-source-quanta-diagnostic');
        assert.equal(report.audio.original_source_sample_coverage_proved, false);
        if (mode === 'overlap') {
            assert.equal(report.audio.original_source_quantum_overlaps, 1);
        } else {
            assert.ok(report.audio.source_quantum_identity_unbound_or_ambiguous > 0);
            const diagnostic = report.audio.source_quantum_diagnostics;
            assert.equal(diagnostic.events.length, 16);
            assert.ok(diagnostic.omitted > 0);
            assert.equal(diagnostic.events[0].reason, mode);
            assert.equal(typeof diagnostic.events[0].origin_ntp, 'string');
            assert.equal(diagnostic.events[0].identity.profile, 'native-sequence-cursor/1');
        }
    }
});

test('diagnostic source quanta never hide actual duplicate decoded frames wrong PCM or missing media', () => {
    for (const failure of [{duplicate:true}, {wrongSource:true}, {gap:true}]) {
        const report = observed({quantumAttribution:'unbound', ...failure});
        assert.equal(report.audio.passed, false);
        assert.equal(report.passed, false);
        assert.ok(report.audio.source_quantum_identity_unbound_or_ambiguous > 0);
        if (failure.duplicate) assert.equal(report.audio.duplicate_frames, 1);
        if (failure.wrongSource) assert.equal(report.audio.bad_identity_frames, 1);
        if (failure.gap) assert.ok(report.audio.maximum_gap_ms > 220);
    }
});


function provenanceNative(rtp,sequence,{ssrc=7,generation=1}={}) {
    return {profile:'connect/native-decoded-sequence/1',ssrc,rtpTimestamp:rtp,rtpSequence:sequence,
        datagramSequences:[sequence+100],identity:{profile:'native-sequence-cursor/1',
            physical_incarnation:'00000000000000000000000000000001',connection_generation:1,
            stream_generation:generation},sourceClock:{ntp:'1234',rtpTimestamp:0}};
}
function provenanceRoute(){const route=new DecodedRoute(0,1);route.start(0);return route;}
function provenanceFrame(route,at,rtp,sequence,options={}){
    const native=provenanceNative(rtp,sequence,options);
    route.audioFrame(options.owner??'original',sequence*20000,at,tone(660),null,native);
    return native;
}

test('audio provenance distinguishes a same-epoch PCM hole from callback batching or callback delay',()=>{
    const gap=provenanceRoute();
    provenanceFrame(gap,1000,0,1);provenanceFrame(gap,1001,960,2);provenanceFrame(gap,1001,3840,5);
    const report=gap.report(),witness=report.audio_provenance_diagnostics.gaps;
    assert.equal(witness.length,1);assert.equal(witness[0].pcm_step_ms,60);
    assert.equal(witness[0].previous.callback_seconds,witness[0].current.callback_seconds);
    assert.equal(witness[0].previous.native_rtp_timestamp,960);
    assert.equal(witness[0].current.native_rtp_timestamp,3840);
    assert.equal(witness[0].previous.native_rtp_sequence,2);
    assert.deepEqual(witness[0].current.native_datagram_sequences,[105]);
    assert.equal(report.audio.maximum_pcm_sample_gap_ms,60);
    const delayed=provenanceRoute();
    provenanceFrame(delayed,1000,0,1);provenanceFrame(delayed,1200,960,2);
    assert.deepEqual(delayed.report().audio_provenance_diagnostics.gaps,[],
        'A delayed callback with contiguous original PCM is not a source-position hole');
});

test('audio provenance never measures a gap across actual SSRC, owner or declared stream epochs',()=>{
    const route=provenanceRoute();
    provenanceFrame(route,1000,0,1);provenanceFrame(route,1020,960,2);
    provenanceFrame(route,1200,900000,3,{ssrc:8});
    provenanceFrame(route,1400,1900000,4,{ssrc:8,generation:2});
    provenanceFrame(route,1600,2900000,5,{ssrc:8,generation:2,owner:'target'});
    assert.deepEqual(route.report().audio_provenance_diagnostics.gaps,[]);
    provenanceFrame(route,1620,2902880,6,{ssrc:8,generation:2,owner:'target'});
    const pair=route.report().audio_provenance_diagnostics.gaps[0];
    assert.equal(pair.previous.owner,'target');assert.equal(pair.current.owner,'target');
    assert.deepEqual(pair.previous.clock,pair.current.clock);assert.equal(pair.pcm_step_ms,60);
});

test('audio provenance retains exact included window edges and leaves native quality scores unchanged',()=>{
    const full=provenanceRoute(),without=provenanceRoute();
    for(const [i,at]of [980,1000,60980,61000,61020].entries()){
        const native=provenanceNative(i*960,i+1);
        full.audioFrame('original',i*20000,at,tone(660),null,native);
        const bare={...native};delete bare.rtpSequence;delete bare.datagramSequences;
        without.audioFrame('original',i*20000,at,tone(660),null,bare);
    }
    const report=full.report(),edges=report.audio_provenance_diagnostics.window_edges;
    assert.equal(edges.last_before_start.callback_seconds,.98);
    assert.equal(edges.first_at_or_after_start.callback_seconds,1);
    assert.equal(edges.last_before_end.callback_seconds,60.98);
    assert.equal(edges.first_at_or_after_end.callback_seconds,61);
    const strip=({audio_provenance_diagnostics,...quality})=>quality;
    assert.deepEqual(strip(report),strip(without.report()));
    assert.equal(report.audio.decoded_seconds,.04,'Only original callbacks inside[1,61) earn existing credit');
    const filtered=full.report(row=>row.seconds!==1);
    assert.equal(filtered.audio_provenance_diagnostics.window_edges.first_at_or_after_start.callback_seconds,60.98,
        'Diagnostic window witnesses use the same existing public-membership filter');
    assert.equal(filtered.audio.decoded_seconds,.02);
});

test('audio provenance has fixed caps and detached immutable reports survive retirement',()=>{
    const route=provenanceRoute();let original;
    for(let i=0;i<40;i++)original=provenanceFrame(route,1000+i*60,i*2880,i+1);
    const report=route.report(),evidence=report.audio_provenance_diagnostics;
    assert.equal(evidence.gaps.length,32);assert.equal(evidence.omitted,7);
    assert.ok(evidence.gaps.every((pair,i)=>pair.current.row_index===i+1));
    assert.ok(Object.isFrozen(evidence)&&Object.isFrozen(evidence.gaps)&&Object.isFrozen(evidence.window_edges));
    assert.ok(Object.isFrozen(evidence.gaps[0].previous.clock));
    assert.throws(()=>evidence.gaps[0].current.native_datagram_sequences.push(99),TypeError);
    const saved=JSON.stringify(evidence);
    original.datagramSequences.push(999);original.sourceClock.ntp='5678';
    assert.equal(JSON.stringify(evidence),saved,'Valid caller metadata changes cannot mutate the detached report');
    provenanceFrame(route,3500,40*2880,41);route.report();
    assert.equal(JSON.stringify(evidence),saved,'Later observer activity never mutates returned evidence');
    route.retireWorkingState();
    assert.ok(Object.values(route.workingState()).every(value=>value===0));
    assert.equal(route.audioProvenance,null);
    assert.equal(JSON.stringify(evidence),saved,'Retirement releases working roots without erasing original reports');
});


test('bounded duplicate-video evidence preserves original identities and failure through retirement without inventing packet causality',()=>{
    const baseline=observed(),repeated=observed({videoDuplicate:true});
    assert.equal(baseline.passed,true);
    assert.equal(repeated.video.passed,false);assert.equal(repeated.video.duplicate_frames,1);
    assert.deepEqual(repeated.audio,baseline.audio);assert.deepEqual(repeated.sync,baseline.sync);
    assert.deepEqual(repeated.video.changing_frames_per_second,baseline.video.changing_frames_per_second,
        'No duplicate output acquires additional changing-frame credit');
    const pair=repeated.video.duplicate_diagnostics.events[0];
    assert.equal(pair.first.owner,'target');assert.equal(pair.current.owner,'target');
    assert.equal(pair.first.row_index,1000);assert.equal(pair.current.row_index,1001);
    assert.equal(pair.first.original_timestamp_us,Math.round(1000/30*1e6));
    assert.equal(pair.first.original_timestamp_us,pair.current.original_timestamp_us);
    assert.ok(pair.current.callback_seconds>pair.first.callback_seconds);
    assert.equal(pair.first.native,null);assert.equal(pair.current.native,null);
    assert.equal(pair.first.presentation,null);assert.equal(pair.current.presentation,null);

    const route=new DecodedRoute(0,1);route.start(0);
    const gray=Array(64).fill(100),pixels=new Uint8Array([1]);
    const firstNative={profile:'connect/native-decoded-sequence/1',ssrc:7,rtpTimestamp:90,rtpSequence:1,datagramSequences:[101]},
        currentNative={profile:'connect/native-decoded-sequence/1',ssrc:7,rtpTimestamp:91,rtpSequence:2,datagramSequences:[102]};
    const add=(owner,stamp,at,native=null)=>route.videoFrame(owner,stamp,at,gray,pixels,640,360,native);
    add('original',99,2000,firstNative);
    add('different-owner',99,2010);
    add('original',200,2020);
    assert.equal(route.videoDuplicateEvents.length,0,'Neither another owner nor repeated pixels with distinct timestamps collides');
    add('original',99,2030,currentNative);
    for(let i=0;i<19;i++)add('original',99,2040+i,currentNative);
    const report=route.report(),diagnostic=report.video.duplicate_diagnostics,saved=JSON.stringify(diagnostic);
    assert.equal(report.video.duplicate_frames,20);assert.equal(report.video.passed,false);
    assert.equal(diagnostic.events.length,16);assert.equal(diagnostic.omitted,4);
    assert.equal(diagnostic.source_frame_bits,null,'No unverified overlay format becomes source identity');
    const witness=diagnostic.events[0];
    assert.equal(witness.first.row_index,0);assert.equal(witness.current.row_index,3);
    assert.equal(witness.first.native.rtp_timestamp,90);assert.equal(witness.current.native.rtp_timestamp,91,
        'Original equal decoded timestamps do not manufacture equal RTP timestamps');
    assert.deepEqual(witness.first.native.datagram_sequences,[101]);assert.deepEqual(witness.current.native.datagram_sequences,[102]);
    assert.equal(route.report(row=>row.owner!=='original').video.duplicate_diagnostics.events.length,0,
        'The same original inclusion filter excludes both diagnostic owners');
    assert.equal(route.report(row=>row.seconds>2.01).video.duplicate_diagnostics.events.length,0,
        'A filtered-out first observation cannot become diagnostic evidence');
    firstNative.datagramSequences.push(999);currentNative.rtpTimestamp=999;
    assert.equal(JSON.stringify(diagnostic),saved,'Captured original headers are detached from caller metadata');
    assert.ok(Object.isFrozen(diagnostic)&&Object.isFrozen(diagnostic.events)&&Object.isFrozen(witness.first.native));
    assert.throws(()=>witness.current.native.datagram_sequences.push(999),TypeError);
    route.retireWorkingState();assert.ok(Object.values(route.workingState()).every(value=>value===0));
    assert.equal(JSON.stringify(diagnostic),saved,'Retirement releases working roots without erasing original duplicate evidence');
});
