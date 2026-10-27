import assert from 'node:assert/strict';
import {test} from 'node:test';
import {microphoneStream} from '../typescript/dist/browser_capture.js';

async function fixture(run){
    const descriptor=Object.getOwnPropertyDescriptor(globalThis,'navigator');
    let resolve,reject;
    const state={requests:0,stops:[0,0],pending:new Promise((yes,no)=>{resolve=yes;reject=no;})};
    state.stream={getTracks:()=>state.stops.map((_,i)=>({stop(){state.stops[i]++;}}))};
    state.resolve=()=>resolve(state.stream);state.reject=reject;
    Object.defineProperty(globalThis,'navigator',{configurable:true,value:{mediaDevices:{getUserMedia(){state.requests++;return state.pending;}}}});
    try{await run(state);}finally{if(descriptor)Object.defineProperty(globalThis,'navigator',descriptor);else delete globalThis.navigator;}
}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
test('cancelled capture never opens a permission request',()=>fixture(async f=>{
    const abort=new AbortController();abort.abort();
    await assert.rejects(microphoneStream(500,true,{},abort.signal),{code:'cancelled'});
    await assert.rejects(microphoneStream(0.5,true,{},abort.signal),{code:'cancelled'});
    assert.equal(f.requests,0);assert.deepEqual(f.stops,[0,0]);
}));
test('control retirement cancels pending capture and stops every late track once',()=>fixture(async f=>{
    const abort=new AbortController(),wait=microphoneStream(500,true,{},abort.signal);
    abort.abort();await assert.rejects(wait,{code:'cancelled'});assert.equal(f.requests,1);
    f.resolve();await tick();assert.deepEqual(f.stops,[1,1]);
    abort.abort();assert.deepEqual(f.stops,[1,1]);
}));
test('capture deadline owns and stops permission results arriving after timeout',()=>fixture(async f=>{
    const wait=microphoneStream(5,true);await assert.rejects(wait,{code:'timeout'});
    f.resolve();await tick();assert.deepEqual(f.stops,[1,1]);
}));
test('successful capture transfers ownership and removes its cancellation callback',()=>fixture(async f=>{
    const abort=new AbortController(),wait=microphoneStream(500,true,{},abort.signal);
    f.resolve();assert.equal(await wait,f.stream);abort.abort();await tick();
    assert.deepEqual(f.stops,[0,0]);
}));
test('late permission denial after cancellation is handled without changing the original result',()=>fixture(async f=>{
    const abort=new AbortController(),wait=microphoneStream(500,true,{},abort.signal);
    abort.abort();await assert.rejects(wait,{code:'cancelled'});f.reject(Error('late denial'));await tick();
    assert.deepEqual(f.stops,[0,0]);
}));
test('capture rejects invalid deadlines before any permission prompt',()=>fixture(async f=>{
    for(const timeout of [0,-1,NaN,Infinity,60001])await assert.rejects(microphoneStream(timeout,false),{code:'invalid-request'});
    assert.equal(f.requests,0);
}));
