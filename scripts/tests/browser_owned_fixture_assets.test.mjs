import assert from 'node:assert/strict';
import {test} from 'node:test';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import vm from 'node:vm';
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
