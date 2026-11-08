import test from 'node:test';
import assert from 'node:assert/strict';
import {closeOwnedChromium} from '../lib/browser_owned_chromium.mjs';
function fixture(send=async()=>{}) {
    const calls=[];
    return {calls,browser:{async newBrowserCDPSession(){calls.push('session');return {async send(method){calls.push(method);return send();}};},async close(){calls.push('disconnect');}},
        context:{async close(){calls.push('context');}},assets:{close(callback){calls.push('assets');callback();}}};
}
test('owned Chromium cleanup requests actual shutdown before disconnect and joins local assets',async()=>{
    const f=fixture(),result=await closeOwnedChromium(f);
    assert.deepEqual(f.calls,['session','Browser.close','context','disconnect','assets']);
    assert.deepEqual(result.browserClose,{requested:true,acknowledged:true,error:null});assert.deepEqual(result.failures,[]);
});
test('owned Chromium failure cleanup still closes every original resource without replacing the primary failure',async()=>{
    const f=fixture(),primary=Error('original media failure');let observed;
    try{try{throw primary;}finally{await closeOwnedChromium(f);}}catch(error){observed=error;}
    assert.equal(observed,primary);assert.ok(f.calls.includes('Browser.close'));assert.ok(f.calls.includes('disconnect'));assert.ok(f.calls.includes('assets'));
});
test('owned Chromium expected shutdown reply loss remains metadata and preserves actual process proof',async()=>{
    const error=Error('Target page, context or browser has been closed');error.name='TargetClosedError';
    const f=fixture(async()=>{throw error;});
    f.context.close=async()=>{f.calls.push('context');throw Error('browserContext.close: Target page, context or browser has been closed');};
    f.browser.close=async()=>{f.calls.push('disconnect');throw Error('browser.close: Target page, context or browser has been closed');};
    const result=await closeOwnedChromium(f);
    assert.deepEqual(result.browserClose,{requested:true,acknowledged:false,error:{name:'TargetClosedError'}});
    assert.deepEqual(result.failures,[]);assert.equal(f.calls.filter(c=>c==='Browser.close').length,1);assert.ok(f.calls.includes('assets'));
});
test('owned Chromium missing session or failed context still attempts disconnect and asset closure',async()=>{
    const f=fixture();f.browser.newBrowserCDPSession=async()=>{f.calls.push('session');throw Error('session unavailable');};
    f.context.close=async()=>{f.calls.push('context');throw Error('real context cleanup failure');};
    const result=await closeOwnedChromium(f);
    assert.equal(result.browserClose.requested,false);assert.deepEqual(result.failures,[{phase:'context-close',name:'Error'}]);
    assert.deepEqual(f.calls,['session','context','disconnect','assets']);
});
test('owned Chromium stalled shutdown uses one bounded cutoff and still attempts remaining resources',async()=>{
    const f=fixture(()=>new Promise(()=>{})),started=performance.now(),result=await closeOwnedChromium({...f,timeoutMs:15});
    assert.ok(performance.now()-started<500);assert.equal(result.browserClose.error.name,'CleanupTimeoutError');
    assert.deepEqual(f.calls,['session','Browser.close','context','disconnect','assets']);
    assert.equal(f.calls.filter(c=>c==='Browser.close').length,1);
});
