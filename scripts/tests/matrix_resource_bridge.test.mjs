import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {matrixResourceBridge} from '../lib/matrix_resource_bridge.mjs';

test('resource bridge retains original terminal time instead of restarting the idle window after cleanup',async()=>{
    const directory=await fs.mkdtemp(path.join(os.tmpdir(),'matrix-resource-'));
    try{
        for(const phase of ['baseline','terminal','retired'])await fs.writeFile(path.join(directory,'resources-'+phase+'-response.json'),'{"passed":true}');
        const send=matrixResourceBridge(directory),baseline=await send('baseline');
        assert.ok(Number.isFinite(baseline.monotonic_seconds));
        const terminal=baseline.monotonic_seconds-.5;
        await send('terminal',terminal);await send('retired');
        const recorded=JSON.parse(await fs.readFile(path.join(directory,'resources-terminal.json'),'utf8'));
        assert.equal(recorded.monotonic_seconds,terminal);
        assert.equal(recorded.clock_basis,process.platform==='darwin'?'macos-mach-continuous':'linux-clock-monotonic');
        await assert.rejects(send('terminal',terminal),/phase order/);
        const refusedFolder=await fs.mkdtemp(path.join(directory,'refused-'));
        await fs.writeFile(path.join(refusedFolder,'resources-baseline-response.json'),
            JSON.stringify({passed:false,failure:'original-owned-resources-not-restored'}));
        const refused=matrixResourceBridge(refusedFolder);
        let authenticated=0,closed=0;
        const originalOwner=async()=>{try{
            // Same awaited Node bridge as browser_matrix_admission before auth.
            await refused('baseline');authenticated++;
        }finally{closed++;}};
        await assert.rejects(originalOwner(),/Original resource baseline refused/);
        assert.equal(authenticated,0);assert.equal(closed,1);
        const baselineRequest=JSON.parse(await fs.readFile(path.join(refusedFolder,'resources-baseline.json'),'utf8'));
        assert.equal(baselineRequest.phase,'baseline');
        await assert.rejects(fs.stat(path.join(refusedFolder,'resources-terminal.json')),error=>error.code==='ENOENT');
        await assert.rejects(fs.stat(path.join(refusedFolder,'resources-retired.json')),error=>error.code==='ENOENT');
        await assert.rejects(refused('baseline'),/phase order/);
    }finally{await fs.rm(directory,{recursive:true,force:true});}
});

test('three resource cycles keep independent original terminal requests and refuse replay or skipping',async()=>{
    const directory=await fs.mkdtemp(path.join(os.tmpdir(),'matrix-cycle-resource-'));
    try{
        const send=matrixResourceBridge(directory,{cycles:3});
        await assert.rejects(send('baseline',undefined,2),/phase order/);
        for(let index=1;index<=3;index++){
            const folder=path.join(directory,'cycles',`cycle-${index}`);await fs.mkdir(folder,{recursive:true});
            for(const phase of ['baseline','terminal','retired'])await fs.writeFile(path.join(folder,'resources-'+phase+'-response.json'),'{"passed":true}');
            const baseline=await send('baseline',undefined,index),terminal=baseline.monotonic_seconds-.25;
            await send('terminal',terminal,index);await send('retired',undefined,index);
            assert.equal(JSON.parse(await fs.readFile(path.join(folder,'resources-terminal.json'),'utf8')).monotonic_seconds,terminal);
            await assert.rejects(send('baseline',undefined,index),/phase order/);
        }
        await assert.rejects(send('baseline',undefined,4),/phase order/);
    }finally{await fs.rm(directory,{recursive:true,force:true});}
});

test('resource bridge declares its original clock and refuses invalid terminal without publishing it',async()=>{
    const directory=await fs.mkdtemp(path.join(os.tmpdir(),'matrix-resource-clock-'));
    try{
        await fs.writeFile(path.join(directory,'resources-baseline-response.json'),'{"passed":true}');
        const send=matrixResourceBridge(directory),baseline=await send('baseline');
        const request=JSON.parse(await fs.readFile(path.join(directory,'resources-baseline.json'),'utf8'));
        assert.equal(request.clock_basis,process.platform==='darwin'?'macos-mach-continuous':'linux-clock-monotonic');
        assert.ok(Number.isFinite(request.monotonic_seconds));
        assert.ok(request.monotonic_seconds<=baseline.monotonic_seconds);
        // Each refused terminal uses a fresh original bridge: no retry or phase
        // reset is introduced after an invalid original request.
        for(const value of[undefined,NaN,Infinity,baseline.monotonic_seconds+100,baseline.monotonic_seconds-31]){
            const folder=await fs.mkdtemp(path.join(directory,'case-'));
            await fs.writeFile(path.join(folder,'resources-baseline-response.json'),'{"passed":true}');
            const original=matrixResourceBridge(folder);await original('baseline');
            await assert.rejects(original('terminal',value),/Original terminal clock required/);
            await assert.rejects(fs.stat(path.join(folder,'resources-terminal.json')),error=>error.code==='ENOENT');
        }
    }finally{await fs.rm(directory,{recursive:true,force:true});}
});
