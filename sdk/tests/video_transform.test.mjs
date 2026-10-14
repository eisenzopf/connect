import assert from 'node:assert/strict';
import { test } from 'node:test';
import fs from 'node:fs';
import vm from 'node:vm';

// Exercise the actual compiled passthrough worker with controlled frame metadata,
// streams and keyframe promises. This does not simulate encoded content/decode.
test('encoded worker distinguishes pending keyframe requests from completion and observed keyframes', async () => {
    let now=0,resolveKey,rejectKey;
    const failures=[];const emitted=[];
    const scope={performance:{now:()=>now},Number,Math,Promise,TransformStream,setTimeout,clearTimeout,DOMException,
        postMessage:value=>failures.push(value)};
    vm.runInNewContext(fs.readFileSync(new URL('../typescript/dist/browser_video_transform.js',import.meta.url),'utf8')
        .replace(/^export \{\};/m,''),scope);
    let source;
    const readable=new ReadableStream({start:controller=>{source=controller;}});
    const writable=new WritableStream({write:frame=>emitted.push(frame)});
    let requests=0;
    scope.onrtctransform({transformer:{readable,writable,generateKeyFrame:()=>{
        requests++;return new Promise((resolve,reject)=>{resolveKey=resolve;rejectKey=reject;});}}});
    let releasedPortClosed=false;
    assert.doesNotThrow(()=>scope.onmessage({data:{op:'video-diagnostics'},ports:[{
        postMessage:()=>{throw Error('already released');},close:()=>{releasedPortClosed=true;}}]}));
    assert.equal(releasedPortClosed,true);
    const tick=()=>new Promise(resolve=>setTimeout(resolve,0));
    const sample=()=>{
        let snapshot,closed=0;
        scope.onmessage({data:{op:'video-diagnostics'},ports:[{postMessage:value=>{snapshot={...value};},close:()=>closed++}]});
        assert.equal(closed,1);return snapshot;
    };
    now=100;source.enqueue({type:'key'});await tick();
    now=1100;source.enqueue({type:'delta'});await tick();
    now=5100;source.enqueue({type:'delta'});await tick();
    assert.equal(requests,1);assert.equal(emitted.length,3);
    const pending=sample();assert.equal(pending.observedFrames,3);assert.equal(pending.observedKeyframes,1);
    assert.equal(pending.keyframeRequests,1);assert.equal(pending.keyframeCompletions,0);
    assert.equal(pending.keyframeRequestPending,true);assert.equal(pending.pendingRequestSeconds,4);
    resolveKey();await tick();
    const completed=sample();assert.equal(completed.keyframeCompletions,1);
    assert.equal(completed.observedKeyframes,1);assert.equal(completed.keyframeRequestPending,false);
    now=6000;source.enqueue({type:'delta'});await tick();
    rejectKey(Error('controlled refusal'));await tick();
    assert.equal(sample().keyframeFailures,1);
    assert.equal(JSON.stringify(failures),JSON.stringify([{state:'keyframe-unavailable',errorName:'Error'}]));
    assert.equal(emitted.length,4);source.close();await tick();
});
