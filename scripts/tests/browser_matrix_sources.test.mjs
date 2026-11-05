import assert from 'node:assert/strict';
import {test} from 'node:test';
import {browserMatrixSources,matrixSourceFrame} from '../lib/browser_matrix_sources.mjs';

// Ownership/clock boundary fixtures only, not browser decoding or media proof.
async function fixture(run) {
    const keys=['AudioContext','MediaStream','document','performance','setInterval','clearInterval','setTimeout','clearTimeout'];
    const originals=keys.map(key=>[key,Object.getOwnPropertyDescriptor(globalThis,key)]);
    const intervals=new Set(),timeouts=new Set(),tracks=[],gains=[],canvasCaptures=[];let audio,clock=0;
    class Track {
        readyState='live';requests=0;
        constructor(kind){
            this.kind=kind;tracks.push(this);
        }
        clone(){return new Track(this.kind);}
        stop(){this.readyState='ended';}
        requestFrame(){this.requests++;}
    }
    class Stream {
        constructor(tracks){this.tracks=tracks;}
        getTracks(){return this.tracks;}
        getAudioTracks(){return this.tracks.filter(track=>track.kind==='audio');}
        getVideoTracks(){return this.tracks.filter(track=>track.kind==='video');}
    }
    class Audio {
        state='suspended';currentTime=10;closed=0;
        constructor(){audio=this;}
        async resume(){this.state='running';}
        async close(){this.state='closed';this.closed++;}
        createOscillator(){return {frequency:{},connect(){},start(){},stop(){this.onended?.();},disconnect(){}};}
        createGain(){const events=[];gains.push(events);return {gain:{setValueAtTime:(value,time)=>events.push({value,time})},connect(){},disconnect(){}};}
        createMediaStreamDestination(){throw Error('No unused source destination or master tracks');}
    }
    const values={AudioContext:Audio,MediaStream:Stream,performance:{now:()=>clock},
        document:{createElement:()=>({getContext:()=>({fillRect(){}}),captureStream(){throw Error('Only the public SDK may create derived canvas tracks');}})},
        setInterval:fn=>{intervals.add(fn);return fn;},clearInterval:fn=>intervals.delete(fn),
        setTimeout:fn=>{timeouts.add(fn);return fn;},clearTimeout:fn=>timeouts.delete(fn)};
    for(const [key,value] of Object.entries(values))Object.defineProperty(globalThis,key,{configurable:true,value});
    let owner;
    const route=index=>({index,role:'participant-'+([0,1,2,3,0,1][index]+1)});
    const captures=[];
    const connection=id=>({id,capture:async({stream,audioNode,videoCanvas})=>{
        assert.equal(stream,undefined);assert.ok(videoCanvas);
        const derived=new Track('video');
        const handle={closed:false,audioNode,videoCanvas,derived,close(){this.closed=true;derived.stop();}};
        captures.push(handle);return handle;
    }});
    try{
        owner=await browserMatrixSources();
        await run({owner,route,connection,captures,tracks,gains,audio,canvasCaptures,
            tick:(time,sourceTime)=>{clock=time;audio.currentTime=sourceTime;for(const fn of [...intervals])fn();},
            timers:()=>({intervals:intervals.size,timeouts:timeouts.size})});
    }finally{
        await owner?.close();
        for(const[key,descriptor]of originals)if(descriptor)Object.defineProperty(globalThis,key,descriptor);else delete globalThis[key];
    }
}
test('matrix source pulses retain fixed 100ms duration and a fixed 62s endpoint',()=>{
    assert.equal(matrixSourceFrame(-1),null);assert.equal(matrixSourceFrame(62000),null);
    assert.equal(matrixSourceFrame(299).pulse,false);assert.equal(matrixSourceFrame(300).pulse,true);
    assert.equal(matrixSourceFrame(399).pulse,true);assert.equal(matrixSourceFrame(400).pulse,false);
    assert.equal(matrixSourceFrame(15000).frame,450);assert.equal(matrixSourceFrame(40000).frame,1200);
    assert.equal(matrixSourceFrame(61999).frame,1859);assert.throws(()=>matrixSourceFrame(NaN));
});
test('two device moves retain source masters, identity and the original programme clock',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    const masters=f.owner.sources.map(source=>({canvas:source.canvas,gain:source.gain})),programme=f.owner.beginProgramme();
    assert.equal(programme.audioCaptureMode,'caller-owned-audio-node');
    assert.equal(f.owner.observation().capture_input_mode,'caller-owned-audio-node+caller-owned-canvas');
    for(const [index,capture]of f.captures.entries()){assert.equal(capture.audioNode,f.owner.sources[index].gain);assert.equal(capture.videoCanvas,f.owner.sources[index].canvas);assert.equal(capture.derived.readyState,'live');}
    assert.ok(f.owner.observation().sources.every(source=>source.canvas_capture_mode==='sdk-owned-native-canvas-per-connection'));
    assert.equal(programme.videoCaptureMode,'caller-owned-canvas');assert.deepEqual(f.canvasCaptures,[]);
    assert.equal(programme.deadlineMs-programme.originMs,62000);assert.ok(Object.isFrozen(programme));
    assert.deepEqual(f.gains.slice(1),Array(5).fill(f.gains[0]));
    f.tick(programme.originMs,programme.audioOriginSeconds);
    f.owner.retireConnection('original-0');
    assert.ok(f.captures[0].closed);assert.ok(f.owner.sources.every(source=>source.ownerLive()));
    f.tick(programme.originMs+15000,programme.audioOriginSeconds+15);
    await f.owner.start({route:f.route(0),connection:f.connection('switch'),options:{}});
    f.owner.retireConnection('switch');
    f.tick(programme.originMs+40000,programme.audioOriginSeconds+40);
    await f.owner.start({route:f.route(0),connection:f.connection('return'),options:{}});
    const report=f.owner.observation();assert.strictEqual(report.programme,programme);
    assert.equal(report.sources[0].programme_draws,3);assert.equal(report.sources[0].last_programme_frame,1200);
    assert.equal(report.sources[0].skipped_programme_frames,1198);
    assert.equal(report.sources[0].attachments.length,3);assert.ok(report.sources[0].original_source_owner_live);
    assert.deepEqual(f.owner.sources.map(source=>({canvas:source.canvas,gain:source.gain})),masters);
    assert.throws(()=>f.owner.beginProgramme(),/One running original programme/);
    await assert.rejects(()=>f.owner.start({route:f.route(0),connection:f.connection('extra'),options:{}}),/one switch and one return/);
    await f.owner.close();assert.ok(f.tracks.every(track=>track.readyState==='ended'));assert.equal(f.audio.closed,1);
    assert.deepEqual(f.timers(),{intervals:0,timeouts:0});
}));

test('source ownership is not inferred from an empty track list or a retired graph',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    assert.equal(f.tracks.length,6,'Only the six capture-owned test tracks exist');
    assert.ok(f.owner.observation().sources.every(source=>source.original_source_owner_live));
    f.audio.state='suspended';
    assert.ok(f.owner.observation().sources.every(source=>!source.original_source_owner_live));
    assert.throws(()=>f.owner.beginProgramme(),/One running original programme/);
    f.audio.state='running';f.owner.sources[0].canvas.width=641;
    assert.equal(f.owner.observation().sources[0].original_source_owner_live,false);
    assert.throws(()=>f.owner.beginProgramme(),/One running original programme/);
    f.owner.sources[0].canvas.width=640;f.owner.beginProgramme();await f.owner.close();
    for(const source of f.owner.observation().sources){
        assert.equal(source.original_source_owner_live,false);assert.equal(source.original_source_timer_active,false);
        assert.equal(source.original_source_retired,true);assert.equal(source.original_oscillator_ended,true);
        assert.ok(source.attachments.every(binding=>binding.retired));
        assert.equal('original_master_tracks_live' in source,false);
    }
}));
test('a stalled source audio clock cannot extend the original monotonic terminal',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    const programme=f.owner.beginProgramme();
    f.tick(programme.originMs,programme.audioOriginSeconds);
    f.tick(programme.deadlineMs,programme.audioOriginSeconds);
    const report=f.owner.observation();assert.equal(report.source_terminal,true);
    assert.ok(report.sources.every(source=>source.programme_draws===1));
    assert.ok(f.tracks.every(track=>track.readyState==='ended'));assert.ok(f.captures.every(capture=>capture.closed));
    assert.deepEqual(f.timers(),{intervals:0,timeouts:0});
    await assert.rejects(()=>f.owner.start({route:f.route(0),connection:f.connection('late'),options:{}}),/retired/);
}));
test('capture resolving after source closure is retired rather than escaping to the replacement',()=>fixture(async f=>{
    let accept;const pending=f.owner.start({route:f.route(0),connection:{id:'pending',capture:()=>new Promise(resolve=>{accept=resolve;})},options:{}});
    await f.owner.close();let closed=0;accept({close(){closed++;}});
    await assert.rejects(()=>pending,/retired during capture/);assert.equal(closed,1);
    assert.ok(f.tracks.every(track=>track.readyState==='ended'));assert.equal(f.audio.closed,1);
}));

test('quantized audio clock polling emits each current source frame once without changing its cadence',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    const original=f.owner.beginProgramme();
    for(let elapsed=0;elapsed<1000;elapsed+=5){
        const quantum=Math.floor(elapsed/(128/48))*128/48000;
        f.tick(original.originMs+elapsed,original.audioOriginSeconds+quantum);
        f.tick(original.originMs+elapsed,original.audioOriginSeconds+quantum);
    }
    for(const source of f.owner.observation().sources){
        assert.equal(source.programme_draws,30);assert.equal(source.last_programme_frame,29);
        assert.equal(source.skipped_programme_frames,0);
    }
    assert.strictEqual(f.owner.observation().programme,original);
}));
