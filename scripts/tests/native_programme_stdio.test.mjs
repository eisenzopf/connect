import assert from 'node:assert/strict';
import {test} from 'node:test';
import {EventEmitter} from 'node:events';
import {mkdtempSync,openSync,closeSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {ownWorkerStdio} from '../lib/native_programme_profile.mjs';

async function fixture(run,options){
    const root=mkdtempSync(join(tmpdir(),'native-original-stdio-')),path=join(root,'original.log'),fd=openSync(path,'wx',0o600),stream=new EventEmitter();
    try{await run({stream,owner:ownWorkerStdio(stream,fd,options),read:()=>readFileSync(path)});}
    finally{closeSync(fd);rmSync(root,{recursive:true,force:true});}
}

test('only the original ended error-free stdio releases its reference after exact synchronous writes',()=>fixture(async f=>{
    const original=f.owner.joined;assert.equal(f.owner.release(),false);
    assert.equal(f.owner.receipt().reference_released,false);assert.equal(f.stream.listenerCount('data'),1);
    f.stream.emit('data',Buffer.from('original'));assert.equal(f.owner.release(),false);
    assert.equal(f.owner.receipt().bytes,8);assert.equal(f.owner.receipt().ended,false);
    f.stream.emit('end');await original;
    assert.equal(f.owner.release(),true);assert.equal(f.owner.release(),true,'Retired reference release is idempotent');
    assert.equal(f.owner.joined,original,'Do not replace the actual original join');
    assert.deepEqual(f.owner.receipt(),{bytes:8,ended:true,failure:null,reference_released:true});
    assert.equal(f.read().toString(),'original');assert.equal(f.stream.listenerCount('data'),0);assert.equal(f.stream.listenerCount('error'),0);
    assert.equal(Object.values(f.owner.receipt()).includes(f.stream),false,'Receipt contains only original numeric/failure facts');
}));

test('original incomplete-close or stream error keeps failed reference proof and original counters',async()=>{
    for(const failed of ['incomplete','error'])await fixture(async f=>{
        f.stream.emit('data',Buffer.from('original'));
        if(failed==='incomplete')f.stream.emit('close');else{f.stream.emit('error',Error('Original stream failure'));f.stream.emit('end');}
        await f.owner.joined;assert.equal(f.owner.release(),false);assert.equal(f.owner.receipt().reference_released,false);
        assert.equal(f.owner.receipt().failure,failed==='incomplete'?'worker-stdio-incomplete':'worker-stdio-error');
        assert.equal(f.owner.receipt().bytes,8);assert.equal(f.read().toString(),'original');
        assert.equal(f.stream.listenerCount('error'),1,'Retain original failed stream ownership/listener');
    });
});

test('nonbuffer and original byte-bound failures cannot become released or erase failed output facts',async()=>{
    for(const failed of ['nonbuffer','bound'])await fixture(async f=>{
        f.stream.emit('data',failed==='nonbuffer'?'unexpected':Buffer.from('too-large'));
        f.stream.emit('end');await f.owner.joined;
        assert.equal(f.owner.release(),false);assert.equal(f.owner.receipt().reference_released,false);
        assert.equal(f.owner.receipt().failure,failed==='nonbuffer'?'non-buffer-worker-stdio':'worker-stdio-bound');
        assert.equal(f.owner.receipt().bytes,failed==='nonbuffer'?0:9);assert.equal(f.read().length,0);
    },{limit:1});
});
