import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserIceRestart} from '../typescript/dist/browser_webrtc_restart.js';
import {ConnectError} from '../typescript/dist/errors.js';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const initial = {cid:'conversation',sid:'session',connid:'connection',operation_id:'initial-offer',expected_revision:0};
const sdp = 'v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\na=ice-ufrag:newU\r\na=ice-pwd:PRIVATE-TEST-PASSWORD\r\n';
class Peer extends EventTarget {
    signalingState='stable'; iceGatheringState='complete'; localDescription; remoteDescription;
    offers=[]; local=[]; answers=[];
    async createOffer(options) { this.offers.push(options); return {type:'offer',sdp}; }
    async setLocalDescription(value) { this.local.push(value); this.localDescription={...value}; this.signalingState='have-local-offer'; }
    async setRemoteDescription(value) { this.answers.push(value); this.remoteDescription={...value}; this.signalingState='stable'; }
}
function fixture() {
    const peer=new Peer(), lifetime=new AbortController(), source={...initial}, requests=[], failures=[];
    let submitted;
    const client={
        async request(type,payload,scope) {
            requests.push({type,payload:structuredClone(payload),scope:structuredClone(scope)}); submitted=scope;
            return [{v:2,type:'ack',id:'answer',ts:'2026-10-30T00:00:00Z',...scope,in_reply_to:'request',payload:{details:{reply_complete:true,ice_restart:{proposed_ice_generation:payload.details.expected_ice_generation+1,setup:{kind:'websocket+webrtc',sdp:sdp+'a=answer:original\r\n'}}}}}];
        },
        async operationStatus(cid,operation) {
            return {operation_id:operation,state:'completed',result:{action:'ice-restart',connection_id:'connection',connection_generation:7,stream_revision:submitted.expected_revision,ice_generation:requests.at(-1).payload.details.expected_ice_generation+1,resources_changed:false}};
        }
    };
    const owner=new BrowserIceRestart(client,peer,source,lifetime.signal,()=>{if(lifetime.signal.aborted)throw new ConnectError('closed','fixture closed');},error=>failures.push(error));
    return {owner,peer,client,lifetime,source,requests,failures};
}
test('same original peer preserves scope and gathered bytes, applies one answer and requires captured completion',async()=>{
    const f=fixture(); f.source.connid='mutated'; let reads=0; const status=f.client.operationStatus;
    f.client.operationStatus=async(...args)=>++reads===1?{operation_id:'restart',state:'preparing'}:status(...args);
    const result=await f.owner.restart('restart',3,0,{});
    assert.deepEqual(result,{operationId:'restart',iceGeneration:1,streamRevision:3,connectionGeneration:7});
    assert.deepEqual(f.peer.offers,[{iceRestart:true}]); assert.equal(f.peer.local.length,1);assert.equal(f.peer.answers.length,1);assert.equal(reads,2);
    assert.equal(f.requests[0].scope.connid,'connection');assert.equal(f.requests[0].scope.operation_id,'restart');assert.equal(f.requests[0].scope.expected_revision,3);
    assert.equal(f.requests[0].payload.details.setup.sdp,sdp);assert.equal(f.requests[0].payload.details.expected_ice_generation,0);
    const saved=f.owner.saved;saved.payload.details.setup.sdp='forged';saved.scope.operation_id='forged';
    assert.equal(f.owner.saved.payload.details.setup.sdp,sdp);assert.equal(f.owner.saved.scope.operation_id,'restart');
    assert.equal((await f.owner.restart('restart',3,0,{})).iceGeneration,1);assert.equal(f.peer.offers.length,1);
    await f.owner.shutdown();
});
test('discarded reply recovers exact original request without another offer, peer or local application',async()=>{
    const f=fixture(), request=f.client.request;let lost=true;
    f.client.request=async(...args)=>{const frames=await request(...args);if(lost){lost=false;throw new ConnectError('timeout','discarded reply');}return frames;};
    await assert.rejects(f.owner.restart('original-restart',2,0,{}),{code:'timeout'});
    assert.equal(f.owner.saved.submissionAttempted,true);
    assert.throws(()=>f.owner.restart('different',2,0,{}),{code:'conflict'});
    assert.throws(()=>f.owner.restart('original-restart',3,0,{}),{code:'conflict'});
    const result=await f.owner.restart('original-restart',2,0,{});assert.equal(result.iceGeneration,1);
    assert.deepEqual(f.requests[0],f.requests[1]);assert.equal(f.peer.offers.length,1);assert.equal(f.peer.answers.length,1);assert.equal(f.failures.length,0);
});
test('cancelled gathering wait rejoins original preparation and retains newly gathered candidate bytes',async()=>{
    const f=fixture();f.peer.iceGatheringState='gathering';const stop=new AbortController();
    const first=f.owner.restart('restart',2,0,{signal:stop.signal});while(!f.peer.localDescription)await delay(1);
    stop.abort();await assert.rejects(first,{code:'cancelled'});assert.equal(f.requests.length,0);assert.equal(f.peer.offers.length,1);
    f.peer.localDescription.sdp=sdp+'a=end-of-candidates\r\n';f.peer.iceGatheringState='complete';f.peer.dispatchEvent(new Event('icegatheringstatechange'));
    await f.owner.restart('restart',2,0,{});assert.equal(f.peer.offers.length,1);assert.match(f.requests[0].payload.details.setup.sdp,/end-of-candidates/);
});
test('pending original setRemoteDescription is joined once after observer timeout',async()=>{
    const f=fixture();let release;const apply=f.peer.setRemoteDescription.bind(f.peer);let calls=0;
    f.peer.setRemoteDescription=value=>{calls++;return new Promise(resolve=>{release=()=>apply(value).then(resolve);});};
    await assert.rejects(f.owner.restart('restart',2,0,{timeoutMs:10}),{code:'timeout'});
    assert.equal(calls,1);assert.equal(f.requests.length,1);
    const recovered=f.owner.restart('restart',2,0,{});release();await recovered;
    assert.equal(calls,1);assert.equal(f.requests.length,1);assert.equal(f.peer.offers.length,1);
});
test('proposed answer alone cannot become ready when original operation is unknown',async()=>{
    const f=fixture();f.client.operationStatus=async()=>({operation_id:'restart',state:'unknown'});
    await assert.rejects(f.owner.restart('restart',2,0,{}),{code:'unresolved-operation'});
    assert.equal(f.peer.answers.length,1);assert.throws(()=>f.owner.restart('replacement',2,0,{}),{code:'conflict'});
    assert.equal(f.peer.offers.length,1);
});
test('wrong correlation, generations and mixed readiness assertions refuse before applying remote SDP',async()=>{
    for(const change of [frame=>{frame.operation_id='foreign';},frame=>{frame.payload.details.ice_restart.proposed_ice_generation=2;},frame=>{frame.payload.details.ready=true;},frame=>{frame.payload.details.ice_restart.setup.kind='native-prepared/1';}]){
        const f=fixture(), request=f.client.request;f.client.request=async(...args)=>{const frames=await request(...args);change(frames[0]);return frames;};
        await assert.rejects(f.owner.restart('restart',2,0,{}),{code:'protocol'});assert.equal(f.peer.answers.length,0);assert.equal(f.failures.length,1);
    }
});
test('invalid deadlines, counters and pre-cancelled waits perform zero browser or wire IO',async()=>{
    for(const [expected,options] of [[Number.MAX_SAFE_INTEGER,{}],[-1,{}],[0,{timeoutMs:0}],[0,{timeoutMs:30001}],[0,{signal:AbortSignal.abort()}]]){
        const f=fixture();await assert.rejects(async()=>f.owner.restart('restart',2,expected,options));assert.equal(f.peer.offers.length,0);assert.equal(f.requests.length,0);
    }
});
test('owner retirement cancels and joins original gathering without another peer or request',async()=>{
    const f=fixture();f.peer.iceGatheringState='gathering';const pending=f.owner.restart('restart',2,0,{});while(!f.peer.localDescription)await delay(1);
    f.lifetime.abort();await assert.rejects(pending);await f.owner.shutdown();assert.equal(f.peer.offers.length,1);assert.equal(f.requests.length,0);
});
