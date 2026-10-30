import test from 'node:test';
import assert from 'node:assert/strict';
import {observedProxyPaths} from '../video/browser_network_proxy.mjs';

const path=()=>({peer_count:1,selected_pairs:[{succeeded:true,nominated:true,udp:true,sent:true,received:true,
    prescribed_address_selected:true,prescribed_port_selected:true}]});
const page=value=>({evaluate:async()=>structuredClone(value)});
test('both current nominated duplex paths are required',async()=>{
    assert.equal((await observedProxyPaths([page(path()),page(path())],{})).length,2);
});
test('an in-progress selected-path check is freshly reobserved',async()=>{
    let calls=0;
    const transient={evaluate:async()=>{const row=path();row.selected_pairs[0].succeeded=++calls>1;return row;}};
    await observedProxyPaths([transient,page(path())],{});
    assert.ok(calls>1);
});
test('persistent foreign addressing remains refused within one bounded observation',async()=>{
    const foreign=path();foreign.selected_pairs[0].prescribed_address_selected=false;
    const began=performance.now();
    await assert.rejects(observedProxyPaths([page(path()),page(foreign)],{}),error=>{
        assert.equal(error.owned_network_paths[1].selected_pairs[0].prescribed_address_selected,false);return true;
    });
    assert.ok(performance.now()-began>=2000);
});
test('an unresolved metadata read cannot outlive the observation budget',async()=>{
    const stalled={evaluate:()=>new Promise(()=>{})};
    await assert.rejects(observedProxyPaths([stalled,page(path())],{}));
});
