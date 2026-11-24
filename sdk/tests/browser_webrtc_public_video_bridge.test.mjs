// Source-only authored controls; metadata is not Core/native/browser proof.
import {test} from 'node:test';import assert from 'node:assert/strict';
import {WEBRTC_BRIDGE_OPT_IN,parseWebRtcBridgeOptIn,parseWebRtcPreCutKeyframes,parseWebRtcBridgeSnapshot,parseWebRtcReceiverPostCut,postCutRtpEligible} from '../typescript/dist/browser_webrtc_public_video_bridge.js';
import {ReceiverVideoReleaseOwner} from '../typescript/dist/browser_receiver_video_release.js';
import {moveIntent} from '../typescript/dist/browser_sdk_movements.js';
const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'old',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'};
const binding={stream_id:'public-b',preview_stream_id:'camera-a',mid:'2',stream_generation:4,connection_generation:7,physical_writer_incarnation:'a'.repeat(32),primary_ssrc:0,payload_type:96,clock_rate_hz:90000,codec:'vp8'};
const phase={profile:'connect/webrtc-public-video-phase/1',phase:'cut-bound-post-f',reservation_id:'reservation',operation_record_id:'record',operation_id:'move',target_connection_id:'target',target_generation:9,binding,source_cut_id:'cut',public_readiness:{nonce:'public-nonce',decoder_startup:'fresh-receiver-first-decoded-frame'}};
const witness=()=>({profile:'connect-receiver-video-post-cut-v1',operation_record_id:'record',operation_id:'move',reservation_id:'reservation',source_connection_id:'old',target_connection_id:'target',stream_id:'public-b',stream_generation:4,connection_generation:7,target_generation:9,source_cut_id:'cut',original_frame:{realm:'b'.repeat(32),ordinal:'12',last_old_ordinal:'11'},floor:{carrier:'webrtc-rtp',physical_writer_incarnation:'a'.repeat(32),primary_ssrc:0,first_public_rtp_timestamp:0xfffffff0}});
test('original move selects bridge only explicitly in signed body and preserves unopted video shape',()=>{
    const input={retain:'retained',target:{endpoint:'endpoint'},expectedTopologyRevision:1,video:{publications:[{previous:'camera',replacement:'camera-a'}],receivers:[{previous:'video',replacement:'public-b'}]}};
    const bad=()=>{throw Error('invalid move');};const normal=moveIntent(input,'old',bad);assert.equal(normal.body.receiver_video_bridge,undefined);
    const bridge=moveIntent({...input,receiverVideoBridge:'webrtc-precut-private-decode'},'old',bad);assert.deepEqual(bridge.body.receiver_video_bridge,WEBRTC_BRIDGE_OPT_IN);assert.deepEqual(bridge.body.video,normal.body.video);
    assert.throws(()=>moveIntent({...input,receiverVideoBridge:true},'old',bad));assert.throws(()=>moveIntent({...input,video:undefined,receiverVideoBridge:'webrtc-precut-private-decode'},'old',bad));
    assert.throws(()=>parseWebRtcBridgeOptIn({...WEBRTC_BRIDGE_OPT_IN,permission:true}));
});
test('provisional exact receiver floor refuses foreign identities delayed pre-F half range and clock substitutions',()=>{
    const w=parseWebRtcReceiverPostCut(witness(),root,phase);assert.equal(postCutRtpEligible(w,{rtpTimestamp:0xfffffff0,presentedFrames:1}),true);assert.equal(postCutRtpEligible(w,{rtpTimestamp:0x10,presentedFrames:2}),true);
    assert.equal(postCutRtpEligible(w,{rtpTimestamp:0xffffffef,presentedFrames:2}),false);assert.equal(postCutRtpEligible(w,{rtpTimestamp:0x7ffffff0,presentedFrames:2}),false);assert.equal(postCutRtpEligible(w,undefined),false);
    for(const change of [v=>v.target_generation=7,v=>v.connection_generation=9,v=>v.operation_record_id='foreign',v=>v.operation_id=v.operation_record_id,v=>v.source_cut_id='other',v=>v.binding={},v=>v.floor.primary_ssrc=1,v=>v.floor.physical_writer_incarnation='c'.repeat(32),v=>v.original_frame.ordinal='11',v=>v.floor.first_public_rtp_timestamp=-1]){const v=witness();change(v);assert.throws(()=>parseWebRtcReceiverPostCut(v,root,phase));}
});
test('final released receiver carries monotonic same-writer coordinates across cumulative natural wrap without repeating a public frame',()=>{
    const w=witness(),{profile,...scope}=w;delete scope.original_frame;delete scope.floor;
    const owner=new ReceiverVideoReleaseOwner(scope,{carrier:'webrtc-rtp',physical_writer_incarnation:binding.physical_writer_incarnation,primary_ssrc:0},()=>true);
    owner.observe({...w,profile:'connect-receiver-video-release-v1'},{operationRecordId:'record',operationId:'move',targetConnectionId:'target',releaseAuthorized:true});
    const loan=owner.borrow();let n=0;
    for(const ts of [0xfffffff0,0x3ffffff0,0x7ffffff0,0xbffffff0,0xfffffff0]){const output={rtpTimestamp:ts,presentedFrames:++n};assert.equal(loan.eligibleRtp(output),true);loan.presentedRtp(output);assert.equal(loan.eligibleRtp(output),false);}
    loan.release();const again=owner.borrow();assert.equal(again.eligibleRtp({rtpTimestamp:0xfffffff0,presentedFrames:n+1}),false);owner.close();assert.equal(again.eligibleRtp({rtpTimestamp:0x20,presentedFrames:n+2}),false);
});


test('retained pre-cut keyframe coordinates use exact bounded ordered u32 including natural RTP wrap',()=>{
    assert.equal(parseWebRtcPreCutKeyframes(undefined),undefined);
    const raw=[0xfffffff0,0,3000],parsed=parseWebRtcPreCutKeyframes(raw);
    assert.deepEqual(parsed,raw);assert.ok(Object.isFrozen(parsed));raw[0]=1;assert.equal(parsed[0],0xfffffff0);
    for(const bad of [null,[],Array(17).fill(1),[1,1],[-1],[1.1],[0x100000000],['12000'],[3,2],[0,0x80000000],Array(1)])
        assert.throws(()=>parseWebRtcPreCutKeyframes(bad),{code:'protocol'});
});
