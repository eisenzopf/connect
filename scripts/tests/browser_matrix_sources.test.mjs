import {readMatrixFrameOrdinal,matrixFrameIdentity} from '../lib/browser_matrix_frame_identity.mjs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {browserMatrixSources,matrixSourceFrame} from '../lib/browser_matrix_sources.mjs';
import {classifySourceFrame} from '../video/source_identity.mjs';

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
    const rectangles = new WeakMap();
    const createCanvas = () => {
        const rows = [];
        const paint = {fillStyle:'black', fillRect(x, y, width, height) {
            if (x === 0 && y === 0 && width === 640 && height === 360) rows.length = 0;
            const gray = /^rgb\((\d+),/.exec(this.fillStyle);
            const value = this.fillStyle === 'white' ? 255 : this.fillStyle === 'black' ? 0 : gray ? Number(gray[1]) : 100;
            rows.push({x,y,width,height,value});
        }};
        const canvas = {getContext:() => paint,
            captureStream(){throw Error('Only the public SDK may create derived canvas tracks');}};
        rectangles.set(canvas, rows);
        return canvas;
    };
    const sample = (canvas, width, height) => Uint8Array.from({length:width * height}, (_, index) => {
        const x = (index % width + .5) * 640 / width;
        const y = (Math.floor(index / width) + .5) * 360 / height;
        const pixel = rectangles.get(canvas).findLast(row => x >= row.x && x < row.x + row.width
            && y >= row.y && y < row.y + row.height);
        assert.ok(pixel, 'Actual source drawing must cover the sample');
        return pixel.value;
    });
    const sampleFooter = canvas => Array.from({length:11},(_,bit)=>{
        const x=(Math.floor((16+32*bit)/5)+.5)*5,y=357.5;
        return rectangles.get(canvas).findLast(row=>x>=row.x&&x<row.x+row.width&&y>=row.y&&y<row.y+row.height).value;
    });
    const values={AudioContext:Audio,MediaStream:Stream,performance:{now:()=>clock},
        document:{createElement:createCanvas},
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
        await run({owner,route,connection,captures,tracks,gains,audio,canvasCaptures,sample,sampleFooter,
            tick:(time,sourceTime)=>{clock=time;audio.currentTime=sourceTime;for(const fn of [...intervals])fn();},
            setClock:time=>{clock=time;},
            takeTimeout:()=>{const callback=[...timeouts][0];assert.ok(callback,'Original terminal timer required');timeouts.delete(callback);return callback;},
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
    assert.ok(masters.every(source=>source.canvas.width===0&&source.canvas.height===0));
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
        assert.equal(source.original_canvas_backing_released,true);
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


test('retired source observation stays immutable and exact after original graph/node references release',()=>fixture(async f=>{
    for(let i=0;i<6;i++)await f.owner.start({route:f.route(i),connection:f.connection('original-'+i),options:{}});
    const original=f.owner.beginProgramme(),closing=f.owner.close();await closing;
    const before=f.owner.observation(),references=f.owner.retireReferences(),receipt=await references;
    assert.equal(f.owner.retireReferences(),references);assert.equal(f.owner.close(),closing,'Reuse original AudioContext close');
    assert.equal(receipt.joined,true);assert.equal(receipt.released,true);assert.equal(f.owner.sources.length,0);
    const after=f.owner.observation();assert.deepEqual(after,before);assert.equal(after.programme,original);
    assert.equal(after,receipt.source_snapshot);assert.ok(Object.isFrozen(after)&&Object.isFrozen(after.sources)&&Object.isFrozen(after.sources[0]));
    assert.equal(f.audio.closed,1);assert.ok(f.captures.every(capture=>capture.closed));
    await assert.rejects(()=>f.owner.start({route:f.route(0),connection:f.connection('after-release'),options:{}}),/retired/);
    assert.deepEqual(f.owner.observation(),before,'Illegal later start cannot erase original source evidence');
}));

test('source reference release waits for the exact pending capture admission after graph close',()=>fixture(async f=>{
    let accept;const admitted=new Promise(resolve=>{accept=resolve;});
    const pending=f.owner.start({route:f.route(0),connection:{id:'pending',capture:()=>admitted},options:{}});
    const refusal=assert.rejects(pending,/retired during capture/);
    await f.owner.close();const originalRecord=f.owner.sources[0],references=f.owner.retireReferences();let settled=false;
    references.then(()=>{settled=true;});for(let i=0;i<8;i++)await Promise.resolve();
    assert.equal(settled,false);assert.equal(f.owner.sources[0],originalRecord);
    const handle={closed:false,close(){this.closed=true;}};accept(handle);await refusal;
    const receipt=await references;assert.equal(handle.closed,true);assert.equal(receipt.joined,true);
    assert.equal(f.owner.sources.length,0);assert.equal(f.owner.observation().sources.length,1);
}));

test('actual original source graph-close failure retains reference records and exact failure',async()=>{
    const original=Error('Original AudioContext close refusal');
    await assert.rejects(fixture(async f=>{
        await f.owner.start({route:f.route(0),connection:f.connection('original'),options:{}});
        const record=f.owner.sources[0];f.audio.close=async()=>{throw original;};
        await assert.rejects(f.owner.retireReferences(),error=>error===original);
        await assert.rejects(f.owner.close(),error=>error===original);
        assert.equal(f.owner.sources[0],record);assert.equal(f.audio.closed,0);
    }),error=>error===original);
});


test('early source terminal timer wakes keep all six original captures live until the exact original deadline',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    const original=f.owner.beginProgramme();
    for(const earlyBy of [1,.5]){
        f.setClock(original.deadlineMs-earlyBy);f.takeTimeout()();
        assert.equal(f.owner.observation().source_terminal,false);
        assert.strictEqual(f.owner.observation().programme,original,'Never rebase the original clock');
        assert.ok(f.owner.sources.every(source=>source.ownerLive()));
        assert.ok(f.captures.every(capture=>!capture.closed));assert.ok(f.tracks.every(track=>track.readyState==='live'));
        assert.deepEqual(f.timers(),{intervals:6,timeouts:1},'Rearm only the same original terminal wait');
    }
    f.setClock(original.deadlineMs);f.takeTimeout()();
    assert.equal(f.owner.observation().source_terminal,true);
    assert.ok(f.captures.every(capture=>capture.closed));assert.ok(f.tracks.every(track=>track.readyState==='ended'));
    assert.deepEqual(f.timers(),{intervals:0,timeouts:0});
}));

test('a dequeued source terminal callback cannot rearm after explicit source close or reference release',()=>fixture(async f=>{
    for(let index=0;index<6;index++)await f.owner.start({route:f.route(index),connection:f.connection('original-'+index),options:{}});
    const original=f.owner.beginProgramme(),pending=f.takeTimeout();
    await f.owner.close();
    assert.equal(f.audio.closed,1);assert.ok(f.captures.every(capture=>capture.closed));
    f.setClock(original.deadlineMs-1);pending();
    assert.deepEqual(f.timers(),{intervals:0,timeouts:0},'Explicit close stays immediate before the programme deadline');
    const receipt=await f.owner.retireReferences();assert.equal(receipt.joined,true);
    pending();
    assert.deepEqual(f.timers(),{intervals:0,timeouts:0},'A late callback cannot resurrect a released source owner');
    assert.equal(f.audio.closed,1);
}));


test('actual source pulse frames keep white identity centres and distinct motion samples', () => fixture(async f => {
    for (let index = 0; index < 6; index++) {
        await f.owner.start({route:f.route(index),connection:f.connection('pulse-' + index),options:{}});
    }
    const programme = f.owner.beginProgramme(), frames = [];
    for (const elapsed of [301, 335, 368]) {
        f.tick(programme.originMs + elapsed, programme.audioOriginSeconds + elapsed / 1000);
        frames.push(f.owner.sources.map(source => ({identity:f.sample(source.canvas, 8, 8),
            change:f.sample(source.canvas, 128, 72)})));
    }
    for (let index = 0; index < 6; index++) {
        assert.ok(frames.every(frame => classifySourceFrame(frame[index].identity) === 'pulse'));
        assert.deepEqual(frames[0][index].identity, frames[1][index].identity);
        assert.ok(frames[0][index].change.some((value, pixel) => value !== frames[1][index].change[pixel]));
        assert.ok(frames[1][index].change.some((value, pixel) => value !== frames[2][index].change[pixel]));
    }
    assert.equal(matrixSourceFrame(399).pulse, true);
    assert.equal(matrixSourceFrame(400).pulse, false);
}));

test('actual source footer preserves source identity and never changes a repeated source frame', () => fixture(async f => {
    for (let index = 0; index < 6; index++) {
        await f.owner.start({route:f.route(index),connection:f.connection('identity-' + index),options:{}});
    }
    const programme = f.owner.beginProgramme();
    f.tick(programme.originMs + 1001, programme.audioOriginSeconds + 1.001);
    const first = f.owner.sources.map(source => f.sample(source.canvas, 128, 72));
    f.tick(programme.originMs + 1001, programme.audioOriginSeconds + 1.001);
    for (const [index, source] of f.owner.sources.entries()) {
        assert.deepEqual(f.sample(source.canvas, 128, 72), first[index]);
        assert.equal(classifySourceFrame(f.sample(source.canvas, 8, 8)), String(source.sourceId));
    }
    f.tick(programme.originMs + 1035, programme.audioOriginSeconds + 1.035);
    for (const [index, source] of f.owner.sources.entries()) {
        const next = f.sample(source.canvas, 128, 72);
        assert.ok(first[index].some((value, pixel) => value !== next[pixel]));
        assert.equal(classifySourceFrame(f.sample(source.canvas, 8, 8)), String(source.sourceId));
    }
}));

// Geometry of the ORIGINAL painter, independent of codec/downsampling proof.
test('actual canvas painter tags preprogramme separately and preserves all1860 programme footer IDs and source/cadence pixels',()=>fixture(async f=>{
 for(let source=0;source<6;source++)await f.owner.start({route:f.route(source),connection:f.connection('footer-source-'+source),options:{}});
 const rgba=new Uint8ClampedArray(128*72*4);for(let i=3;i<rgba.length;i+=4)rgba[i]=255;
 const read=source=>{const values=f.sampleFooter(source.canvas);for(let bit=0;bit<11;bit++){const offset=(71*128+Math.floor((16+32*bit)/5))*4;rgba[offset]=rgba[offset+1]=rgba[offset+2]=values[bit];}return readMatrixFrameOrdinal(rgba);};
 for(const source of f.owner.sources){assert.equal(read(source).ordinal,2047);assert.equal(matrixFrameIdentity(rgba,String(source.sourceId),source.sourceId).status,'unavailable');}
 const programme=f.owner.beginProgramme();
 for(let ordinal=0;ordinal<1860;ordinal++){
  const elapsed=ordinal*1000/30+.001;f.tick(programme.originMs+elapsed,programme.audioOriginSeconds+elapsed/1000);
  for(const source of f.owner.sources){assert.equal(read(source).ordinal,ordinal);const kind=classifySourceFrame(f.sample(source.canvas,8,8));
   const identity=matrixFrameIdentity(rgba,kind,source.sourceId);assert.equal(identity.status,'identified');assert.equal(identity.ordinal,ordinal);
   assert.equal(identity.observed_source_id,kind==='pulse'?null:source.sourceId);
  }
 }
}));
