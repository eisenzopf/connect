import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdtemp,readFile,rm,stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {nativeMatrixLifecycle} from '../lib/native_matrix_lifecycle.mjs';

test('terminal persists before close without waiting for a missing resource observer',async()=>{
    const directory=await mkdtemp(join(tmpdir(),'native-matrix-lifecycle-'));
    try{
        const lifecycle=nativeMatrixLifecycle(directory),started=performance.now();
        const result=await lifecycle.terminal();
        assert.equal(result.passed,true);assert.ok(performance.now()-started<500);
        const path=join(directory,'resources-terminal.json'),value=JSON.parse(await readFile(path,'utf8'));
        assert.equal(value.phase,'terminal');assert.ok(Number.isFinite(value.monotonic_seconds));
        assert.equal((await stat(path)).mode&0o077,0);
        await assert.rejects(readFile(join(directory,'resources-terminal-response.json')),{code:'ENOENT'});
    }finally{await rm(directory,{recursive:true});}
});
