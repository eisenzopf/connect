import assert from 'node:assert/strict';
import {test} from 'node:test';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import vm from 'node:vm';
import {strictPcmIdentity,PcmSampleLedger} from '../lib/browser_matrix_pcm.mjs';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('../../',import.meta.url));
const fixtures=['browser-public-video-roundtrip.mjs','browser-public-video-abort.mjs',
    'browser-native-public-b-target.mjs','browser-replacement-roundtrip.mjs','browser-replacement-terminal.mjs'];
const diagnostic='/browser_matrix_rtc_audio_diagnostics.mjs';
const imports=source=>[
    ...[...source.matchAll(/\b(?:import|export)\s+(?:[^'";]*?\s+from\s*)?['"]([^'"]+)['"]/g)].map(m=>m[1]),
    ...[...source.matchAll(/\bimport\s*\(\s*['"]([^'"]+)['"]/g)].map(m=>m[1]),
];
async function fixtureServer(t,name,omitDiagnostic=false){
    const source=await fs.readFile(path.join(root,'scripts/video',name),'utf8');
    // Execute ONLY the actual isolated HTTP server declaration. Do not import
    // the media entrypoint, read private stdin, connect CDP or start a browser.
    const begin=source.indexOf('const assets=http.createServer(');
    const end=source.indexOf('await new Promise',begin);
    assert.ok(begin>=0&&end>begin,'actual original asset handler retained');
    const sdkRoot=await fs.realpath(path.join(root,'sdk/typescript/dist'));
    const server=vm.runInNewContext(source.slice(begin,end)+'\nassets;',{
        assert,fs,http,path,URL,root,sdkRoot,sdk:sdkRoot,
        tapFile:path.join(root,'scripts/lib/browser_matrix_rtc_observation.mjs'),
        pcmFile:path.join(root,'scripts/lib/browser_matrix_pcm.mjs'),
        boundaryFile:path.join(root,'scripts/lib/browser_matrix_stages.mjs'),
    });
    if(omitDiagnostic){
        const original=server.listeners('request');server.removeAllListeners('request');
        server.on('request',(request,response)=>{
            if(new URL(request.url,'http://127.0.0.1').pathname===diagnostic){response.writeHead(404);response.end('negative-control');return;}
            for(const handler of original)handler.call(server,request,response);
        });
    }
    t.after(async()=>{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));});
    await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
    return {origin:`http://127.0.0.1:${server.address().port}`,source};
}
async function graph({origin,source}){
    const pending=imports(source).filter(spec=>spec.startsWith('/')).map(spec=>new URL(spec,origin));
    assert.ok(pending.length>=4,'actual browser entrypoint module imports');
    const seen=new Set();
    while(pending.length){
        const url=pending.shift();assert.equal(url.origin,origin);
        if(seen.has(url.pathname))continue;
        assert.ok(seen.size<256,'bounded fixture import graph');
        const response=await fetch(url);assert.equal(response.status,200,'HTTP module '+url.pathname);
        assert.match(response.headers.get('content-type'),/^text\/javascript/);
        seen.add(url.pathname);
        for(const spec of imports(await response.text())){
            assert.ok(spec.startsWith('.')||spec.startsWith('/'),'local browser module import '+spec);
            pending.push(new URL(spec,url));
        }
    }
    assert.ok(seen.has(diagnostic),'real observer dependency traversed');
    return seen;
}
for(const name of fixtures){
    test(name+' serves the actual browser entrypoint and complete HTTP import graph',async t=>{
        const seen=await graph(await fixtureServer(t,name));
        assert.ok(seen.has(name==='browser-native-public-b-target.mjs'?'/observer.mjs':'/rtc-observer.mjs'));
        assert.ok(seen.has('/sdk/browser_sdk_webrtc.js'));
    });
    test(name+' graph rejects a missing real observer diagnostic dependency',async t=>{
        await assert.rejects(graph(await fixtureServer(t,name,true)),error=>
            error.code==='ERR_ASSERTION'&&error.message.includes('HTTP module '+diagnostic));
    });
    test(name+' exact dependency route retains private asset and method boundaries',async t=>{
        const {origin}=await fixtureServer(t,name);
        for(const [url,method]of [[diagnostic,'POST'],['/browser_matrix_rtc_audio_diagnostics.mjs.extra','GET'],
            ['/scripts/lib/browser_matrix_rtc_audio_diagnostics.mjs','GET'],['/sdk/%2e%2e/package.json','GET']]){
            const response=await fetch(new URL(url,origin),{method});assert.equal(response.status,404,url+' '+method);
            await response.text();
        }
    });
}

for(const name of fixtures)test(name+' serves executable strict identity and sample-ledger exports through its unchanged exact PCM route',async t=>{
    const {origin}=await fixtureServer(t,name);
    const response=await fetch(new URL('/pcm-identity.mjs',origin));assert.equal(response.status,200);
    const source=await response.text(),module=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
    const samples=Float32Array.from({length:960},(_,i)=>i<832?0:.15*Math.sin(2*Math.PI*660*(i-832)/48000+.3));
    const identity=module.strictPcmIdentity(samples,1);assert.equal(identity.legacy.source_identified,false);assert.equal(identity.source_identified,true);
    const ledger=new module.PcmSampleLedger();ledger.push({owner:'public',clock:{owner:'public',profile:'connect/rtc-decoded-pcm-clock/1',epoch_id:1},timestamp:0,at:0,identity});
    assert.equal(ledger.report().available_samples,128);assert.equal(ledger.report().unavailable_samples,832);
    assert.equal(module.PCM_MEASUREMENT_METHOD,'connect/finite-support-decoded-tone-and-sample-coverage/1');
});

for(const name of fixtures)test(name+' actual audio callback refunds only a proved original crossing even on its original quiet-return path',async()=>{
    const source=await fs.readFile(path.join(root,'scripts/video',name),'utf8'),native=name==='browser-native-public-b-target.mjs',
        begin=source.indexOf(native?'audio(id,ts,at,samples,clock){':'audio(id,timestamp,at,samples,clock){'),
        end=source.indexOf(native?'video(id,ts,at,frame,w,h)':'video(id,timestamp,at,frame,width,height)',begin);
    assert.ok(begin>=0&&end>begin,'actual original callback method');
    const method=source.slice(begin,end),owner='original',ledger=new PcmSampleLedger(),p={audio:0,lastAudio:null},
        progress=name==='browser-replacement-terminal.mjs'?[p]:new Map([[owner,p]]);
    const powerDeclaration=native?source.slice(source.indexOf('const measuredPower='),source.indexOf('\n',source.indexOf('const measuredPower='))):'';
    const context={pcmIdentity:strictPcmIdentity,pcmLedger:ledger,progress,byConnection:new Map([[owner,0]]),bindingById:new Map([[owner,0]]),
        active:native?true:new Map([[0,owner]]),prepared:{connection:{id:owner}},toneIndex:1,expected:[1],sourceHz:[440,660,880,1100],
        check:(ok,message)=>assert.ok(ok,message),invalid:()=>{},pcmFailures:[],audioFailures:[],failures:[],renderTimes:new Map(),pcmBoundary:()=>({}),
        spec:{expectedAudioHz:660}};
    const entry=vm.runInNewContext(`(()=>{let audioSamples=0,lastAudio=0,minAudioExplainedEnergy=Infinity;
        ${powerDeclaration}const audio=({${method}}).audio;return {audio,counter:()=>audioSamples,last:()=>lastAudio};})()`,context);
    const clock={profile:'connect/rtc-decoded-pcm-clock/1',owner,epoch_id:1},samples=Float32Array.from({length:960},(_,i)=>i===959?0:.15*Math.sin(2*Math.PI*660*(i-959)/48000)),
        quiet=Float32Array.from({length:960},(_,i)=>.0005*Math.sin(2*Math.PI*660*(i+1)/48000));
    assert.equal(entry.audio(owner,0,100,samples,clock),true);assert.equal(native?entry.counter():p.audio*960,959);
    assert.equal(entry.audio(owner,20000,120,quiet,clock),true,'Actual quiet expected tone can supply an observed sign without usable-volume credit');
    assert.equal(native?entry.counter():p.audio*960,960,'Exactly the earlier1sample crossing is refunded immediately, before a quiet return');
    assert.equal(native?entry.last():p.lastAudio,100,'Quiet PCM never moves original fresh-audio timestamp');
    assert.equal(ledger.report().available_samples,960);
    assert.deepEqual(ledger.rows[0].unavailable_runs,[],'SAME earlier row gap accounting is updated, not merely its aggregate');
});

async function abortCallbackFixture(){
    const source=await fs.readFile(path.join(root,'scripts/video/browser-public-video-abort.mjs'),'utf8'),
        begin=source.indexOf('  HTMLVideoElement.prototype.requestVideoFrameCallback=function(callback){'),
        end=source.indexOf('  CanvasRenderingContext2D.prototype.drawImage=',begin);
    assert.ok(begin>=0&&end>begin,'actual original ownership hook');
    const track={id:'actual-track',kind:'video',readyState:'live'},receiver={track},transceiver={mid:'2',receiver},
        peer={connectionState:'connected',signalingState:'stable',getTransceivers:()=>[transceiver]},
        peers=new Map([['original-candidate',peer]]),decoded=new Map(),pending=[],invalid=[],retired=[],failures=[];
    class Stream{constructor(tracks){this.tracks=tracks;}getVideoTracks(){return this.tracks;}}
    class Element{constructor(tracks=[track]){this.srcObject=new Stream(tracks);}}
    const context={HTMLVideoElement:Element,MediaStream:Stream,peers,decoded,closeProof:null,
        callbackOwnershipFailures:failures,retiredCallbackObservations:retired,
        check:(ok,message)=>assert.ok(ok,message),invalid:message=>invalid.push(message),performance:{now:()=>10},
        nativeFrame:function(callback){const token=Object.freeze({registration:pending.length+1});pending.push({element:this,callback,token});return token;}};
    vm.runInNewContext(source.slice(begin,end),context);
    const element=new Element(),calls=[],result=Object.freeze({same:'original-callback-result'}),metadata=Object.freeze({presentedFrames:8,mediaTime:1.25});
    const callback=function(...args){calls.push({self:this,args});return result;};
    const register=(owner=element,fn=callback)=>{const token=owner.requestVideoFrameCallback(fn);assert.equal(token,pending.at(-1).token);return token;};
    const fire=()=>{const item=pending.shift();return item.callback.call(item.element,123,metadata);};
    const close=()=>{context.closeProof={connection_id:'original-candidate',only_native_peer_closed:true};peer.connectionState='closed';peer.signalingState='closed';track.readyState='ended';transceiver.mid=null;};
    return {track,receiver,transceiver,peer,peers,decoded,pending,invalid,retired,failures,Stream,Element,context,element,calls,result,metadata,register,fire,close};
}
test('actual Abort ownership hook retains native registration token and original live callback this/args/result',async()=>{
    const f=await abortCallbackFixture();f.register();assert.equal(f.fire(),f.result);
    assert.equal(f.calls.length,1);assert.equal(f.calls[0].self,f.element);assert.equal(f.calls[0].args[0],123);assert.equal(f.calls[0].args[1],f.metadata);
    const value=f.decoded.get(f.track);assert.equal(value.callbacks,1);assert.equal(value.last,10);assert.ok(Object.isFrozen(value.ownerProof));
    assert.equal(f.invalid.length,0);assert.equal(f.retired.length,0);
});
test('actual queued Abort callback retains registered owner when close clears MID but earns zero new decoded/public credit',async()=>{
    const f=await abortCallbackFixture();f.register();f.fire();const value=f.decoded.get(f.track),before={callbacks:value.callbacks,last:value.last};
    f.register();f.close();assert.equal(f.fire(),f.result);
    assert.equal(f.calls.length,2);assert.equal(f.invalid.length,0);assert.equal(f.retired.length,1);
    assert.equal(value.callbacks,before.callbacks);assert.equal(value.last,before.last);
    assert.equal(f.retired[0].public_grade,false);assert.equal(f.retired[0].mid_cleared,true);assert.equal(f.retired[0].registered_mid,'2');
});
test('first queued callback after exact close can retain registration proof without inventing prior decoded readiness',async()=>{
    const f=await abortCallbackFixture();f.register();f.close();assert.equal(f.fire(),f.result);
    assert.equal(f.decoded.size,0);assert.equal(f.retired.length,1);assert.equal(f.retired[0].public_grade,false);assert.equal(f.invalid.length,0);
});
test('actual live Abort callbacks refuse missing/foreign/duplicate owner, changed MID, receiver or track while invoking original once',async()=>{
    for(const mode of['missing','foreign','duplicate','mid','receiver','track']){
        const f=await abortCallbackFixture();f.register();
        if(mode==='missing')f.peers.clear();
        if(mode==='foreign')f.peers.set('original-candidate',{...f.peer,getTransceivers:()=>[f.transceiver]});
        if(mode==='duplicate')f.peers.set('foreign-peer',f.peer);
        if(mode==='mid')f.transceiver.mid='9';
        if(mode==='receiver')f.transceiver.receiver={track:f.track};
        if(mode==='track')f.element.srcObject=new f.Stream([{...f.track}]);
        assert.equal(f.fire(),f.result);assert.equal(f.calls.length,1,mode);assert.equal(f.invalid.length,1,mode);
        assert.equal(f.decoded.size,0,mode);assert.equal(f.retired.length,0,mode);
    }
});
test('closed Abort callbacks cannot use initial null MID, prior history or unproven registration as retirement proof',async()=>{
    for(const history of[false,true]){
        const f=await abortCallbackFixture();if(history){f.register();f.fire();}
        f.transceiver.mid=null;f.register();f.close();assert.equal(f.fire(),f.result);
        assert.equal(f.invalid.length,1);assert.equal(f.retired.length,0);
        assert.equal(f.decoded.get(f.track)?.callbacks??0,history?1:0);
    }
});
test('exact closed callback exception refuses changed binding, live track, incomplete close or absent original candidate-close proof',async()=>{
    for(const mode of['mid','track-live','pc-open','foreign','track-replaced','no-stream','no-close-proof','foreign-close','removed']){
        const f=await abortCallbackFixture();f.register();f.close();
        if(mode==='mid')f.transceiver.mid='9';
        if(mode==='track-live')f.track.readyState='live';
        if(mode==='pc-open')f.peer.signalingState='stable';
        if(mode==='foreign')f.peers.set('original-candidate',{...f.peer,getTransceivers:()=>[f.transceiver]});
        if(mode==='track-replaced')f.transceiver.receiver.track={...f.track};
        if(mode==='no-stream')f.element.srcObject=null;
        if(mode==='no-close-proof')f.context.closeProof=null;
        if(mode==='foreign-close')f.context.closeProof.connection_id='foreign';
        if(mode==='removed')f.peer.getTransceivers=()=>[];
        assert.equal(f.fire(),f.result);assert.equal(f.invalid.length,1,mode);assert.equal(f.retired.length,0,mode);assert.equal(f.decoded.size,0,mode);
    }
});
test('MID drift or another element cannot replace prior live decoder ownership even immediately before close',async()=>{
    for(const mode of['live-mid','closed-mid','other-element']){
        const f=await abortCallbackFixture();f.register();f.fire();const original=f.decoded.get(f.track);
        if(mode==='other-element'){const second=new f.Element();f.register(second);}else{f.transceiver.mid='9';f.register();}
        if(mode!=='live-mid')f.close();
        assert.equal(f.fire(),f.result);assert.equal(f.invalid.length,1,mode);assert.equal(f.retired.length,0,mode);
        assert.equal(original.callbacks,1);assert.equal(original.last,10);
    }
});
test('ownership diagnostics preserve the SAME underlying callback exception after a legitimate queued close',async()=>{
    const f=await abortCallbackFixture(),original=Error('original-callback-failure');let calls=0;
    f.register(f.element,function(now,metadata){calls++;assert.equal(this,f.element);assert.equal(now,123);assert.equal(metadata,f.metadata);throw original;});
    f.close();assert.throws(()=>f.fire(),error=>error===original);assert.equal(calls,1);
    assert.equal(f.invalid.length,0);assert.equal(f.retired.length,1);assert.equal(f.decoded.size,0);
});
