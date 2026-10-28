import assert from 'node:assert/strict';
import {test} from 'node:test';
import {canonicalMediaClient,readCanonicalMedia} from '../../examples/conversation-client/canonical-media.js';
import {BrowserClient} from '../typescript/dist/browser.js';
for(const algorithm of ['EdDSA','ES256'])test(`${algorithm} example imports enrolled private/public keys and keeps original receive intent`,async()=>{
    const pair=await crypto.subtle.generateKey(algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'},true,['sign','verify']);
    const configuration={url:'wss://fixture.invalid/connect',credential:'synthetic-only',identityId:'identity',device:{id:'device',kind:'browser',platform:'fixture'},
        signing:{algorithm,keyId:'enrolled-client',jwk:await crypto.subtle.exportKey('jwk',pair.privateKey)},
        server:{algorithm,keyId:'enrolled-server',jwk:await crypto.subtle.exportKey('jwk',pair.publicKey),notBefore:1,expiresAt:4000000000},
        connection:{conversationId:'conversation',sessionId:'session',connectionId:'connection',operationId:'offer',participantId:'participant',audioStreamId:'audio',expectedRevision:0,inputStream:'ignored-json-member'},
        subscribe:{operationId:'original-select',expectedRevision:1,subscriptions:[{from_participant:'other',kinds:['audio']}]},end:{operationId:'original-end',expectedRevision:2}};
    const original=BrowserClient.open;let enrolled,close;const commands=[];
    BrowserClient.open=async options=>{enrolled=options;return {
        request:async(...args)=>{commands.push(args);},operationStatus:async()=>({state:'completed'}),
        onClose(listener){close=listener;},
    };};
    try {
        const media=await canonicalMediaClient(configuration,{conversationId:'conversation',sessionId:'session',identityId:'identity'});
        assert.equal(enrolled.transport,'websocket');assert.equal(enrolled.signing.key.type,'private');assert.equal(enrolled.server.key.type,'public');
        assert.equal(enrolled.signing.key.extractable,false);assert.equal(enrolled.server.key.extractable,false);assert.equal(media.options.inputStream,undefined);
        configuration.subscribe.operationId='replacement';media.options.conversationId='retarget';
        const first=media.receive();assert.equal(first,media.receive());await first;
        assert.equal(commands.length,1);assert.equal(commands[0][0],'stream.subscribe');assert.equal(commands[0][2].operation_id,'original-select');assert.equal(commands[0][2].cid,'conversation');
        await assert.rejects(canonicalMediaClient(configuration,{sessionId:'foreign'}),/configuration for this identity/);
    } finally {close?.();BrowserClient.open=original;}
});
test('example requires a bounded explicitly selected private configuration file',async()=>{
    await assert.rejects(readCanonicalMedia(undefined,{}),/configuration file/);
    await assert.rejects(readCanonicalMedia({size:65537,text(){throw Error('must not read');}},{}),/configuration file/);
});
