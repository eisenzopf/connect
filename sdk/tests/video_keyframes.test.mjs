import assert from 'node:assert/strict';
import {test} from 'node:test';
import vm from 'node:vm';
import fs from 'node:fs';
import {keyframeAdapter,stopVideoWorker} from '../typescript/dist/browser_video_keyframes.js';

const tick=()=>new Promise(resolve=>setTimeout(resolve,5));
function workerHarness(adapterPort) {
    let now=0,source;const failures=[],emitted=[];
    const scope={performance:{now:()=>now},Number,Math,Promise,TransformStream,DOMException,setTimeout,clearTimeout,
        postMessage:value=>failures.push(value)};
    vm.runInNewContext(fs.readFileSync(new URL('../typescript/dist/browser_video_transform.js',import.meta.url),'utf8')
        .replace(/^export \{\};/m,''),scope);
    scope.onrtctransform({transformer:{options:{keyframePort:adapterPort},
        readable:new ReadableStream({start:controller=>{source=controller;}}),writable:new WritableStream({write:frame=>emitted.push(frame)})}});
    return {failures,emitted,frame:async(type,at)=>{now=at;source.enqueue({type});await tick();},
        snapshot:()=>{let result;scope.onmessage({data:{op:'video-diagnostics'},ports:[{postMessage:value=>{result={...value};},close(){}}]});return result;},
        close:async()=>{source.close();await tick();}};
}

test('sender option adapter preserves exact parameters and completes only on observed worker keyframe', async()=>{
    let calls=0;const params={transactionId:'private-canary',encodings:[{active:true,maxBitrate:1000000},{active:false}]};
    const ownedWorker={terminate(){}};
    const port=keyframeAdapter(ownedWorker,{getParameters:()=>params,setParameters:async(actual,options)=>{
        calls++;assert.equal(actual,params);assert.deepEqual(options,{encodingOptions:[{keyFrame:true},{keyFrame:false}]});}});
    const worker=workerHarness(port);
    try{
        await worker.frame('key',100);await worker.frame('delta',1100);await tick();
        assert.equal(calls,1);assert.equal(worker.snapshot().keyframeCompletions,0);
        assert.equal(worker.snapshot().keyframeRequestPending,true);
        await worker.frame('key',1150);await tick();
        assert.equal(worker.snapshot().keyframeCompletions,1);assert.equal(worker.snapshot().observedKeyframes,2);
        assert.equal(worker.snapshot().keyframeRequestPending,false);assert.equal(worker.failures.length,0);
        assert.equal(worker.emitted.length,3);
        assert.equal(JSON.stringify(worker.snapshot()).includes('private-canary'),false);
    }finally{await worker.close();stopVideoWorker(ownedWorker);}
});

test('ignored keyframe option cannot claim completion and adapter rejection retains a safe typed error', async()=>{
    const ownedWorker={terminate(){}};
    const port=keyframeAdapter(ownedWorker,{getParameters:()=>({encodings:[{}]}),setParameters:async()=>{}});
    const worker=workerHarness(port);
    try{
        await worker.frame('key',100);await worker.frame('delta',1100);await tick();
        assert.equal(worker.snapshot().keyframeCompletions,0);
        await new Promise(resolve=>setTimeout(resolve,2050));
        assert.equal(worker.snapshot().keyframeFailures,1);
        assert.equal(JSON.stringify(worker.failures),JSON.stringify([{state:'keyframe-unavailable',errorName:'TimeoutError'}]));
    }finally{await worker.close();stopVideoWorker(ownedWorker);}
    const refusedWorker={terminate(){}};
    const failedPort=keyframeAdapter(refusedWorker,{getParameters:()=>({encodings:[{}]}),setParameters:async()=>{
        throw new DOMException('private-canary','NotSupportedError');}});
    const failed=workerHarness(failedPort);
    try{await failed.frame('key',100);await failed.frame('delta',1100);await tick();
        assert.equal(JSON.stringify(failed.failures),JSON.stringify([{state:'keyframe-unavailable',errorName:'NotSupportedError'}]));
        assert.equal(failed.snapshot().keyframeCompletions,0);
    }finally{await failed.close();stopVideoWorker(refusedWorker);}
});

test('stopping an adapter closes owned ports and prevents a delayed sender result from recreating work',async()=>{
    let complete,stopped=false;
    const ownedWorker={terminate(){stopped=true;}};
    const port=keyframeAdapter(ownedWorker,{getParameters:()=>({encodings:[{}]}),setParameters:()=>new Promise(resolve=>{complete=resolve;})});
    let replies=0;port.onmessage=()=>replies++;
    port.postMessage({op:'keyframe',id:1});await tick();stopVideoWorker(ownedWorker);
    complete();await tick();assert.equal(replies,0);assert.equal(stopped,true);port.close();
});
