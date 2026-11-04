import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
const dist=process.env.CONNECT_GATE6_TS_DIST;
assert.ok(dist && dist.startsWith('/'), 'Use the runner isolated TypeScript build');
const load=name=>import(pathToFileURL(join(dist,name)).href);
const {NodeClient}=await load('index.js');
const {SdkRtpPacket,captureRtpVideoEpoch}=await load('node_media_types.js');
const {captureVideoEpoch,captureVideoFeedback,takeVideoFeedback,consumedVideoFeedback}=await load('node_video_feedback.js');
const scope={conversationId:'room',sessionId:'session',connectionId:'connection'};
const wire={conversation_id:'room',session_id:'session',connection_id:'connection'};
const epoch={epoch:'b'.repeat(32),incarnation:'c'.repeat(32),revision:2,ssrc:83,first_sequence:100,first_timestamp:3000};
test('video IPC history and one-shot references refuse forged metadata and delayed lifetime renewal',()=>{
    const captured=captureVideoEpoch(epoch);assert.ok(Object.isFrozen(captured));
    const rtp=Buffer.from([0x80,96,0,100,0,0,11,184,0,0,0,83,0x10,0x31]);
    const packet=new SdkRtpPacket(100,rtp);captureRtpVideoEpoch(packet,epoch);
    assert.equal(packet.videoEpoch.epoch,epoch.epoch);assert.deepEqual(Buffer.from(packet.bytes),rtp);
    for (const [field,value] of [['epoch','0'.repeat(32)],['incarnation','C'.repeat(32)],['revision',0],['ssrc',0],['first_sequence',2**32],['extra',true]])
        assert.throws(()=>captureVideoEpoch({...epoch,[field]:value}));
    assert.throws(()=>captureRtpVideoEpoch(new SdkRtpPacket(100,rtp),{...epoch,ssrc:84}));
    assert.throws(()=>captureRtpVideoEpoch(new SdkRtpPacket(99,rtp),epoch));
    const owner={},frame={kind:0x81,id:2,payload:Buffer.alloc(0),metadata:{type:'video_feedback',binding:wire,stream_id:'camera',token:'a'.repeat(32),datagram_sequence:7,ssrc:83,epoch:epoch.epoch,epoch_revision:2}};
    const hint=captureVideoFeedback(owner,scope,'camera',frame);
    assert.throws(()=>takeVideoFeedback({},scope,'camera',hint));
    assert.throws(()=>takeVideoFeedback(owner,scope,'other',hint));
    assert.throws(()=>takeVideoFeedback(owner,scope,'camera',{...hint}));
    const token=takeVideoFeedback(owner,scope,'camera',hint);assert.throws(()=>takeVideoFeedback(owner,scope,'camera',hint));
    const reply={kind:0x81,id:3,payload:Buffer.alloc(0),metadata:{type:'video_feedback_consumed',binding:wire,stream_id:'camera',token,remaining_ms:50}};
    assert.equal(consumedVideoFeedback(scope,'camera',hint,token,performance.now()-60,reply),undefined);
    const permit=consumedVideoFeedback(scope,'camera',hint,token,performance.now(),reply);
    assert.ok(permit.isFresh());assert.ok(permit.expiresInMs<=50);
    assert.throws(()=>consumedVideoFeedback(scope,'camera',hint,token,performance.now(),{...reply,metadata:{...reply.metadata,remaining_ms:201}}));
});

test('public Node media feedback traverses owned IPC and discards delayed consumption replies',async()=>{
    const base=resolve('.local/gate-6/video-feedback-ipc-node');await mkdir(base,{recursive:true,mode:0o700});
    const helper=fileURLToPath(new URL('./fixtures/sdk_media_helper.py',import.meta.url));
    for (const mode of ['feedback','feedback-delayed']) {
        const dir=await mkdtemp(join(base,'case-')),config=join(dir,'config.json');
        await writeFile(config,JSON.stringify({mode,pids:join(dir,'pids'),capture:join(dir,'capture'),release:join(dir,'release')}));
        const client=new NodeClient({config,executable:helper});
        try {
            await client.capabilities.refresh({timeoutMs:5000});
            const stream=client.media.connection(scope).streamById('camera');
            assert.equal((await stream.requestKeyframe({timeoutMs:1000})).datagramSequence,7);
            const packet=await stream.receivePacket({timeoutMs:1000});assert.equal(packet.videoEpoch.epoch,'b'.repeat(32));
            const hint=await stream.receiveVideoFeedback({timeoutMs:1000});
            assert.throws(()=>stream.consumeVideoFeedback({...hint},{timeoutMs:1000}));
            const permit=await stream.consumeVideoFeedback(hint,{timeoutMs:1000});
            if (mode==='feedback') assert.ok(permit?.isFresh());else assert.equal(permit,undefined);
            assert.throws(()=>stream.consumeVideoFeedback(hint,{timeoutMs:1000}));
            const calls=(await readFile(join(dir,'capture'),'utf8')).trim().split('\n').map(JSON.parse);
            assert.equal(calls.filter(v=>v.channel==='media' && v.request.command.action==='consume_video_feedback').length,1);
        } finally {
            await client.close({timeoutMs:2000});
            for(const pid of (await readFile(join(dir,'pids'),'utf8')).trim().split('\n')) assert.throws(()=>process.kill(Number(pid),0),{code:'ESRCH'});
            await rm(dir,{recursive:true,force:true});
        }
    }
});
