import assert from "node:assert/strict";
import {test} from "node:test";
import {readFile} from "node:fs/promises";
import {matrixSdkAssetName} from "../lib/browser_matrix_sdk_asset.mjs";

test("owned matrix resolves the original observation SDK import and its actual dependency URLs",async()=>{
    const source=await readFile(new URL("../lib/browser_matrix_observation.mjs",import.meta.url),"utf8");
    const imports=[...source.matchAll(/\bfrom\s+["']([^"']+)["']/g)].map(match=>match[1]);
    const entry=imports.find(value=>value.endsWith("/browser_native_activation.js"));
    assert.ok(entry,"Use the actual unchanged observation import");
    const origin="https://fixture.invalid",url=new URL(entry,origin+"/_matrix/observation.mjs");
    assert.equal(url.pathname,"/sdk/typescript/dist/browser_native_activation.js");
    const pending=[url],seen=new Map();
    while(pending.length){
        const next=pending.shift();assert.equal(next.origin,origin);
        const name=matrixSdkAssetName(next.pathname);if(seen.has(name))continue;
        assert.ok(seen.size<32,"Bounded original SDK dependency graph");
        const bytes=await readFile(new URL("../../sdk/typescript/dist/"+name,import.meta.url),"utf8");
        seen.set(name,next.pathname);
        for(const match of bytes.matchAll(/\bfrom\s+["']([^"']+)["']/g)){
            assert.ok(match[1].startsWith("./"),"Original native-cut dependency remains a local built module");
            pending.push(new URL(match[1],next));
        }
    }
    for(const required of ["browser_native_activation.js","errors.js","uctp_json.js"])
        assert.ok(seen.has(required),"Actual dependency URL was resolved: "+required);
    for(const [name,nested] of seen){
        assert.equal(matrixSdkAssetName(nested),name);
        assert.equal(matrixSdkAssetName("/sdk/"+name),name);
    }
});

test("matrix SDK aliases reject directories traversal encoding and non-JavaScript inputs",()=>{
    assert.equal(matrixSdkAssetName("/sdk/browser.js"),"browser.js");
    for(const value of [null,"/sdk/typescript/src/browser.js","/sdk/typescript/dist/sub/errors.js",
        "/sdk/../errors.js","/sdk/typescript/dist/../errors.js","/sdk/%2e%2e/errors.js",
        "/sdk/typescript/dist/%2ferrors.js","/sdk/errors.js?x=1","/sdk/errors.js#x",
        "/sdk/errors.d.ts","/sdk/Errors.js","/sdk/\\errors.js","/other/errors.js"])
        assert.throws(()=>matrixSdkAssetName(value),TypeError);
});

test('browser fixture serves the actual transition coverage import at its resolved module URL',async()=>{
    const observer=await readFile(new URL('../lib/browser_matrix_observation.mjs',import.meta.url),'utf8');
    const entry=[...observer.matchAll(/\bfrom\s+["']([^"']+)["']/g)]
        .map(match=>match[1]).find(name=>name.endsWith('/media_transition_coverage.mjs'));
    assert.ok(entry);
    const url=new URL(entry,'https://fixture.invalid/_matrix/observation.mjs');
    const server=await readFile(new URL('../video/browser_matrix_admission.mjs',import.meta.url),'utf8');
    assert.ok(server.includes("url.pathname==='"+url.pathname+"'"));
    assert.ok(server.includes("path.join(root,'scripts/lib/media_transition_coverage.mjs')"));
    const module=await import('../lib/media_transition_coverage.mjs');
    assert.equal(module.TRANSITION_PROFILE,'connect/media-transition-coverage/1');
});
