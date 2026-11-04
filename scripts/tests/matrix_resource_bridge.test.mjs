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
        await assert.rejects(send('terminal',terminal),/phase order/);
    }finally{await fs.rm(directory,{recursive:true,force:true});}
});
