// Real BrowserClient signing/continuation/response boundary with controlled
// physical control transport. No TLS/native/Core/media qualification.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {canonicalJson} from '../typescript/dist/uctp_json.js';
import {fixture} from './fixtures/browser_control_peer.mjs';

const scope=()=>({cid:'room',sid:'session',connid:'target',operation_id:'move',expected_revision:0});
const offer=()=>({by_participant:'participant',substrate:'websocket+webrtc',
    substrate_setup:{kind:'websocket+webrtc',sdp:'v=0\r\ns=original frozen offer\r\n'},capabilities:{},
    streams_offered:[{id:'audio',kind:'audio',direction:'sendrecv',codec_preferences:['opus']},
        {id:'camera-a',kind:'video',direction:'sendrecv',codec_preferences:['vp8']},
        {id:'public-b',kind:'video',direction:'recvonly',codec_preferences:['vp8']}],
    webrtc_video_receive_reservation:{profile:'connect/webrtc-public-video-reservation/1',phase:'intent',reservation_id:'reservation',nonce:'reservation-nonce',preview_stream_id:'camera-a',preview_role:'receive',public_stream_id:'public-b'},
    replacement_preparation:{profile:'connect-replacement-public-video-ready-v1',operation_record_id:'record',
        public_video_readiness:{reservation_id:'reservation',source_cut_id:'original-cut',nonce:'public-nonce',
            binding:{stream_id:'public-b',preview_stream_id:'camera-a',mid:'2',stream_generation:1,physical_writer_incarnation:'original-writer',connection_generation:3,primary_ssrc:0,payload_type:96,clock_rate_hz:90000,codec:'vp8'},
            decoded_frames:2,decoded_keyframes:null,decoder_startup:'fresh-receiver-first-decoded-frame'}}});

async function boundary(run,{algorithm='EdDSA',transport='websocket'}={}){
    await fixture(async f=>{
        const client=await BrowserClient.open({...f.options,transport,url:transport==='websocket'?f.options.url:'https://fixture.invalid/connect'});
        let failure;
        try{await run({client,f});}catch(error){failure=error;}
        finally{await client.shutdown();}
        assert.equal(client.closed,true);
        assert(transport==='websocket'?f.sockets.every(socket=>socket.readyState===3&&socket.closeCount===1):f.transports.every(peer=>peer.didClose),
            'Every original control owner closes before behavior outcome');
        if(failure)throw failure;
    },{algorithm,wtMode:transport==='webtransport'?'ok':undefined,handler:async({peer,frame})=>{
        if(frame.type!=='connection.offer')return false;
        // This models the authoritative original Root check, not a client
        // privilege: a well-formed foreign opaque record never grants success.
        if(frame.payload.replacement_preparation.operation_record_id!=='record'){
            await peer.emit('error',{code:403,category:'authorization',reason:'forbidden'},frame.id,frame);return true;
        }
        await peer.emit('operation.updated',{operation_id:'move',record_id:'record',command_type:'connection.replace',revision:3,state:'preparing'},frame.id,frame);
        await peer.emit('ack',{},frame.id,frame);return true;
    }});
}

test('real signed BrowserClient original continuation admits exact public B ready marker and freezes the original offer before write',async()=>{
    for(const algorithm of ['EdDSA','ES256'])await boundary(async({client,f})=>{
        const payload=offer(),original=structuredClone(payload),binding=payload.replacement_preparation.public_video_readiness.binding;
        const response=client.requestReplacementContinuation('connection.offer',payload,scope(),{timeoutMs:200});
        binding.mid='caller-mutated-after-intention';
        const frames=await response;assert.equal(frames.find(frame=>frame.type==='operation.updated').payload.record_id,'record');
        const sent=f.sent.filter(frame=>frame.type==='connection.offer');assert.equal(sent.length,1);
        assert.equal(canonicalJson(sent[0].payload),canonicalJson(original));assert.equal(sent[0].operation_id,'move');assert.equal(sent[0].connid,'target');
        assert.equal(sent[0].expected_revision,0);assert.equal(sent[0].payload.replacement_preparation.public_video_readiness.binding.primary_ssrc,0);
        assert.equal(sent[0].payload.replacement_preparation.public_video_readiness.decoded_keyframes,null);
        assert.equal(client.closed,false);
    },{algorithm});
});

test('real BrowserClient public B continuation refuses malformed mixed foreign-role and wrong-terminal markers without broadening the original gate',async()=>{
    await boundary(async({client,f})=>{
        const changes=[
            p=>{p.replacement_preparation.readiness={nonce:'private'};},
            p=>{p.replacement_preparation.public_video_readiness.reservation_id='foreign';},
            p=>{p.replacement_preparation.public_video_readiness.source_cut_id='';},
            p=>{p.replacement_preparation.public_video_readiness.nonce='reservation-nonce';},
            p=>{p.replacement_preparation.public_video_readiness.decoded_frames=0;},
            p=>{delete p.replacement_preparation.public_video_readiness.decoded_keyframes;},
            p=>{p.replacement_preparation.public_video_readiness.decoded_keyframes=3;},
            p=>{p.replacement_preparation.public_video_readiness.decoded_keyframes=0;},
            p=>{p.replacement_preparation.public_video_readiness.decoder_startup='private-peer';},
            p=>{p.replacement_preparation.public_video_readiness.permission=true;},
            p=>{p.replacement_preparation.public_video_readiness.binding.stream_id='camera-a';},
            p=>{p.replacement_preparation.public_video_readiness.binding.preview_stream_id='foreign';},
            p=>{p.replacement_preparation.public_video_readiness.binding.mid='bad mid';},
            p=>{p.replacement_preparation.public_video_readiness.binding.primary_ssrc=0x100000000;},
            p=>{p.replacement_preparation.public_video_readiness.binding.payload_type=128;},
            p=>{p.replacement_preparation.public_video_readiness.binding.connection_generation=0;},
            p=>{p.replacement_preparation.public_video_readiness.binding.stream_generation=Number.MAX_SAFE_INTEGER+1;},
            p=>{p.replacement_preparation.public_video_readiness.binding.physical_writer_incarnation='';},
            p=>{p.replacement_preparation.public_video_readiness.binding.clock_rate_hz=48000;},
            p=>{p.replacement_preparation.public_video_readiness.binding.codec='h264';},
            p=>{p.webrtc_video_receive_reservation.phase='cut-bound-post-f';},
            p=>{p.webrtc_video_receive_reservation.preview_role='recvonly';},
            p=>{p.streams_offered[2].direction='sendrecv';},
            p=>{p.streams_offered[0].id='public-b';},
            p=>{p.handoff_preparation={profile:'connect-handoff-preparation-v1'};},
        ];
        for(const change of changes){const payload=offer();change(payload);assert.throws(()=>client.requestReplacementContinuation('connection.offer',payload,scope(),{timeoutMs:200}),{code:'invalid-request'});}
        for(const changed of [{...scope(),expected_revision:1},{...scope(),cid:''},{...scope(),operation_id:''}])
            assert.throws(()=>client.requestReplacementContinuation('connection.offer',offer(),changed,{timeoutMs:200}),{code:'invalid-request'});
        assert.throws(()=>client.requestReplacementContinuation('connection.offer',offer(),scope(),{timeoutMs:200,terminalType:'connection.answer'}),{code:'invalid-request'});
        assert.throws(()=>client.requestReplacementContinuation('connection.ready',offer(),scope(),{timeoutMs:200}),{code:'invalid-request'});
        let getterReads=0;const accessor=offer();Object.defineProperty(accessor.replacement_preparation.public_video_readiness.binding,'codec',{get(){getterReads++;return'vp8';},enumerable:true});
        assert.throws(()=>client.requestReplacementContinuation('connection.offer',accessor,scope(),{timeoutMs:200}),{code:'invalid-request'});assert.equal(getterReads,0);
        assert.equal(f.sent.filter(frame=>frame.type==='connection.offer').length,0);assert.equal(client.closed,false);
        const foreign=offer();foreign.replacement_preparation.operation_record_id='foreign-record';
        await assert.rejects(client.requestReplacementContinuation('connection.offer',foreign,scope(),{timeoutMs:200}),error=>error.code==='refused'&&error.problem?.code===403);
        assert.equal(f.sent.filter(frame=>frame.type==='connection.offer').length,1);assert.equal(client.closed,false);
    });
    await boundary(async({client,f})=>{
        assert.throws(()=>client.requestReplacementContinuation('connection.offer',offer(),scope(),{timeoutMs:200}),{code:'invalid-request'});
        assert.equal(f.sent.filter(frame=>frame.type==='connection.offer').length,0);
    },{transport:'webtransport'});
});


const bridgeOffer=()=>{
    const p=offer(),r=p.replacement_preparation.public_video_readiness;
    p.webrtc_public_video_bridge_opt_in={profile:'connect/webrtc-public-video-precut-bridge/1',carrier:'webrtc-rtp',presentation:'exclusive-public',feed:'public-programme-private-decode'};
    p.replacement_preparation={profile:'connect-replacement-public-video-bridge-ready-v1',operation_record_id:'record',bridge_readiness:{reservation_id:r.reservation_id,nonce:'bridge-nonce',binding:{...r.binding},decoded_rtp_timestamp:0,decoded_frames:1,decoder_startup:'fresh-b-receiver-precut-keyframe'}};return p;
};
test('real signed BrowserClient bridge Ready carries exact original opt-in and first RTP zero without mixed legacy readiness',async()=>{
    for(const algorithm of ['EdDSA','ES256'])await boundary(async({client,f})=>{
        const body=bridgeOffer(),frozen=structuredClone(body),pending=client.requestReplacementContinuation('connection.offer',body,scope(),{timeoutMs:200});
        body.replacement_preparation.bridge_readiness.decoded_rtp_timestamp=88;await pending;
        const sent=f.sent.filter(v=>v.type==='connection.offer');assert.equal(sent.length,1);assert.equal(canonicalJson(sent[0].payload),canonicalJson(frozen));
        assert.equal(sent[0].payload.replacement_preparation.bridge_readiness.decoded_rtp_timestamp,0);assert.equal(sent[0].operation_id,'move');assert.equal(client.closed,false);
    },{algorithm});
});
test('real signed BrowserClient bridge boundary rejects absent opt-in mixed cut invented counters roles and alternate transport',async()=>{
    await boundary(async({client,f})=>{
        for(const change of [p=>{delete p.webrtc_public_video_bridge_opt_in;},p=>{p.webrtc_public_video_bridge_opt_in.feed='private-preview';},p=>{p.replacement_preparation.public_video_readiness={};},p=>{p.replacement_preparation.bridge_readiness.source_cut_id='fake';},p=>{p.replacement_preparation.bridge_readiness.decoded_rtp_timestamp=-1;},p=>{p.replacement_preparation.bridge_readiness.decoded_rtp_timestamp=0x100000000;},p=>{p.replacement_preparation.bridge_readiness.decoded_frames=0;},p=>{p.replacement_preparation.bridge_readiness.binding.stream_id='camera-a';},p=>{p.replacement_preparation.bridge_readiness.decoder_startup='fresh-peer-first-decoded-frame';},p=>{p.streams_offered[2].direction='sendrecv';}]) {
            const body=bridgeOffer();change(body);assert.throws(()=>client.requestReplacementContinuation('connection.offer',body,scope(),{timeoutMs:200}),{code:'invalid-request'});
        }
        assert.equal(f.sent.filter(v=>v.type==='connection.offer').length,0);
        const body=bridgeOffer();body.replacement_preparation.operation_record_id='foreign-record';await assert.rejects(client.requestReplacementContinuation('connection.offer',body,scope(),{timeoutMs:200}),e=>e.code==='refused'&&e.problem?.code===403);
    });
    await boundary(async({client,f})=>{assert.throws(()=>client.requestReplacementContinuation('connection.offer',bridgeOffer(),scope(),{timeoutMs:200}),{code:'invalid-request'});assert.equal(f.sent.filter(v=>v.type==='connection.offer').length,0);},{transport:'webtransport'});
});
