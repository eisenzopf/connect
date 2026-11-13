import {rtcAudioBoundaryDiagnostics} from '../lib/browser_matrix_rtc_audio_diagnostics.mjs';
import {DecodedRoute} from '../lib/browser_matrix_observation.mjs';
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
    const keys=['AudioContext','MediaStreamAudioSourceNode','AudioWorkletNode','CanvasRenderingContext2D','HTMLVideoElement'];
    const saved=Object.fromEntries(keys.map(key=>[key,globalThis[key]]));
    const nodes=[],contexts=[],audio=[],video=[],errors=[];
    class MediaStreamAudioSourceNode {
        constructor(context){this.context=context;}
        connect(){}
        disconnect(){}
    }
    class AudioContext {
        state='running';destination={};audioWorklet={addModule:async()=>{}};
        constructor(){contexts.push(this);}
        createMediaStreamSource(){return new MediaStreamAudioSourceNode(this);}
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
        render(){const context=new AudioContext();context.createMediaStreamSource({}).connect(context.destination);return Promise.resolve({});}
    }
    Object.assign(globalThis,{AudioContext,MediaStreamAudioSourceNode,AudioWorkletNode,CanvasRenderingContext2D,HTMLVideoElement});
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
        element.callback(100,{mediaTime:1.25,presentedFrames:1});
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

async function warmSourceObservation(run, { own = false, measurementOrigin = null } = {}) {
  const names = [
    "AudioContext",
    "MediaStreamAudioSourceNode",
    "AudioWorkletNode",
    "CanvasRenderingContext2D",
    "HTMLVideoElement",
  ];
  const saved = names.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]);
  const nodes = [],
    audio = [],
    errors = [];
  class AudioNode {
    connect(node) {
      this.connections.push(node);
      return node;
    }
    disconnect(node) {
      this.connections = this.connections.filter((value) => value !== node);
    }
  }
  class MediaStreamAudioSourceNode extends AudioNode {
    constructor(context) {
      super();
      this.context = context;
      this.connections = [];
    }
  }
  if (own)
    Object.defineProperty(MediaStreamAudioSourceNode.prototype, "connect", {
      value: AudioNode.prototype.connect,
      writable: true,
      configurable: true,
      enumerable: false,
    });
  class AudioContext {
    state = "running";
    destination = {};
    audioWorklet = { async addModule() {} };
    createGain() {
      return { gain: { value: 0 }, connect() {}, disconnect() {} };
    }
  }
  class AudioWorkletNode {
    port = { postMessage() {}, close() {} };
    constructor() {
      nodes.push(this);
    }
    connect() {}
    disconnect() {}
  }
  class CanvasRenderingContext2D {
    drawImage() {}
  }
  class HTMLVideoElement {
    requestVideoFrameCallback() {}
  }
  const context = new AudioContext(),
    source = new MediaStreamAudioSourceNode(context),
    muted = context.createGain();
  source.connect(muted);
  class Rtc {
    input = { connectionId: "same-target" };
    render() {
      const gain = context.createGain();
      source.connect(gain);
      source.connect(gain);
      return Promise.resolve({});
    }
  }
  Object.assign(globalThis, {
    AudioContext,
    MediaStreamAudioSourceNode,
    AudioWorkletNode,
    CanvasRenderingContext2D,
    HTMLVideoElement,
  });
  const descriptor = Object.getOwnPropertyDescriptor(
    MediaStreamAudioSourceNode.prototype,
    "connect",
  );
  const original = MediaStreamAudioSourceNode.prototype.connect;
  const observer = browserMatrixRtcObservation(Rtc, {
    measurementOrigin: () => measurementOrigin,
    audio: (...value) => audio.push(value),
    video() {},
    invalid: (...value) => errors.push(value),
  });
  try {
    await run({
      Rtc,
      context,
      source,
      muted,
      nodes,
      audio,
      errors,
      observer,
      descriptor,
      original,
      MediaStreamAudioSourceNode,
    });
  } finally {
    await observer.close();
    assert.deepEqual(
      Object.getOwnPropertyDescriptor(
        MediaStreamAudioSourceNode.prototype,
        "connect",
      ),
      descriptor,
    );
    assert.equal(MediaStreamAudioSourceNode.prototype.connect, original);
    for (const [name, value] of saved)
      if (value) Object.defineProperty(globalThis, name, value);
      else delete globalThis[name];
  }
}

test("RTC observer captures the actual already-warm source public connection once and excludes private graph connections", () =>
  warmSourceObservation(async (f) => {
    f.source.connect(f.muted);
    await Promise.resolve();
    assert.equal(f.nodes.length, 0);
    await new f.Rtc().render();
    await Promise.resolve();
    assert.equal(f.nodes.length, 1);
    assert.equal(f.source.context, f.context);
    assert.ok(f.source.connections.includes(f.nodes[0]));
    f.nodes[0].port.onmessage({
      data: {
        type: "pcm",
        samples: new Float32Array(960).fill(0.125),
        timestamp: 99,
      },
    });
    assert.equal(f.audio.length, 1);
    assert.equal(f.audio[0][0], "same-target");
    assert.equal(f.audio[0][1], 99);
    assert.deepEqual(f.errors, []);
  }));

test("RTC observer restores original inherited and own source connect descriptors after public adoption", async () => {
  for (const own of [false, true])
    await warmSourceObservation(
      async (f) => {
        assert.equal(
          Object.hasOwn(f.MediaStreamAudioSourceNode.prototype, "connect"),
          true,
        );
        await new f.Rtc().render();
        await Promise.resolve();
        assert.equal(f.nodes.length, 1);
        await f.observer.close();
        assert.equal(
          Object.hasOwn(f.MediaStreamAudioSourceNode.prototype, "connect"),
          own,
        );
        assert.deepEqual(
          Object.getOwnPropertyDescriptor(
            f.MediaStreamAudioSourceNode.prototype,
            "connect",
          ),
          f.descriptor,
        );
      },
      { own },
    );
});


test('RTC presentation identity uses each real element counter and refuses repeats without inventing skipped frames', async () => {
    const keys = ['MediaStreamAudioSourceNode', 'CanvasRenderingContext2D', 'HTMLVideoElement'];
    const saved = keys.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]);
    class MediaStreamAudioSourceNode { connect() {} }
    class CanvasRenderingContext2D {
        constructor(canvas) { this.canvas = canvas; }
        drawImage() {}
    }
    class HTMLVideoElement {
        videoWidth = 640;
        videoHeight = 360;
        requestVideoFrameCallback(callback) { this.callback = callback; return 1; }
    }
    class Rtc { render() {} }
    Object.assign(globalThis, {MediaStreamAudioSourceNode, CanvasRenderingContext2D, HTMLVideoElement});
    const received = [], refused = [];
    const observer = browserMatrixRtcObservation(Rtc, {
        audio() {},
        video(...args) { received.push(args); },
        invalid(...args) { refused.push(args); },
    });
    try {
        const firstCanvas = {}, secondCanvas = {};
        observer.attach('original', firstCanvas);
        observer.attach('replacement', secondCanvas);
        const first = new HTMLVideoElement(), second = new HTMLVideoElement();
        const paint = (element, canvas, presentedFrames, extra = {}) => {
            const context = new CanvasRenderingContext2D(canvas);
            element.requestVideoFrameCallback(() => context.drawImage(element, 0, 0));
            element.callback(100, {mediaTime:0, presentedFrames, rtpTimestamp:42,
                presentationTime:90, expectedDisplayTime:100, ...extra});
        };
        paint(first, firstCanvas, 1);
        paint(first, firstCanvas, 3);
        paint(second, secondCanvas, 1);
        assert.equal(received.length, 3, 'A skipped presented counter adds no synthetic callback');
        assert.deepEqual(received.map(row => row[1]), [0, 0, 0], 'Original media time stays unchanged');
        assert.deepEqual(received.map(row => row[6].presentedFrames), [1, 3, 1]);
        assert.equal(received[0][6].element_id, received[1][6].element_id);
        assert.notEqual(received[0][6].element_id, received[2][6].element_id);
        assert.equal(received[0][6].rtpTimestamp, 42);
        assert.equal(received[0][6].callbackTime, 100);
        for (const value of [3, 2, 0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, undefined]) {
            paint(first, firstCanvas, value);
        }
        paint(first, secondCanvas, 4);
        assert.equal(received.length, 3, 'Repeated, backward, malformed and foreign-owner metadata gets no frame credit');
        assert.equal(refused.length, 10);
        assert.equal(refused[0][2].mediaTime, 0, 'Refusal diagnostics retain actual raw metadata');
    } finally {
        await observer.close();
        for (const [key, descriptor] of saved) {
            if (descriptor) Object.defineProperty(globalThis, key, descriptor);
            else delete globalThis[key];
        }
    }
});


const boundaryClock=Object.freeze({profile:'connect/rtc-decoded-pcm-clock/1',owner:'original',epoch_id:1});
function boundaryPcm(){
    const samples=Float32Array.from({length:960},(_,i)=>.1*Math.sin((i+1)*.08));
    samples.fill(0,192,672);return samples;
}
function boundaryInput(samples,context,extra={}){
    return {owner:'original',clock:boundaryClock,timestamp:20000,at:1200,origin:1000,samples,context,...extra};
}

test('RTC boundary evidence preserves copied480-zero input and never invents a zero event for a tone',()=>{
    const diagnostics=rtcAudioBoundaryDiagnostics(),samples=boundaryPcm(),original=samples.slice();
    const context={sampleRate:48000,currentTime:1.1,baseLatency:.01,outputLatency:.02,
        getOutputTimestamp(){return {contextTime:1.09,performanceTime:1190};}};
    diagnostics.observe(boundaryInput(samples,context));
    assert.deepEqual(samples,original);
    const report=diagnostics.report(),row=report.events[0];
    assert.equal(row.exact_zero_run_samples,480);assert.equal(row.exact_zero_run_start_sample,192);
    assert.equal(row.exact_zero_run_end_sample_exclusive,672);
    assert.equal(row.exact_zero_start_worklet_us,24000);assert.equal(row.exact_zero_end_worklet_us,34000);
    assert.equal(row.callback_seconds,.2);assert.equal(row.original_worklet_timestamp_us,20000);
    assert.equal(row.context.output_timestamp.performance_time_ms,1190);
    assert.ok(Object.isFrozen(row)&&Object.isFrozen(row.clock)&&Object.isFrozen(row.context.output_timestamp));
    diagnostics.observe(boundaryInput(Float32Array.from({length:960},(_,i)=>.1*Math.sin(i*.08)),context));
    assert.equal(diagnostics.report().events.length,1);
    samples.fill(1);context.currentTime=99;
    assert.equal(report.events[0].context.current_time_seconds,1.1);
    assert.equal(Object.values(row).some(value=>ArrayBuffer.isView(value)),false);
});

test('RTC boundary metadata never reads optional context APIs before measurement or beyond its bound',()=>{
    const diagnostics=rtcAudioBoundaryDiagnostics();let calls=0;
    const context={get currentTime(){calls++;throw Error('unavailable');},get baseLatency(){throw Error('unavailable');},
        outputLatency:NaN,getOutputTimestamp(){calls++;throw Error('unavailable');}};
    const input=boundaryInput(boundaryPcm(),context);
    for(const origin of [null,undefined,NaN,2000,-100000])diagnostics.observe({...input,origin});
    assert.equal(calls,0);assert.equal(diagnostics.report().events.length,0);
    for(let i=0;i<40;i++)diagnostics.observe({...input,timestamp:i*20000,at:1200+i*20});
    const report=diagnostics.report();assert.equal(report.events.length,32);assert.equal(report.omitted,8);
    assert.equal(calls,64,'Two unavailable getters read only for each retained anomaly');
    assert.ok(report.events.every(row=>row.context.current_time_seconds===null&&row.context.base_latency_seconds===null
        &&row.context.output_latency_seconds===null&&row.context.output_timestamp===null));
    assert.throws(()=>report.events.push({}),TypeError);
    diagnostics.close();diagnostics.observe(input);assert.deepEqual(diagnostics.report(),report);assert.equal(calls,64);
});

test('RTC worklet input zero evidence does not change the existing decoded quality result',()=>{
    const route=new DecodedRoute(0,1);route.start(0);
    const {context,messages,owner}=processor();
    const input=new Float32Array(1024).fill(.125);input.fill(0,192,672);
    for(let i=0;i<input.length;i+=128){context.currentFrame=i;owner.process([[input.subarray(i,i+128)]]);}
    assert.equal(messages.length,1);assert.deepEqual(Array.from(messages[0].samples),Array.from(input.subarray(0,960)));
    route.audioFrame('original',messages[0].timestamp,1000,messages[0].samples,null,null,boundaryClock);
    const before=route.report(),diagnostics=rtcAudioBoundaryDiagnostics();
    diagnostics.observe(boundaryInput(messages[0].samples,{}, {timestamp:messages[0].timestamp,at:1000,origin:0}));
    assert.equal(diagnostics.report().events[0].exact_zero_run_samples,480);
    assert.deepEqual(route.report(),before,'Evidence collection cannot add identity, frame credit, or a quality exception');
});

test('RTC boundary report uses original renderer and survives close without retaining live graph callbacks',()=>
    warmSourceObservation(async f=>{
        f.context.sampleRate=48000;f.context.currentTime=3;f.context.baseLatency=.01;
        await new f.Rtc().render();await Promise.resolve();
        assert.equal(f.nodes.length,1,'No second diagnostic tap or render graph');
        const samples=boundaryPcm();
        f.nodes[0].port.onmessage({data:{type:'pcm',timestamp:120000,samples}});
        assert.equal(f.audio.length,1);assert.equal(f.audio[0][3],samples,'The original PCM callback remains unchanged');
        const report=f.observer.report();assert.equal(report.events.length,1);
        assert.equal(report.events[0].owner,'same-target');assert.equal(report.events[0].clock.owner,'same-target');
        assert.equal(report.events[0].context.current_time_seconds,3);
        await f.observer.close();
        assert.equal(f.nodes[0].port.onmessage,null);assert.equal(f.nodes[0].onprocessorerror,null);
        assert.ok(!f.source.connections.includes(f.nodes[0]));
        f.context.currentTime=9;assert.deepEqual(f.observer.report(),report);
        assert.deepEqual(f.errors,[]);
    },{measurementOrigin:performance.now()-1000}));


test('optional suspect PCM owns only real before/current/after blocks and preserves unchanged input and quality',()=>{
    const diagnostics=rtcAudioBoundaryDiagnostics({retainSuspectPcm:true}),context={};
    const tone=Float32Array.from({length:960},(_,i)=>.1*Math.sin(i*.08)),bad=boundaryPcm();
    diagnostics.observe(boundaryInput(tone,context,{timestamp:0,at:1100}));
    diagnostics.observe(boundaryInput(bad,context,{timestamp:20000,at:1120}));
    const unfinished=diagnostics.report();assert.equal(unfinished.suspect_pcm.complete,false);
    diagnostics.observe(boundaryInput(tone,context,{timestamp:40000,at:1140}));
    const done=diagnostics.report(),pcm=done.suspect_pcm;
    assert.equal(pcm.complete,true);assert.equal(pcm.events.length,1);
    assert.deepEqual(pcm.events[0].before.samples,Array.from(tone));
    assert.deepEqual(pcm.events[0].current.samples,Array.from(bad));
    assert.deepEqual(pcm.events[0].after.samples,Array.from(tone));
    assert.equal(unfinished.suspect_pcm.events[0].after,null,'A published report never gains a later synthetic neighbor');
    const saved=JSON.stringify(pcm);tone.fill(8);bad.fill(9);
    assert.equal(JSON.stringify(pcm),saved,'Borrowed arrays cannot mutate original evidence');
    assert.throws(()=>pcm.events[0].current.samples.push(1),TypeError);
    diagnostics.close();assert.equal(JSON.stringify(diagnostics.report().suspect_pcm),saved);
    assert.equal(rtcAudioBoundaryDiagnostics().report().suspect_pcm.enabled,false);
});

test('optional PCM evidence refuses unknown owners, bounds output and never fabricates a discontinuous neighbor',()=>{
    const diagnostics=rtcAudioBoundaryDiagnostics({retainSuspectPcm:true}),input=boundaryInput(boundaryPcm(),{});
    diagnostics.observe(input);
    diagnostics.observe({...input,timestamp:60000,at:1260});
    assert.equal(diagnostics.report().suspect_pcm.events[0].after_status,'discontinuous');
    for(let i=0;i<20;i++)diagnostics.observe({...input,timestamp:80000+i*20000,at:1280+i*20});
    const captured=diagnostics.report().suspect_pcm;
    assert.equal(captured.events.length,8);assert.ok(captured.omitted>0);assert.equal(captured.complete,false);
    diagnostics.observe({...input,owner:'foreign',clock:{...input.clock,owner:'foreign'}});
    assert.ok(diagnostics.report().suspect_pcm.owner_unknown>0,'A reused clock cannot rebind the physical owner');
    diagnostics.close();const stopped=JSON.stringify(diagnostics.report());diagnostics.observe(input);
    assert.equal(JSON.stringify(diagnostics.report()),stopped);
});
