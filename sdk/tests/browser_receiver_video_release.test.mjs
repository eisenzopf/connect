// CANDIDATE ONLY: local membership/lease model, never signed control or media proof.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {ReceiverVideoReleaseOwner} from '../typescript/dist/browser_receiver_video_release.js';
const scope=()=>({operation_record_id:'record',operation_id:'move',reservation_id:'reservation',source_connection_id:'old',target_connection_id:'target',stream_id:'public-video',stream_generation:7,connection_generation:9,target_generation:2,source_cut_id:'cut'});
const original=()=>({operationRecordId:'record',operationId:'move',targetConnectionId:'target',releaseAuthorized:true});
const writer='a'.repeat(32);
const rtpIdentity=()=>({carrier:'webrtc-rtp',physical_writer_incarnation:writer,primary_ssrc:123});
const nativeIdentity=()=>({carrier:'native-datagram',physical_incarnation:writer,stream_local_id:10});
const cut=()=>({stream_id:'public-video',stream_local_id:10,stream_generation:7,physical_incarnation:writer,sequence_space:'native-outer-u32',public_sequence_floor:100,cursor_ordinal:100,submission_observed:false});
const wire=(carrier='webrtc-rtp')=>({...scope(),profile:'connect-receiver-video-release-v1',original_frame:{realm:'f'.repeat(32),ordinal:'2',last_old_ordinal:'1'},floor:carrier==='webrtc-rtp'?{...rtpIdentity(),first_public_rtp_timestamp:1000}:{carrier:'native-datagram',cut:cut()}});
const packet=(sequences)=>({profile:'connect/native-decoded-sequence/1',identity:{profile:'native-sequence-cursor/1',physical_incarnation:writer,stream_generation:7,connection_generation:9,initial_outer_sequence:0},datagramSequences:sequences,ssrc:123,rtpTimestamp:1000});
test('no original signed release means no output loan; observe is immutable and exact-root',()=>{
 const owner=new ReceiverVideoReleaseOwner(scope(),rtpIdentity(),()=>true);
 assert.throws(()=>owner.borrow());
 for(const mutate of [v=>v.operation_id='other',v=>v.reservation_id='other',v=>v.stream_generation++,v=>v.connection_generation++,v=>v.target_generation++,v=>v.source_cut_id='other-cut',v=>v.floor.physical_writer_incarnation='c'.repeat(32),v=>v.floor.primary_ssrc++,v=>v.original_frame.ordinal='01',v=>v.original_frame.last_old_ordinal='2',v=>v.original_frame.last_old_ordinal='3',v=>v.original_frame.last_old_ordinal='18446744073709551616',v=>v.original_frame.realm='0000']){
  const raw=wire();mutate(raw);assert.throws(()=>owner.observe(raw,original()));
 }
 assert.throws(()=>owner.observe(wire(),{...original(),releaseAuthorized:false}));
 assert.throws(()=>owner.observe(wire(),{...original(),targetConnectionId:'other'}));
 const raw=wire(),selected=owner.observe(raw,original());raw.floor.first_public_rtp_timestamp=900;
 assert.equal(selected.floor.first_public_rtp_timestamp,1000);
 assert.strictEqual(owner.observe(wire(),original()),selected);
 assert.throws(()=>owner.observe(raw,original()),'floor cannot move backward or forward on replay');
 assert.ok(Object.isFrozen(selected)&&Object.isFrozen(selected.floor)&&Object.isFrozen(selected.original_frame));
});
test('RTP floor excludes old output and tracks high water across exclusive reborrow',()=>{
 let current=true;const owner=new ReceiverVideoReleaseOwner(scope(),rtpIdentity(),()=>current);
 owner.observe(wire(),original());const loan=owner.borrow();
 assert.throws(()=>owner.borrow());
 for(const output of [{rtpTimestamp:999,presentedFrames:1},{rtpTimestamp:1000,presentedFrames:0},{rtpTimestamp:NaN,presentedFrames:1},{presentedFrames:1}])assert.equal(loan.eligibleRtp(output),false);
 const first={rtpTimestamp:1000,presentedFrames:10};assert.equal(loan.eligibleRtp(first),true);loan.presentedRtp(first);
 assert.equal(loan.eligibleRtp(first),false);
 assert.equal(loan.eligibleRtp({rtpTimestamp:1001,presentedFrames:10}),false);
 loan.release();const retry=owner.borrow();assert.equal(loan.eligibleRtp({rtpTimestamp:1001,presentedFrames:11}),false);
 assert.equal(retry.eligibleRtp(first),false);assert.equal(retry.eligibleRtp({rtpTimestamp:1001,presentedFrames:11}),true);
 current=false;assert.equal(retry.eligibleRtp({rtpTimestamp:1001,presentedFrames:11}),false);
 current=true;assert.equal(retry.eligibleRtp({rtpTimestamp:1001,presentedFrames:11}),false,'observed retirement cannot revive after regrant');
 owner.close();assert.throws(()=>owner.borrow());assert.throws(()=>owner.observe(wire(),original()));
});
test('RTP wrap membership uses same physical Track+SSRC timeline, refusing exact half range',()=>{
 const owner=new ReceiverVideoReleaseOwner(scope(),rtpIdentity(),()=>true),raw=wire();raw.floor.first_public_rtp_timestamp=0xfffffff0;
 owner.observe(raw,original());const loan=owner.borrow();
 assert.equal(loan.eligibleRtp({rtpTimestamp:0xfffffff0,presentedFrames:1}),true);
 loan.presentedRtp({rtpTimestamp:0xfffffff0,presentedFrames:1});
 assert.equal(loan.eligibleRtp({rtpTimestamp:5,presentedFrames:2}),true);
 assert.equal(loan.eligibleRtp({rtpTimestamp:0x7ffffff0,presentedFrames:2}),false);
 assert.equal(loan.eligibleRtp({rtpTimestamp:0xffffffef,presentedFrames:2}),false);
});
test('native floor reuses original cut and requires all original frame fragments agree',()=>{
 const owner=new ReceiverVideoReleaseOwner(scope(),nativeIdentity(),()=>true);owner.observe(wire('native-datagram'),original());const loan=owner.borrow();
 assert.equal(loan.eligibleNative(packet([99])),false);assert.equal(loan.eligibleNative(packet([99,100])),false);
 assert.equal(loan.eligibleNative(packet([100,101])),true);
 const wrong=packet([100]);wrong.identity.physical_incarnation='c'.repeat(32);assert.equal(loan.eligibleNative(wrong),false);
 const generation=packet([100]);generation.identity.connection_generation++;assert.equal(loan.eligibleNative(generation),false);
 assert.equal(loan.eligibleNative(packet([0x80000000])),false);
 assert.equal(loan.eligibleNative(packet([100,100])),false);
 assert.equal(loan.eligibleNative(undefined),false);
 loan.presentedNative(packet([100,101]));loan.release();const retry=owner.borrow();
 assert.equal(retry.eligibleNative(packet([100,101])),false);assert.equal(retry.eligibleNative(packet([102])),true);
 assert.equal(retry.eligibleRtp({rtpTimestamp:1000,presentedFrames:1}),false);
});
test('native parser rejects rewritten cut identity, floor, generation and unknown fields',()=>{
 for(const mutate of [v=>v.floor.cut.stream_generation++,v=>v.floor.cut.physical_incarnation='c'.repeat(32),v=>v.floor.cut.stream_local_id++,v=>v.floor.cut.public_sequence_floor++,v=>v.floor.cut.submission_observed=true,v=>v.floor.extra=true]){
  const owner=new ReceiverVideoReleaseOwner(scope(),nativeIdentity(),()=>true),raw=wire('native-datagram');mutate(raw);assert.throws(()=>owner.observe(raw,original()));
 }
});
