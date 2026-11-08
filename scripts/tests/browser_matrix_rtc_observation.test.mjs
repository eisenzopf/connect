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

async function warmSourceObservation(run, { own = false } = {}) {
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
