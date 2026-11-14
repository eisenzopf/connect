import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {matrixFirefoxPage,matrixFirefoxUserPreferences,launchMatrixBrowser} from '../video/owned_matrix_browser.mjs';
import {bidiFirefoxPage} from '../video/browser_bidi_adapter.mjs';
import {installPeerDiagnostics} from '../video/browser_failure.mjs';

for(const [name,facade]of [['matrix',matrixFirefoxPage],['BiDi',bidiFirefoxPage]]){
    test(`${name} Firefox facade injects the actual sixteen-peer diagnostics argument`,async()=>{
        class Peer{
            constructor(){this.connectionState=this.iceConnectionState=this.iceGatheringState='new';this.signalingState='stable';}
            getTransceivers(){return [];}
            addEventListener(){}removeEventListener(){}
        }
        const realm=vm.createContext({RTCPeerConnection:Peer,performance:{now:()=>17}});
        const options={peerBound:16},receipt={identifier:'original-init-script'},calls=[];
        const page={async evaluateOnNewDocument(fn,...args){
            calls.push({fn,args});
            vm.runInContext(`(${fn.toString()})(...${JSON.stringify(args)})`,realm);
            return receipt;
        }};
        assert.equal(await facade(page).addInitScript(installPeerDiagnostics,options),receipt);
        assert.equal(calls.length,1);assert.equal(calls[0].fn,installPeerDiagnostics);
        assert.equal(calls[0].args.length,1);assert.equal(calls[0].args[0],options);
        for(let i=0;i<16;i++)new realm.RTCPeerConnection();
        const observed=realm.connectPeerDiagnostics();
        assert.equal(observed.peer_bound,16);assert.equal(observed.peers.length,16);
        assert.equal(observed.omitted_peers,0);
    });
    test(`${name} Firefox facade preserves no-argument injection and original rejection`,async()=>{
        const calls=[],failure=Error('original injection failure');
        const page={evaluateOnNewDocument(...args){calls.push(args);return Promise.reject(failure);}};
        const fn=()=>{};
        await assert.rejects(facade(page).addInitScript(fn),error=>error===failure);
        assert.equal(calls.length,1);assert.deepEqual(calls[0],[fn]);
    });
}


test('Firefox numeric ICE preference is confined to the explicit private source-frame witness profile',()=>{
    const spec={browserName:'firefox',transport:'firefox-wss-webrtc',stage:'quality',origin:'https://127.0.0.1:9443',fixtureAddresses:['127.0.0.1']};
    const baseline=[
        'user_pref("media.peerconnection.ice.loopback", true);',
        'user_pref("media.autoplay.default", 0);',
        'user_pref("media.autoplay.blocking_policy", 0);',
        'user_pref("browser.shell.checkDefaultBrowser", false);',
        'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
        'user_pref("toolkit.telemetry.enabled", false);',
    ].join('\n')+'\n';
    assert.equal(matrixFirefoxUserPreferences(spec),baseline);
    assert.equal(matrixFirefoxUserPreferences({...spec,rtcSourceFrameWitness:false}),baseline);
    const enabled=matrixFirefoxUserPreferences({...spec,rtcSourceFrameWitness:true});
    const numeric='user_pref("media.peerconnection.ice.obfuscate_host_addresses", false);\n';
    assert.equal(enabled.split(numeric).length,2);
    assert.equal(enabled.replace(numeric,''),baseline);
    const lan={...spec,origin:'https://192.168.1.5:9443',fixtureAddresses:['192.168.1.5','10.1.2.3']};
    assert.equal(matrixFirefoxUserPreferences(lan),baseline.replace('user_pref("media.peerconnection.ice.loopback", true);\n',''));
    assert.ok(matrixFirefoxUserPreferences({...lan,rtcSourceFrameWitness:true}).includes(numeric));
    for(const stage of ['admission','programme','quality']){
        assert.equal(matrixFirefoxUserPreferences({...spec,stage,rtcSourceFrameWitness:false}),baseline);
    }
});

test('Owned browser rejects invalid numeric ICE scope before profile writes or external launch',async()=>{
    const good={browserName:'firefox',transport:'firefox-wss-webrtc',stage:'quality',origin:'https://127.0.0.1:9443',fixtureAddresses:['127.0.0.1'],rtcSourceFrameWitness:true};
    const invalid=[{browserName:'chrome'},{browserName:'unknown'},{transport:'chrome-wss-webrtc'},
        {stage:'admission'},{stage:'programme'},{rtcSourceFrameWitness:'true'},{rtcSourceFrameWitness:1},
        {fixtureAddresses:[]},{fixtureAddresses:['name.local']},{fixtureAddresses:['203.0.113.1']},
        {fixtureAddresses:['127.0.0.1','10.0.0.1','10.0.0.2']},{fixtureAddresses:['10.0.0.1']},
        {origin:'http://127.0.0.1:9443'},{origin:'https://user:secret@127.0.0.1:9443'},
        {origin:'https://127.0.0.1:9443/?query=1'},{origin:'https://127.0.0.1:9443/#fragment'}];
    for(const change of invalid){
        const spec={...good,...change};
        assert.throws(()=>matrixFirefoxUserPreferences(spec));
        await assert.rejects(launchMatrixBrowser(spec,'/unused-invalid-profile',()=>{throw Error('Profile initializer must not run');}),
            error=>error.message!=='Profile initializer must not run');
    }
});
