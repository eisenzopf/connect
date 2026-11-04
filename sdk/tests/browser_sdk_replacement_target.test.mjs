import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient,BrowserMediaPeer} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';
const reference={operationId:'original',command:'connection.replace',scope:{conversationId:'room',sessionId:'session',connectionId:'source'},
    continuation:{kind:'replacement',retained_connection_id:'remote',target:{kind:'endpoint',id:'endpoint'}}};
// Signed current-root recovery plus an intercepted physical-preparation boundary.
// The low-level state machine is exercised separately; neither test is live media.
async function scenario(run,{path='webtransport',mode}={}) {
    const previous=BrowserMediaPeer.prototype.beginReplacementConnection;
    let captured,finish,complete=false,reads=0;
    BrowserMediaPeer.prototype.beginReplacementConnection=function(input,root,options){
        captured={input,root,options};
        return {prepared:Promise.resolve({id:input.connectionId,onClose(){return()=>{};}}),
            completed:new Promise(resolve=>{finish=()=>{complete=true;resolve({generation:2,recordId:'record',streamRevision:6});};}),submissionAttempted:true};
    };
    try{await fixture(async f=>{
        const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,
            endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},
            preferredTransport:path,allowFallback:false,timeoutMs:500,resolveEnrollment:async()=>f.options});
        if(path==='webtransport')f.transports.at(-1).datagrams.maxDatagramSize=1200;
        try{await run({client,f,session:client.conversations.byId('room').sessions.byId('session'),
            captured:()=>captured,finish:()=>finish?.()});}
        finally{finish?.();await client.close({timeoutMs:500});}
    },{wtMode:path==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
        if(frame.type!=='operation.get')return false;reads++;
        const readiness={stream_revision:6,streams:['audio','out','in'],participant_revision:2};
        const value={operation_id:'original',record_id:'record',command_type:'connection.replace',expected_revision:7,
            revision:reads,state:complete?'completed':'preparing',native_replacement_prepared:mode==='bad-flag'?'true':reads>=2};
        if(complete)value.result={activation_state:'ready',connection_id:'replacement_record',replaces:'source',generation:2,
            participant_id:'participant',membership_epoch:1,endpoint_id:'endpoint',device_id:'wss_device',
            retained_connection_ids:['remote'],admitted_connection_ids:['replacement_record'],retired_connection_ids:['source'],
            session_revision:8,conversation_revision:null,readiness,physical_readiness:readiness};
        await peer.emit('operation.updated',value,frame.id,{...frame,connid:mode==='foreign-scope'?'foreign':frame.connid});
        await peer.emit('ack',{},frame.id,frame);return true;
    }});}finally{BrowserMediaPeer.prototype.beginReplacementConnection=previous;}
}
test('typed target polls the current original preparation, binds its own Device and exposes capture before completion',()=>scenario(async f=>{
    const input={operation:structuredClone(reference),audio:'audio',video:{send:'out',receive:'in'}};
    const pending=f.session.connections.prepareReplacement(input,{timeoutMs:500});
    input.audio='mutated';input.video.send='mutated';input.operation.operationId='mutated';
    const target=await pending;assert.equal(target.connection.id,'replacement_record');
    assert.equal(f.captured().root.operationId,'original');assert.equal(f.captured().root.deviceId,'wss_device');
    assert.equal(f.captured().root.endpointId,'endpoint');assert.equal(f.captured().input.audio,'audio');assert.equal(f.captured().input.video,'out');
    assert.equal(f.captured().input.receiveVideo,'in');assert.equal(f.captured().root.recordId,'record');
    let completed=false;void target.completion.then(()=>{completed=true;});await Promise.resolve();assert.equal(completed,false);
    f.finish();const receipt=await target.completion;assert.equal(receipt.value.connection.id,target.connection.id);
    assert.equal(receipt.value.binding.generation,2);assert.equal(receipt.snapshot.key.operationId,'original');
    assert.equal(f.f.sent.filter(frame=>frame.type==='connection.replace'||frame.type==='connection.offer').length,0);
    assert.ok(f.f.sent.filter(frame=>frame.type==='operation.get').every(frame=>frame.connid==='source'));
}));
for(const mode of ['bad-flag','foreign-scope'])test(`typed replacement ${mode} cannot allocate target media`,()=>scenario(async f=>{
    await assert.rejects(()=>f.session.connections.prepareReplacement({operation:reference,audio:'audio'}, {timeoutMs:200}),{kind:'protocol'});
    assert.equal(f.captured(),undefined);
},{mode}));
test('typed replacement explicitly refuses the unimplemented WebRTC target path before recovery or allocation',()=>scenario(async f=>{
    const before=f.f.sent.length;
    await assert.rejects(()=>f.session.connections.prepareReplacement({operation:reference,audio:'audio'},{timeoutMs:200}),
        {kind:'unsupported',certainty:'not-submitted'});
    assert.equal(f.f.sent.length,before);assert.equal(f.captured(),undefined);
},{path:'wss-webrtc'}));
