import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {matrixFirefoxPage} from '../video/owned_matrix_browser.mjs';
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
