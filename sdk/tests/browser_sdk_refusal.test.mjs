import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';

// Signed current protocol refusals, not HTTP status guesses or transport faults.
for (const path of ['webtransport','wss-webrtc']) {
    for (const problem of [
        {code:501,category:'unsupported',reason:'unsupported_feature'},
        {code:503,category:'service',reason:'temporarily_unavailable'},
        {code:501,category:'service',reason:'unknown_failure'},
    ]) test(`signed browser ${path} preserves ${problem.code}/${problem.reason} submission certainty`, async()=>{
        await fixture(async f=>{
            const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,
                deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},
                preferredTransport:path,allowFallback:false,timeoutMs:500,resolveEnrollment:async()=>f.options});
            try {
                const key={operationId:'original-upgrade',command:'session.update',scope:{conversationId:'room',sessionId:'session'}};
                await assert.rejects(()=>client.conversations.byId('room').sessions.byId('session').update(
                    {action:'medium-upgrade',media:{first:'screen',rest:[]}},1,{timeoutMs:300,operationId:key.operationId}),error=>{
                    assert.equal(error.kind,problem.reason==='unsupported_feature'?'unsupported':'application');
                    assert.equal(error.certainty,problem.reason==='unsupported_feature'?'refused':'uncertain');
                    assert.deepEqual(error.problem,problem);assert.deepEqual(error.operation,key);return true;
                });
                assert.equal(f.sent.filter(frame=>frame.type==='session.update').length,1);
                assert.equal(f.sent.filter(frame=>frame.type==='operation.get').length,0);
            } finally {await client.close({timeoutMs:500});}
        },{wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
            if(frame.type!=='session.update')return false;
            await peer.emit('error',problem,frame.id,frame);return true;
        }});
    });
}
