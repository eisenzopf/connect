import test from 'node:test';
import assert from 'node:assert/strict';
import {validateBrowserFixtureScope} from '../video/browser_fixture_scope.mjs';
const runtime={platform:'linux',arch:'x64',env:{CONNECT_GATE6_OWNED_BROWSER_CONTAINER:'1'}};
const fixture=()=>({ownedContainerBridge:{ip:'172.19.0.2',port_start:32000,port_end:32031,require_relay:true},
    designatedRemoteHost:'connect.fixture.internal',iceRestart:true,requireRelay:true,profileMode:'clean',
    participants:[{connection:{iceTransportPolicy:'relay'}},{connection:{iceTransportPolicy:'relay'}}]});
test('owned relay bridge admits both original peers and its exact TURN range',()=>{
    validateBrowserFixtureScope(fixture(),runtime);
});
test('owned direct bridge retains its sixteen-port range without ICE recovery',()=>{
    const s=fixture();Object.assign(s,{iceRestart:false,requireRelay:false});
    Object.assign(s.ownedContainerBridge,{require_relay:false,port_start:40000,port_end:40015});
    validateBrowserFixtureScope(s,runtime);
    s.ownedContainerBridge.port_end=40031;assert.throws(()=>validateBrowserFixtureScope(s,runtime));
});
test('remote names and public addresses cannot obtain the owned exception',()=>{
    for(const change of [s=>s.designatedRemoteHost='remote.example',s=>s.ownedContainerBridge.ip='8.8.8.8',
        s=>delete s.ownedContainerBridge]){
        const s=fixture();change(s);assert.throws(()=>validateBrowserFixtureScope(s,runtime));
    }
});
test('other platforms and processes cannot use the bridge exception',()=>{
    for(const r of [{...runtime,platform:'darwin'},{...runtime,arch:'arm64'},{...runtime,env:{}}])
        assert.throws(()=>validateBrowserFixtureScope(fixture(),r));
});
test('relay requires exact ports, both relay-only peers, original recovery and clean mode',()=>{
    for(const change of [s=>s.ownedContainerBridge.port_start++,s=>s.ownedContainerBridge.require_relay='true',s=>s.ownedContainerBridge.port_end--,
        s=>s.ownedContainerBridge.require_relay=false,s=>s.requireRelay=false,s=>s.iceRestart=false,
        s=>s.profileMode='impairment',s=>s.participants.pop(),s=>s.participants[1].connection.iceTransportPolicy='all']){
        const s=fixture();change(s);assert.throws(()=>validateBrowserFixtureScope(s,runtime));
    }
});
test('existing loopback relay and direct recovery scenes remain allowed',()=>{
    const s=fixture();delete s.ownedContainerBridge;delete s.designatedRemoteHost;
    validateBrowserFixtureScope(s,runtime);s.requireRelay=false;validateBrowserFixtureScope(s,runtime);
});
