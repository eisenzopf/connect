import {test} from 'node:test';
import assert from 'node:assert/strict';
import {rtcDecodedVideoOwners} from '../lib/browser_matrix_rtc_decoded.mjs';
import {DecodedRoute} from '../lib/browser_matrix_observation.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
import {readFile} from 'node:fs/promises';

const flush=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function environment(){
    const previous=globalThis.MediaStreamTrackProcessor,streams=[],clones=[],originals=[];
    globalThis.MediaStreamTrackProcessor=class{
        constructor(configuration){
            let controller;
            const row={configuration,cancelled:0,rejectCancel:false};
            this.readable=new ReadableStream({start(c){controller=c;},cancel(){row.cancelled++;if(row.rejectCancel)throw Error('private cancellation');}});
            row.enqueue=value=>controller.enqueue(value);streams.push(row);
        }
    };
    const peer=index=>{
        const cloned={stops:0,stop(){this.stops++;}},track={kind:'video',readyState:'live',stops:0,
            stop(){this.stops++;},clone(){clones.push(cloned);return cloned;}};
        originals.push(track);
        return {input:{connectionId:'connection-'+index,receiveVideo:'receive-'+index},remote:new Map([['receive-'+index,track]]),
            peer:{getReceivers:()=>[{track}]},track};
    };
    return {streams,clones,originals,peer,restore(){globalThis.MediaStreamTrackProcessor=previous;}};
}
function renderHandle(){
    let finish,retire;
    const finished=new Promise(resolve=>{finish=resolve;}),retirement=new Promise(resolve=>{retire=resolve;});
    return {finished,retirement,finish,retire};
}

test('native matrix clone owns exact received track and joins only after both original render futures',async()=>{
    const f=environment(),observed=[],invalid=[];
    const owner=rtcDecodedVideoOwners({observe:(...args)=>observed.push(args),invalid:(...args)=>invalid.push(args)});
    let borrowedClosed=0;const handle=renderHandle();
    try{
        const peer=f.peer(0),entry=owner.begin(peer,{video:{'receive-0':{}}});owner.bind(entry,Promise.resolve(handle));
        assert.equal(f.streams[0].configuration.track,f.clones[0]);assert.equal(f.streams[0].configuration.maxBufferSize,2);
        f.streams[0].enqueue({timestamp:123,displayWidth:640,displayHeight:360,close(){borrowedClosed++;}});
        await flush();assert.equal(observed.length,1);assert.equal(observed[0][0],'connection-0');assert.equal(observed[0][1],123);
        assert.equal(borrowedClosed,1);assert.equal(f.originals[0].stops,0);
        handle.finish();await flush();assert.equal(f.clones[0].stops,0,'finished alone is not original retirement');
        handle.retire();await flush();assert.equal(f.clones[0].stops,1);
        const closed=await owner.close();assert.equal(closed.joined,true);assert.equal(closed.retained_owners,0);
        assert.equal(closed.owners[0].frames,1);assert.equal(closed.owners[0].failed,false);
        assert.deepEqual(await owner.close(),closed);assert.equal(f.originals[0].stops,0);assert.equal(invalid.length,0);
    }finally{handle.finish();handle.retire();await owner.close();f.restore();}
});

test('native matrix rejects foreign track stream unsupported processor and owner overflow without borrowed authority',async()=>{
    const f=environment(),owner=rtcDecodedVideoOwners({observe(){},invalid(){}});
    try{
        const wrong=f.peer('wrong');wrong.peer.getReceivers=()=>[{track:{kind:'video',readyState:'live'}}];
        assert.throws(()=>owner.begin(wrong,{video:{'receive-wrong':{}}}));assert.equal(f.clones.length,0);
        const exact=f.peer('exact');assert.throws(()=>owner.begin(exact,{video:{foreign:{}}}));assert.equal(f.clones.length,0);
        const processor=globalThis.MediaStreamTrackProcessor;globalThis.MediaStreamTrackProcessor=undefined;
        assert.throws(()=>owner.begin(exact,{video:{'receive-exact':{}}}));assert.equal(f.clones.length,0);
        globalThis.MediaStreamTrackProcessor=processor;
        for(let i=0;i<10;i++){const peer=f.peer(i);owner.begin(peer,{video:{['receive-'+i]:{}}});}
        const extra=f.peer(10);assert.throws(()=>owner.begin(extra,{video:{'receive-10':{}}}));assert.equal(f.clones.length,10);
        const closed=await owner.close();assert.equal(closed.joined,true);assert.equal(closed.passed,false,'zero decoded frames cannot qualify');
        assert.equal(closed.retained_owners,0);assert.ok(f.clones.every(clone=>clone.stops===1));assert.ok(f.originals.every(track=>track.stops===0));
    }finally{await owner.close();f.restore();}
});

test('native matrix startup pixel and reader-close failures retain every original clone join and fail qualification',async()=>{
    for(const failure of ['startup','pixels','cancel']){
        const f=environment(),errors=[],handles=[];
        const owner=rtcDecodedVideoOwners({observe(){if(failure==='pixels')throw Error('private pixels');},invalid:(...args)=>errors.push(args)});
        let borrowedClosed=0;
        try{
            for(let i=0;i<6;i++){
                const peer=f.peer(i),entry=owner.begin(peer,{video:{['receive-'+i]:{}}}),handle=renderHandle();handles.push(handle);
                owner.bind(entry,failure==='startup'&&i===0?Promise.reject(Error('private startup')):Promise.resolve(handle));
                f.streams[i].enqueue({timestamp:i+1,displayWidth:640,displayHeight:360,close(){borrowedClosed++;}});
            }
            await flush();assert.equal(borrowedClosed,6,'every actual borrowed frame must close, including failing observation');
            if(failure==='cancel')f.streams[0].rejectCancel=true;
            for(const handle of handles){handle.finish();handle.retire();}
            const closed=await owner.close();assert.equal(closed.joined,true);assert.equal(closed.passed,false);
            assert.equal(closed.retained_owners,0);assert.ok(closed.owners.some(row=>row.failed));assert.ok(errors.length>0);
            assert.ok(f.clones.every(clone=>clone.stops===1));assert.ok(f.originals.every(track=>track.stops===0));
            assert.equal(JSON.stringify(closed).includes('private'),false);
        }finally{for(const handle of handles){handle.finish();handle.retire();}await owner.close();f.restore();}
    }
});

function tone(frequency,amplitude,start){return Float32Array.from({length:960},(_,i)=>amplitude*Math.sin(2*Math.PI*frequency*(start+i)/48000));}
function dualTimeline({wrongRendered=false,noDecoded=false}={}){
    const decoded=new DecodedRoute(0,1),rendered=new DecodedRoute(0,1),pulses=markerFrames().map(frame=>frame/30);
    decoded.start(1000);rendered.start(1000);
    const high=at=>pulses.some(start=>at>=start&&at<start+.1);
    for(let i=0;i<3100;i++)decoded.audioFrame('original',i*20000,1000+i*20+200,tone(660,high(i*.02)?.2:.05,i*960));
    for(let i=0;i<1860;i++){
        const at=i/30,gray=Array(64).fill(high(at)?255:100);
        if(!high(at)){gray[0]=200;gray[1]=40;gray[8]=40;gray[9]=200;}
        if(!noDecoded)decoded.videoFrame('original',Math.round(at*1e6),1000+at*1000+10,gray,new Uint8Array([i%256]),640,360);
        // Real presentation counters can jump; they grant ONE callback, never
        // the difference as invented frame credit. Keep all marker pulses.
        if([3,7,11,14].includes(i%15))continue;
        const actual=gray.slice();if(wrongRendered&&i===100){actual[0]=40;actual[1]=40;actual[8]=200;actual[9]=200;}
        rendered.videoFrame('original',Math.round(at*1e6),1000+at*1000+200,actual,new Uint8Array([i%256]),640,360,null,
            {profile:'connect/rtc-video-presentation/1',element_id:1,presentedFrames:i+1});
    }
    return {decoded,rendered,report:decoded.report(()=>true,rendered)};
}
test('native decoded pixels qualify FPS while independently rendered pixels alone supply AV onset and identity',()=>{
    const {report}=dualTimeline();
    assert.equal(report.video.observer_profile,'native-decoded-frames/1');assert.equal(report.video.minimum_fps,30);
    assert.equal(report.video.rendered_observation.observed_frames,1320);assert.equal(report.video.rendered_observation.identity.status,'passed');
    assert.equal(report.sync.status,'passed','200ms early decoded arrival must NOT replace rendered AV onset');
    assert.equal(report.video.passed,true);assert.equal(report.passed,true);
    assert.equal(dualTimeline({wrongRendered:true}).report.video.passed,false,'decoded identity cannot excuse wrong rendered pixels');
    assert.equal(dualTimeline({noDecoded:true}).report.video.passed,false,'rendered counters cannot supply missing decoded pixels');
    const valid=dualTimeline();valid.rendered.invalid.push('actual-render-refusal');
    assert.equal(valid.decoded.report(()=>true,valid.rendered).video.passed,false);
});

test('native decoded helper has closed matrix-only asset routes and shared RTC callback consumers keep no new static dependency',async()=>{
    const observer=await readFile(new URL('../lib/browser_matrix_observation.mjs',import.meta.url),'utf8');
    const helper=await readFile(new URL('../lib/browser_matrix_rtc_decoded.mjs',import.meta.url),'utf8');
    const shared=await readFile(new URL('../lib/browser_matrix_rtc_observation.mjs',import.meta.url),'utf8');
    const server=await readFile(new URL('../video/browser_matrix_admission.mjs',import.meta.url),'utf8');
    const imports=source=>[...source.matchAll(/\bfrom\s+["']([^"']+)["']/g)].map(match=>match[1]);
    const main=imports(observer).find(value=>value.endsWith('/browser_matrix_rtc_decoded.mjs'));assert.ok(main);
    assert.ok(server.includes("url.pathname==='"+new URL(main,'https://fixture.invalid/_matrix/observation.mjs').pathname+"'"));
    const borrowed=imports(helper).find(value=>value.endsWith('/decoded_video_frames.mjs'));assert.ok(borrowed);
    assert.ok(server.includes("url.pathname==='"+new URL(borrowed,'https://fixture.invalid/_matrix/browser_matrix_rtc_decoded.mjs').pathname+"'"));
    assert.equal(imports(shared).some(value=>value.includes('rtc_decoded')||value.includes('decoded_video_frames')),false);
    assert.ok(server.includes("rtcVideoObserver:spec.browserName==='chrome'?'native-decoded-frames/1':'render-callback/1'"));
});
