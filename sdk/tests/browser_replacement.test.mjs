import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaPeer} from '../typescript/dist/browser_media.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';

// Control/codec ownership fixture only; this does not qualify live browser media.
const root=()=>({recordId:'record',operationId:'original-move',conversationId:'room',sessionId:'session',
    sourceConnectionId:'old',retainedConnectionId:'remote',targetConnectionId:'replacement_record',endpointId:'endpoint',deviceId:'device'});
const input=()=>({conversationId:'room',sessionId:'session',connectionId:'replacement_record',operationId:'original-move',
    audio:'audio',video:'camera',receiveVideo:'incoming-camera'});
async function fixture(run,{mode}={}) {
    const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk','VideoEncoder','VideoDecoder','VideoFrame'];
    const originals=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
    class Codec {
        static async isConfigSupported(){return {supported:true};}
        state='unconfigured';encodeQueueSize=0;decodeQueueSize=0;
        constructor(callbacks){this.callbacks=callbacks;}
        configure(){this.state='configured';}close(){this.state='closed';}
        encode(frame){queueMicrotask(()=>this.callbacks.output({timestamp:frame.timestamp,type:'key',byteLength:1,copyTo:bytes=>bytes.set([9])}));}
    }
    class Frame {constructor(value){Object.assign(this,value);}close(){}}
    for(const name of names)Object.defineProperty(globalThis,name,{configurable:true,value:name.endsWith('Encoder')||name.endsWith('Decoder')?Codec:Frame});
    const sent=[],events=new Set(),lifetimes=new Set();let complete=false,reads=0,parameters;
    const status=()=>({record_id:'record',operation_id:'original-move',command_type:'connection.replace',expected_revision:7,
        state:complete?'completed':'preparing',revision:complete?8:3,native_replacement_prepared:mode!=='not-prepared',
        ...(complete?{result:{activation_state:mode==='stopped'?'stopped':'ready',connection_id:'replacement_record',replaces:'old',
            endpoint_id:'endpoint',device_id:'device',generation:2,retained_connection_ids:['remote'],admitted_connection_ids:['replacement_record'],
            retired_connection_ids:['old'],readiness:{stream_revision:6,streams:['audio','camera','incoming-camera']},
            physical_readiness:{stream_revision:6,streams:['audio','camera','incoming-camera']}}}:{})});
    const client={closed:false,
        datagrams:{maxDatagramSize:1200,readable:new ReadableStream(),writable:new WritableStream({write:()=>{if(mode!=='hold')complete=true;}})},
        onEvent:fn=>{events.add(fn);return()=>events.delete(fn);},onClose:fn=>{lifetimes.add(fn);return()=>lifetimes.delete(fn);},
        close(){if(this.closed)return;this.closed=true;for(const fn of [...lifetimes])fn(Error('closed'));},
        async request(type,payload,scope){
            sent.push({type,payload:structuredClone(payload),scope:{...scope}});
            if(type==='operation.get'){reads++;const value=status();if(mode==='changed-root'&&reads>1)value.record_id='foreign';
                return [envelope('operation.updated',value,scope),envelope('ack',{},scope)];}
            assert.equal(type,'connection.offer');assert.equal(payload.replacement_preparation.profile,'connect-replacement-preparation-v1');
            if(mode==='lost-allocation')throw new ConnectError('timeout','allocation response lost');
            return [envelope('operation.updated',status(),scope),envelope('ack',{},scope)];
        },
        begin(type,payload,scope){
            sent.push({type,payload:structuredClone(payload),scope:{...scope}});const id='media-request';
            const streams=payload.streams_offered.map((stream,index)=>({strm_id:stream.id,kind:stream.kind,
                direction:mode==='wrong-direction'&&stream.id==='incoming-camera'?'recvonly':
                    stream.direction==='sendonly'?'recvonly':stream.direction==='recvonly'?'sendonly':'sendrecv',
                stream_local_id:index+1,codec:{name:stream.kind==='audio'?'opus':'vp8',params:{sample_rate:stream.kind==='audio'?48000:90000,
                    channels:stream.kind==='audio'?1:0,payload_type:stream.kind==='audio'?111:96}}}));
            parameters={by_participant:'participant',substrate:'webtransport',
                substrate_setup:{kind:'native-prepared/1',revision:1,offer_id:id,streams}};
            queueMicrotask(()=>{for(const fn of events)fn(envelope('connection.offer',parameters,scope));});
            return {id,response:Promise.resolve([envelope('connection.answer',{}, {...scope,in_reply_to:id})])};
        },
        async send(type,payload,scope){sent.push({type,payload:structuredClone(payload),scope:{...scope}});},
        async operationStatus(_cid,operation){assert.equal(operation,'server-reconcile');return {operation_id:operation,state:'completed'};},
    };
    const peer=new BrowserMediaPeer(client);
    try{await run({peer,client,sent,finish:()=>{complete=true;},reconcile:()=>{
        const payload=structuredClone(parameters);payload.substrate_setup.revision=7;payload.substrate_setup.offer_id='server-request';
        for(const fn of events)fn(envelope('connection.offer',payload,{cid:'room',sid:'session',connid:'replacement_record',operation_id:'server-reconcile'}));
    }});}
    finally{await peer.shutdown();for(const[name,descriptor]of originals)if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
}
test('replacement exposes negotiated target IO before original-root completion and sends each phase once',()=>fixture(async({peer,sent,reconcile})=>{
    const original=root(),offer=input(),begun=peer.beginReplacementConnection(offer,original,{timeoutMs:500});
    original.recordId='changed';offer.audio='changed';let finished=false;void begun.completed.then(()=>{finished=true;});
    const prepared=await begun.prepared;assert.equal(finished,false);assert.equal(prepared.revision,1);
    assert.deepEqual(prepared.streams.map(stream=>[stream.streamId,stream.direction]),[['audio','sendrecv'],['camera','sendonly'],['incoming-camera','recvonly']]);
    assert.equal(prepared.encodeAudio('audio',new Float32Array(960),100000),true);
    assert.deepEqual(await begun.completed,{recordId:'record',generation:2,streamRevision:6});assert.equal(prepared.revision,6);
    const offers=sent.filter(frame=>frame.type==='connection.offer');assert.equal(offers.length,2);
    assert.deepEqual(offers.map(frame=>frame.payload.replacement_preparation.profile),['connect-replacement-preparation-v1','connect-replacement-media-v1']);
    assert.deepEqual(offers[0].scope,offers[1].scope);assert.equal(offers[0].scope.operation_id,'original-move');
    const first=structuredClone(offers[0].payload),second=structuredClone(offers[1].payload);delete first.replacement_preparation;delete second.replacement_preparation;
    assert.deepEqual(first,second);assert.equal(sent.filter(frame=>frame.type==='connection.answer').length,1);
    assert.ok(sent.filter(frame=>frame.type==='operation.get').every(frame=>frame.scope.connid==='old'));
    reconcile();const until=performance.now()+200;
    while(prepared.revision!==7){assert.ok(performance.now()<until,'transferred receivers did not reconcile');await new Promise(resolve=>setTimeout(resolve,1));}
    assert.equal(sent.filter(frame=>frame.type==='connection.answer'&&frame.scope.operation_id==='server-reconcile').length,1);
}));
for(const [mode,code,allocations]of [['not-prepared','conflict',0],['lost-allocation','timeout',1],['wrong-direction','protocol',2]])
    test(`replacement ${mode} refuses without a second mutation or leaked target`,()=>fixture(async({peer,sent})=>{
        const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:200});
        await assert.rejects(()=>begun.prepared,{code});await assert.rejects(()=>begun.completed,{code});
        assert.equal(sent.filter(frame=>frame.type==='connection.offer').length,allocations);
        assert.equal(peer.diagnostics.activeBindings,0);
    },{mode}));
for(const mode of ['changed-root','stopped','hold'])test(`prepared target retires on ${mode} without reporting committed movement`,()=>fixture(async({peer,client,sent,finish})=>{
    const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:80});
    const prepared=await begun.prepared;let closed=0;prepared.onClose(()=>closed++);
    if(mode==='stopped')finish();
    await assert.rejects(()=>begun.completed,{code:mode==='hold'?'timeout':mode==='stopped'?'refused':'protocol'});
    assert.equal(closed,1);assert.equal(peer.diagnostics.activeBindings,0);assert.equal(client.closed,mode!=='stopped');
    assert.equal(sent.filter(frame=>frame.type==='connection.offer').length,2);
    assert.equal(sent.filter(frame=>frame.type==='connection.replace'||frame.type==='connection.end').length,0);
},{mode}));
