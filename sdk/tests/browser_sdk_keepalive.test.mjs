import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';

// Signed control fixtures only; not a live browser, TLS or media qualification.
function scenario(path, run, handler) {
    return fixture(async f => {
        let resolutions = 0;
        const client = await BrowserSdkClient.connect({tenantId:'tnt_default',
            identityId:f.options.identityId, deviceId:f.options.device.id,
            endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},
            preferredTransport:path, allowFallback:false, timeoutMs:500,
            resolveEnrollment:async()=>{resolutions++;return f.options;}});
        try {await run({client,f,resolutions:()=>resolutions});}
        finally {await client.close({timeoutMs:500});}
    }, {wtMode:path==='webtransport'?'ok':undefined,handler});
}
for (const path of ['webtransport','wss-webrtc']) {
    test(`browser ${path} keepalive uses current signed session without reenrollment or media mutation`,()=>scenario(path,async({client,f,resolutions})=>{
        const before=f.sent.length,epoch=client.transport.physicalEpoch;
        await client.keepAlive({timeoutMs:300});
        const requests=f.sent.slice(before);
        assert.equal(requests.length,1);assert.equal(requests[0].type,'auth.keepalive');
        assert.deepEqual({...requests[0].payload},{session_token:'fixture-session'});
        assert.ok(requests[0].signature);assert.equal(requests[0].operation_id,undefined);
        assert.equal(client.transport.physicalEpoch,epoch);assert.equal(resolutions(),1);
        // Successful reachability renewal does not refresh the original auth expiry.
        const now=Date.now;Date.now=()=>now()+120000;
        try {await assert.rejects(()=>client.keepAlive({timeoutMs:300}),{kind:'authentication',certainty:'read-only'});}
        finally {Date.now=now;}
        assert.equal(f.sent.length,before+1);
    }));
    test(`browser ${path} keepalive validates and cancels before submitting`,()=>scenario(path,async({client,f})=>{
        const before=f.sent.length;
        for(const options of [undefined,{timeoutMs:0},{timeoutMs:60001},{timeoutMs:NaN},
            {timeoutMs:300,signal:{}},{timeoutMs:300,operationId:'not-an-operation'}])
            await assert.rejects(()=>client.keepAlive(options),{kind:'validation'});
        const abort=new AbortController();abort.abort();
        await assert.rejects(()=>client.keepAlive({timeoutMs:300,signal:abort.signal}),{kind:'cancelled'});
        assert.equal(f.sent.length,before);assert.equal(client.transport.state,'connected');
    }));
    test(`browser ${path} keepalive preserves a current authorization refusal without retry`,()=>scenario(path,async({client,f,resolutions})=>{
        f.setCredential('revoked');
        await assert.rejects(()=>client.keepAlive({timeoutMs:300}),{kind:'authentication',certainty:'refused'});
        assert.equal(f.sent.filter(frame=>frame.type==='auth.keepalive').length,1);
        assert.equal(resolutions(),1);assert.equal(f.sent.filter(frame=>frame.type==='auth.response').length,1);
    }));
    test(`browser ${path} keepalive timeout retires original physical owner without retry`,()=>scenario(path,async({client,f,resolutions})=>{
        await assert.rejects(()=>client.keepAlive({timeoutMs:40}),{kind:'deadline'});
        assert.equal(f.sent.filter(frame=>frame.type==='auth.keepalive').length,1);
        assert.equal(resolutions(),1);assert.notEqual(client.transport.state,'connected');
    },async({frame})=>frame.type==='auth.keepalive'));
    test(`browser ${path} close interrupts a pending keepalive and joins the original owner`,()=>{
        let received;const seen=new Promise(resolve=>{received=resolve;});
        return scenario(path,async({client,f,resolutions})=>{
            const pending=client.keepAlive({timeoutMs:500});
            const rejected=assert.rejects(()=>pending);await seen;
            await client.close({timeoutMs:300});await rejected;
            assert.equal(f.sent.filter(frame=>frame.type==='auth.keepalive').length,1);
            assert.equal(resolutions(),1);
            await assert.rejects(()=>client.keepAlive({timeoutMs:300}));
            assert.equal(f.sent.filter(frame=>frame.type==='auth.keepalive').length,1);
        },async({frame})=>{if(frame.type!=='auth.keepalive')return false;received();return true;});
    });
}
