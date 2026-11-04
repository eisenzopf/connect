// Actual VP8 encoder/packetizer/assembler and independent decoder. Feedback
// authority is controlled here; signed SDK/server transport is a separate proof.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { RtpSender } from '../../sdk/typescript/dist/browser.js';
import { SdkRtpPacket } from '../../sdk/typescript/dist/index.js';
import { LiveVp8Encoder } from '../lib/live_vp8_encoder.mjs';
import { PublisherVideoFeedback, VideoReceiveHistory } from '../lib/transport_video_recovery.mjs';
import { SwitchingMediaReader } from '../lib/transport_media_reader.mjs';
const env = { PATH: process.env.PATH, HOME: process.env.HOME, LANG: 'C', PYTHONDONTWRITEBYTECODE: '1' };
const tick = () => new Promise(setImmediate);
const stopped = () => Object.assign(Error('owned feedback cancelled'), { kind: 'cancelled', certainty: 'read-only' });

async function decode(directory, name, frames) {
    const header = Buffer.alloc(32); header.write('DKIF'); header.writeUInt16LE(32, 6); header.write('VP80', 8);
    header.writeUInt16LE(640, 12); header.writeUInt16LE(360, 14);
    header.writeUInt32LE(90000, 16); header.writeUInt32LE(1, 20); header.writeUInt32LE(frames.length, 24);
    const parts = [header];
    for (const frame of frames) {
        assert.equal(frame.timestamp, frame.index * 3000);
        const row = Buffer.alloc(12); row.writeUInt32LE(frame.bytes.length); row.writeBigUInt64LE(BigInt(frame.timestamp), 4);
        parts.push(row, frame.bytes);
    }
    const ivf = join(directory, name + '.ivf'); await writeFile(ivf, Buffer.concat(parts), { mode: 0o600 });
    const decoded = spawnSync('ffmpeg', ['-nostdin', '-hide_banner', '-loglevel', 'error', '-i', ivf,
        '-vf', 'crop=16:16:0:0', '-pix_fmt', 'yuv420p', '-fps_mode', 'passthrough',
        '-frames:v', String(frames.length), '-f', 'rawvideo', 'pipe:1'], { env, timeout: 10000, maxBuffer: 16384 });
    assert.equal(decoded.status, 0, String(decoded.stderr)); assert.equal(decoded.stdout.length, 384 * frames.length);
    for (const [i, frame] of frames.entries()) {
        const mean = decoded.stdout.subarray(i * 384, i * 384 + 256).reduce((a, b) => a + b, 0) / 256;
        assert.ok(Math.abs(mean - (60 + 3 * frame.index)) <= 3, 'fresh original marker at frame ' + frame.index);
    }
}

test('current feedback recovers fresh VP8 after same-SSRC epoch change and fragment loss without resetting clocks', { timeout: 30000 }, async () => {
    assert.equal(process.platform, 'linux'); assert.equal(process.arch, 'arm64');
    const base = resolve('.local/gate-6/live-feedback-recovery'); await mkdir(base, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(base, 'owned-'));
    const controller = new AbortController(); let encoder, worker, joined;
    try {
        const raw = join(directory, 'original.i420'), binary = join(directory, 'encoder'), plane = 640 * 360;
        const originals = Buffer.alloc(plane * 3 / 2 * 16, 128);
        for (let i = 0; i < 16; i++) originals.fill(60 + 3 * i, i * plane * 3 / 2, i * plane * 3 / 2 + plane);
        await writeFile(raw, originals, { mode: 0o600 });
        const build = spawnSync('python3', ['scripts/video/build_live_vp8_encoder.py', '--output', binary], { env, timeout: 60000, encoding: 'utf8' });
        assert.equal(build.status, 0, build.stderr);
        encoder = await LiveVp8Encoder.open(binary, raw, 16);
        const sender = new RtpSender({ handle: 7, streamId: 'camera', connectionId: 'connection', sessionId: 'session',
            generation: 1, kind: 'video', codec: 'vp8', payloadType: 96, direction: 'sendonly' }, 64);
        const history = new VideoReceiveHistory(true);
        let waiting, ssrc, consumed = 0, mode = 'current';
        const authority = {
            receiveVideoFeedback({ signal }) {
                assert.equal(waiting, undefined);
                return new Promise((resolve, reject) => {
                    const abort = () => { waiting = undefined; reject(stopped()); };
                    waiting = hint => { signal.removeEventListener('abort', abort); waiting = undefined; resolve(hint); };
                    if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true });
                });
            },
            async consumeVideoFeedback(hint) {
                consumed++; assert.equal(hint.ssrc, ssrc);
                if (mode === 'uncertain') throw Object.assign(Error('original consumption response lost'), { certainty: 'uncertain' });
                if (mode === 'expired') return undefined;
                const deadline = performance.now() + 100;
                return { isFresh: () => performance.now() < deadline };
            },
        };
        const publisher = new PublisherVideoFeedback(authority, controller.signal);
        worker = publisher.run(performance.now() + 10000);
        const offer = async () => { assert.ok(waiting); waiting({ ssrc }); await tick(); };
        const receive = (frame, epoch, drop, now = frame.index * 34) => {
            const packets = sender.packetize(frame.bytes, frame.timestamp); let result;
            if (drop !== undefined) assert.ok(packets.length > drop + 1, 'loss is an actual middle fragment');
            for (const [i, wire] of packets.entries()) {
                const packet = new SdkRtpPacket(new DataView(wire.buffer).getUint32(4), wire.subarray(8));
                ssrc = packet.header.ssrc;
                if (i === drop) continue;
                // Synthetic authenticated-history metadata at the SDK boundary.
                const observed = { header: packet.header, payload: packet.payload,
                    videoEpoch: { epoch, incarnation: 'owner', revision: Number(epoch), ssrc } };
                result = history.push('camera', observed, now) ?? result;
            }
            return result;
        };
        const frame = async (i, force = false) => encoder.frame(i, force, { timeoutMs: 200 });
        const first = await frame(0); assert.ok(receive(first, '1'));
        const delta = await frame(1); assert.equal(delta.key, false);
        assert.equal(receive(delta, '2'), undefined); assert.equal(history.takeRequest(100), true);
        assert.equal(history.takeRequest(199), false);
        await offer(); const permit = await publisher.take(ssrc, 100);
        const second = await frame(2, Boolean(permit?.isFresh()));
        const recovered = receive(second, '2'); assert.equal(recovered.key, true);
        assert.equal(history.takeRequest(201), false);
        const lost = await frame(3, true); assert.equal(receive(lost, '2', 1), undefined);
        assert.equal(receive(await frame(4), '2'), undefined);
        assert.equal(history.takeRequest(250), true);
        await offer(); const lossPermit = await publisher.take(ssrc, 100);
        const fifth = await frame(5, Boolean(lossPermit?.isFresh()));
        const recoveredLoss = receive(fifth, '2'); assert.equal(recoveredLoss.key, true);
        assert.notEqual(recoveredLoss.decoderHistory, recovered.decoderHistory);
        const sixth = await frame(6); assert.ok(receive(sixth, '2'));
        mode = 'expired'; await offer(); assert.equal(await publisher.take(ssrc, 100), undefined);
        const seventh = await frame(7); assert.equal(seventh.key, false);
        assert.equal(receive(seventh, '3'), undefined);
        assert.equal(receive(await frame(8), '3'), undefined);
        mode = 'current'; await offer(); const lastPermit = await publisher.take(ssrc, 100);
        const ninth = await frame(9, Boolean(lastPermit?.isFresh())); assert.ok(receive(ninth, '3'));
        assert.equal(await publisher.take(ssrc, 100), undefined, 'consumed hint must not force another frame');
        history.gap();
        // A decoder reset cannot make an old complete frame current again.
        assert.equal(receive(fifth, '3', undefined, 400), undefined);
        mode = 'uncertain'; await offer();
        await assert.rejects(publisher.take(ssrc, 100), /response lost/);
        assert.equal(await publisher.take(ssrc, 100), undefined); assert.equal(consumed, 5);
        await decode(directory, 'epoch-bootstrap', [second]);
        await decode(directory, 'fragment-recovery', [fifth, sixth]);
        await decode(directory, 'expired-hint-recovery', [ninth]);
        console.log(JSON.stringify({ independently_decoded_frames: 4, recovered_original_indices: [2, 5, 6, 9],
            feedback_authority: 'controlled SDK boundary', signed_transport_qualified: false }));
    } finally {
        controller.abort(); if (worker) await worker;
        if (encoder) { joined = await encoder.stopAndJoin(); assert.equal(joined.clean, true);
            assert.throws(() => process.kill(encoder.pid, 0), e => e.code === 'ESRCH'); }
        await rm(directory, { recursive: true });
    }
});

test('receiver feedback stays with its exact read across same-ID selection and refuses cloned observations', async () => {
    const parent = new AbortController(); let old = 0, current = 0, release;
    const reader = new SwitchingMediaReader('camera', { async receivePacket() { return 'old'; }, async requestKeyframe() { old++; } }, parent.signal);
    const read = await reader.receivePacket({ timeoutMs: 100 });
    const switching = reader.switchWith('selection', async () => {
        await new Promise(done => { release = done; });
        return { streamId: 'camera', media: { async receivePacket() { return 'new'; }, async requestKeyframe() { current++; return 'submitted'; } } };
    });
    assert.equal(await reader.requestKeyframe(read, { timeoutMs: 100 }), undefined);
    release(); await switching;
    assert.equal(await reader.requestKeyframe(read, { timeoutMs: 100 }), undefined);
    const next = await reader.receivePacket({ timeoutMs: 100 });
    assert.equal(await reader.requestKeyframe({ ...next }, { timeoutMs: 100 }), undefined);
    assert.equal(await reader.requestKeyframe(next, { timeoutMs: 100 }), 'submitted');
    parent.abort(); assert.equal(await reader.requestKeyframe(next, { timeoutMs: 100 }), undefined);
    assert.equal(old, 0); assert.equal(current, 1);
});
