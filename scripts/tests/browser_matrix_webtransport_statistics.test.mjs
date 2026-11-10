import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {browserMatrixWebTransportStatistics} from '../lib/browser_matrix_webtransport_statistics.mjs';
import {browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';

const flush=async()=>{for(let i=0;i<30;i++)await Promise.resolve();};
function fixture(){
    const instances=[];
    class Transport{
        constructor(...args){if(args[0]==='throw')throw failure;this.args=args;this.newTarget=new.target;
            this.ready=Promise.resolve();this.closed=new Promise(resolve=>{this.finish=resolve;});this.reads=0;
            this.value={timestamp:123,datagrams:{droppedIncoming:0,expiredIncoming:1,expiredOutgoing:2,lostOutgoing:3}};instances.push(this);}
        getStats(){this.reads++;return Promise.resolve(this.value);}
        get datagrams(){throw Error('Diagnostics must not borrow streams');}
        close(){throw Error('Diagnostics must not close original sessions');}
    }
    const failure=Error('original-constructor-failure'),scope={};
    Object.defineProperty(scope,'WebTransport',{value:Transport,writable:true,configurable:true,enumerable:false});
    return {scope,Transport,instances,failure};
}
async function withTimer(work){
    const previous=[globalThis.setInterval,globalThis.clearInterval];let callback,cleared=0;
    globalThis.setInterval=fn=>{callback=fn;return 1;};globalThis.clearInterval=()=>{cleared++;};
    try{return await work({tick:()=>callback?.(),cleared:()=>cleared});}
    finally{[globalThis.setInterval,globalThis.clearInterval]=previous;}
}

test('WT scoped original construction preserves identity/options/descriptor and excludes unrelated connections',async()=>{
    const f=fixture(),before=Object.getOwnPropertyDescriptor(f.scope,'WebTransport'),stats=browserMatrixWebTransportStatistics(f.scope);
    new f.scope.WebTransport('wrong-pin');const options={allowPooling:false};
    const actual=await stats.capture('participant-1',()=>new f.scope.WebTransport('original',options));
    assert.equal(actual,f.instances[1]);assert.equal(Object.getPrototypeOf(actual),f.Transport.prototype);
    assert.equal(actual.newTarget,f.Transport);assert.equal(actual.args[1],options);
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.scope,'WebTransport'),before);
    new f.scope.WebTransport('selection-helper');
    assert.deepEqual(stats.report().owners.map(row=>row.actor),['participant-1']);
    await assert.rejects(stats.capture('participant-2',()=>new f.scope.WebTransport('throw')),error=>error===f.failure);
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.scope,'WebTransport'),before);
    const rejected=Error('original-connect-rejection');
    await assert.rejects(stats.capture('participant-3',async()=>{new f.scope.WebTransport('failed-original');throw rejected;}),error=>error===rejected);
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.scope,'WebTransport'),before);
    assert.equal(stats.report().owners.length,2);assert.equal((await stats.close()).joined,true);
    assert.ok(stats.report().owners.every(row=>!row.live_owner_retained));
});

test('WT unavailable interception preserves original work with readonly and throwing global properties',async()=>{
    for(const kind of ['readonly','getter-only','throwing-accessor']){
        const f=fixture(),scope={},failure=Error('original-global-getter'),sentinel={original:true};let calls=0;
        Object.defineProperty(scope,'WebTransport',kind==='readonly'
            ?{value:f.Transport,writable:false,configurable:false}
            :{get:()=>{if(kind==='throwing-accessor')throw failure;return f.Transport;},configurable:false});
        const descriptor=Object.getOwnPropertyDescriptor(scope,'WebTransport'),stats=browserMatrixWebTransportStatistics(scope);
        const value=await stats.capture('participant-1',()=>{calls++;return kind==='throwing-accessor'?sentinel:new scope.WebTransport('original');});
        assert.equal(calls,1);assert.equal(kind==='throwing-accessor'?value===sentinel:value instanceof f.Transport,true);
        assert.deepEqual(Object.getOwnPropertyDescriptor(scope,'WebTransport'),descriptor);
        assert.equal(stats.report().owners.length,0);assert.equal(stats.report().interception_unavailable.length,1);
        assert.equal(stats.report().interception_unavailable[0].reason,kind==='readonly'?'constructor-interception-unavailable':'constructor-not-own-data-property');
        if(kind==='throwing-accessor')await assert.rejects(stats.capture('participant-2',()=>new scope.WebTransport()),error=>error===failure);
        assert.equal((await stats.close()).joined,true,'Unavailable optional observation is not an unjoined owner');
    }
});

test('WT foreign concurrent constructor replacement remains intact and cannot claim exact restoration',async()=>{
    for(const accessor of [false,true]){
        const f=fixture(),stats=browserMatrixWebTransportStatistics(f.scope);class Foreign{}
        let reads=0;const getter=()=>{reads++;throw Error('foreign getter must not run');};
        const actual=await stats.capture('participant-1',async()=>{
            const owner=new f.scope.WebTransport('original');
            if(accessor)Object.defineProperty(f.scope,'WebTransport',{get:getter,configurable:true});
            else f.scope.WebTransport=Foreign;
            return owner;
        });
        assert.equal(actual,f.instances[0]);assert.equal(reads,0);
        assert.equal(Object.getOwnPropertyDescriptor(f.scope,'WebTransport')[accessor?'get':'value'],accessor?getter:Foreign);
        assert.equal(stats.report().constructor_restoration_failures,1);
        const receipt=await stats.close();assert.equal(receipt.constructor_restored,false);assert.equal(receipt.joined,false);
        assert.equal(reads,0);
        assert.equal(Object.getOwnPropertyDescriptor(f.scope,'WebTransport')[accessor?'get':'value'],accessor?getter:Foreign);
    }
});

test('WT diagnostics do not consume stateful own or inherited constructor getters',async()=>{
    for(const inherited of [false,true]){
        const f=fixture(),holder={},failure=Error('first original getter failure');let reads=0,calls=0;
        Object.defineProperty(holder,'WebTransport',{configurable:true,get(){
            reads++;if(reads===1)throw failure;return f.Transport;
        }});
        const scope=inherited?Object.create(holder):holder,stats=browserMatrixWebTransportStatistics(scope);
        await assert.rejects(stats.capture('participant-1',()=>{calls++;return new scope.WebTransport('first');}),error=>error===failure);
        assert.equal(reads,1);assert.equal(calls,1);
        const actual=await stats.capture('participant-1',()=>{calls++;return new scope.WebTransport('second');});
        assert.ok(actual instanceof f.Transport);assert.equal(reads,2);assert.equal(calls,2);
        assert.equal(stats.report().owners.length,0);assert.equal(stats.report().interception_unavailable.length,2);
        assert.equal((await stats.close()).joined,true);assert.equal(reads,2);
    }
});

test('WT statistics preserve explicit unknown fields and bound physical owners without altering original construction',async()=>withTimer(async()=>{
    const f=fixture(),stats=browserMatrixWebTransportStatistics(f.scope);
    await stats.capture('participant-1',()=>{for(let i=0;i<20;i++)new f.scope.WebTransport('original');});
    f.instances[0].getStats=undefined;
    f.instances[1].value={datagrams:{droppedIncoming:0,expiredIncoming:Number.MAX_SAFE_INTEGER+1}};
    f.instances[2].getStats=()=>Promise.reject(Error('private-error-not-retained'));
    Object.defineProperty(f.instances[3],'getStats',{get(){throw Error('unavailable');}});
    await flush();stats.start(performance.now());await flush();
    const report=stats.report(),rows=report.samples[0].transports;
    assert.equal(f.instances.length,20);assert.equal(report.owners.length,16);assert.equal(report.omitted_owners,4);
    assert.equal(rows[0].unavailable,'getStats-unavailable');assert.equal(rows[0].datagrams.droppedIncoming,null);
    assert.deepEqual(rows[1].datagrams,{droppedIncoming:0,expiredIncoming:null,expiredOutgoing:null,lostOutgoing:null});
    assert.equal(rows[2].unavailable,'getStats-rejected-or-unreadable');assert.equal(rows[3].unavailable,'getStats-rejected-or-unreadable');
    assert.deepEqual(rows[4].datagrams,{droppedIncoming:0,expiredIncoming:1,expiredOutgoing:2,lostOutgoing:3});
    assert.equal(JSON.stringify(report).includes('private-error'),false);
    report.samples[0].transports[4].datagrams.lostOutgoing=999;
    assert.equal(stats.report().samples[0].transports[4].datagrams.lostOutgoing,3);
    assert.equal((await stats.close()).joined,true);
}));

test('WT sampler permits only one original batch and at most64 attempts',async()=>withTimer(async timer=>{
    const f=fixture(),stats=browserMatrixWebTransportStatistics(f.scope);let resolve,calls=0;
    const owner=await stats.capture('participant-1',()=>new f.scope.WebTransport('original'));
    owner.getStats=()=>{calls++;return new Promise(done=>{resolve=done;});};
    await flush();stats.start(performance.now());await flush();timer.tick();timer.tick();await flush();assert.equal(calls,1);
    resolve(owner.value);await flush();owner.getStats=async()=>{calls++;return owner.value;};
    for(let i=0;i<80;i++){timer.tick();await flush();}
    assert.equal(calls,64);assert.equal(stats.report().attempts,64);assert.equal(stats.report().samples.length,64);
    assert.ok(timer.cleared()>0);const first=stats.close();assert.equal(stats.close(),first);assert.equal((await first).joined,true);
    timer.tick();await flush();assert.equal(calls,64);
}));

test('WT original pending statistics is not a fake join and cannot rewrite frozen retirement evidence',async()=>withTimer(async()=>{
    const f=fixture(),stats=browserMatrixWebTransportStatistics(f.scope);let resolve;
    const owner=await stats.capture('participant-1',()=>new f.scope.WebTransport('original'));
    owner.getStats=()=>new Promise(done=>{resolve=done;});
    class Renderer{audio(){}video(){}}
    const observer=browserMatrixObservation(Renderer,{routes:[]},null,null,{webTransportStatistics:stats});
    observer.start({originMs:performance.now()});await flush();
    assert.equal(observer.report().passed,true,'Diagnostic pending state does not alter media predicates');
    const close=observer.close(),receipt=await close;
    assert.equal(receipt.joined,false);assert.equal(receipt.webtransport_statistics.outstanding_original_snapshot,true);
    const frozen=stats.report();assert.equal(frozen.pending,true);assert.ok(frozen.owners.every(row=>!row.live_owner_retained));
    resolve(owner.value);await flush();
    assert.deepEqual(stats.report(),frozen);assert.equal(await observer.close(),receipt);
}));

test('WT closed original owner releases references and absent API leaves media quality unchanged',async()=>withTimer(async()=>{
    const f=fixture(),stats=browserMatrixWebTransportStatistics(f.scope);
    const owner=await stats.capture('participant-1',()=>new f.scope.WebTransport('original'));owner.getStats=undefined;
    class Renderer{audio(){}video(){}}
    const observer=browserMatrixObservation(Renderer,{routes:[]},null,null,{webTransportStatistics:stats});
    const before=observer.report();observer.start({originMs:performance.now()});await flush();
    assert.equal(observer.report().passed,before.passed);assert.deepEqual(observer.report().routes,before.routes);
    owner.finish();await flush();assert.equal(stats.report().owners[0].closed,true);
    assert.equal(stats.report().owners[0].live_owner_retained,false);
    assert.equal((await observer.close()).joined,true);
}));

test('WT fixture helper and its reused scheduler have actual browser routes',async()=>{
    const server=await readFile(new URL('../video/browser_matrix_admission.mjs',import.meta.url),'utf8');
    for(const name of ['browser_matrix_webtransport_statistics.mjs','browser_matrix_stages.mjs']){
        assert.ok(server.includes("url.pathname==='/_matrix/"+name+"'"));
        assert.ok(server.includes("path.join(root,'scripts/lib/"+name+"')"));
    }
    assert.ok(server.includes('webTransportStatistics.capture(name,connect)'));
    assert.ok(server.indexOf('Wrong WT certificate pin was not refused and joined')<server.indexOf('webTransportStatistics.capture(name,connect)'));
});
