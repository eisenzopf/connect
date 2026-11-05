import {test} from 'node:test';
import assert from 'node:assert/strict';
import {nativeActivation,nativeMembership,nativeCut,nativeEgressIdentity} from '../typescript/dist/browser_native_activation.js';
import {bindNativeStreams,nativeBoundary} from './native_activation_fixture.mjs';
const root={recordId:'record',operationId:'move',sourceConnectionId:'old',targetConnectionId:'new'};
const streams=bindNativeStreams([{strm_id:'audio',kind:'audio',direction:'sendrecv',stream_local_id:1},{strm_id:'video',kind:'video',direction:'sendonly',stream_local_id:2}],2);
const receivers=streams.map(s=>({streamId:s.strm_id,handle:s.stream_local_id,identity:s.native_egress}));
const boundary=()=>nativeBoundary(root,streams,2,3,{audio:4,video:8});
const status=()=>({state:'completed',record_id:'record',operation_id:'move',result:{activation_state:'ready',connection_id:'new',replaces:'old',generation:2,readiness:{stream_revision:3},native_activation:boundary()}});
const observation=(...seqs)=>({profile:'connect/native-decoded-sequence/1',identity:streams[0].native_egress,datagramSequences:seqs,ssrc:7,rtpTimestamp:960});
test('original frozen reader classifies actual public sequence before completion reply without a transmission claim',()=>{
 const before=observation(4),original=structuredClone(before),receipt=nativeActivation(root,status(),receivers);
 assert.equal(nativeMembership(receipt.audience.streams[0],before),'public');assert.deepEqual(before,original);
 assert.equal(nativeMembership(receipt.audience.streams[0],observation(3)),'private');assert.equal(receipt.audience.streams[0].submission_observed,false);
 assert.deepEqual(nativeActivation(root,status(),receivers),receipt);assert.ok(Object.isFrozen(receipt.audience.streams));
});
for(const [name,mutate] of [
 ['foreign record',v=>v.operation_record_id='other'],['foreign operation',v=>v.operation_id='other'],['foreign source',v=>v.source_connection_id='other'],['foreign target',v=>v.target_connection_id='other'],['generation',v=>v.target_generation=3],['revision',v=>v.stream_revision=4],['incarnation',v=>v.audience.streams[0].physical_incarnation='f'.repeat(32)],['stream',v=>v.audience.streams[0].stream_id='foreign'],['handle',v=>v.audience.streams[0].stream_local_id=3],['stream generation',v=>v.audience.streams[0].stream_generation=3],['missing stream',v=>v.audience.streams.pop()],['duplicate stream',v=>v.audience.streams[1]=structuredClone(v.audience.streams[0])],['unobserved IO',v=>v.audience.streams[0].submission_observed=true],['unknown property',v=>v.audience.transmitted=true]])
 test(`native cut refuses ${name}`,()=>{const value=status();mutate(value.result.native_activation);assert.throws(()=>nativeActivation(root,value,receivers),{code:'protocol'});});
test('original private consultation can never be interpreted as public media',()=>{
 const s=status();s.result.native_activation.audience={scope:'private-consultation',original_operation:'consultation'};
 assert.equal(nativeActivation(root,s,receivers).audience.scope,'private-consultation');s.result.native_activation.audience.streams=boundary().audience.streams;
 assert.throws(()=>nativeActivation(root,s,receivers),{code:'protocol'});
});
test('a frame spanning the cut, repeated reservations, half-range or wrapped cursor refuses',()=>{
 const cut=boundary().audience.streams[0];for(const seqs of [[3,4],[4,4],[2**31],[2**32-1],[]])assert.throws(()=>nativeMembership(cut,observation(...seqs)),{code:'protocol'});
 for(const patch of [{cursor_ordinal:2**31,public_sequence_floor:2**31},{cursor_ordinal:2**32,public_sequence_floor:0},{cursor_ordinal:5},{stream_id:'a'.repeat(257)}])assert.throws(()=>nativeCut({...cut,...patch}),{code:'protocol'});
});
test('decoded provenance cannot substitute another actual physical epoch or original start',()=>{
 const cut=boundary().audience.streams[0];for(const patch of [{physical_incarnation:'f'.repeat(32)},{stream_generation:2},{initial_outer_sequence:4}])assert.throws(()=>nativeMembership(cut,{...observation(4),identity:{...streams[0].native_egress,...patch}}),{code:'protocol'});
 assert.throws(()=>nativeEgressIdentity({...streams[0].native_egress,source:'guessed'}),{code:'protocol'});
});
