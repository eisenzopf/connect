import test from 'node:test';
import assert from 'node:assert/strict';
import {observedBridgePaths} from '../video/browser_bridge_proof.mjs';

const path=()=>({nominated:true,succeeded:true,expected_address:true,expected_port:true,udp:true,sent:true,received:true});
const page=paths=>({evaluate:async()=>paths});
test('owned bridge requires a current nominated duplex path from both peers',async()=>{
    assert.equal((await observedBridgePaths([page([[path()]]),page([[path()]])],{})).selected_paths,2);
});
test('foreign addresses, ports, inactive paths and one-way counters cannot qualify the bridge',async()=>{
    for(const field of Object.keys(path())){
        await assert.rejects(observedBridgePaths([page([[path()]]),page([[{...path(),[field]:false}]])],{}));
    }
});
test('missing or extra peers and selected transports cannot qualify the bridge',async()=>{
    for(const paths of [[],[[]],[[path(),path()]],[[path()],[path()]]]){
        await assert.rejects(observedBridgePaths([page([[path()]]),page(paths)],{}));
    }
});
test('relay scope requires actual local and remote relay candidates',async()=>{
    const relay=()=>({...path(),local_relay:true,remote_relay:true});
    const result=await observedBridgePaths([page([[relay()]]),page([[relay()]])],{require_relay:true});
    assert.equal(result.expected_private_relay,true);assert.equal(result.expected_private_gateway,false);
    for(const field of ['local_relay','remote_relay']){
        await assert.rejects(observedBridgePaths([page([[relay()]]),page([[{...relay(),[field]:false}]])],{require_relay:true}));
    }
});
