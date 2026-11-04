import test from 'node:test';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
const base = process.env.CONNECT_GATE6_TS_DIST
  ? pathToFileURL(path.resolve(process.env.CONNECT_GATE6_TS_DIST) + '/').href
  : new URL('../typescript/dist/', import.meta.url).href;
const { decodeNativeRtpDatagram, decodeNativeVideoEpoch, decodeNativeVideoFeedback, encodeNativeVideoFeedback } = await import(base + 'browser_video_wire.js');
const { decodeRtcpSenderReport } = await import(base + 'browser_rtcp.js');

test('native browser feedback matches the independent 48-byte Rust contract and refuses alternate framing', () => {
  const value = { handle: 0x1234, datagramSequence: 0xffffffff, senderSsrc: 0x10203040, mediaSsrc: 0x50607080, epoch: '0102030405060708090a0b0c0d0e0f10' };
  const literal = Uint8Array.from(Buffer.from('01001234ffffffff81ce0002102030405060708080cc000610203040434e45500102030405060708090a0b0c0d0e0f10', 'hex'));
  assert.deepEqual(encodeNativeVideoFeedback(value), literal);
  assert.deepEqual(decodeNativeVideoFeedback(literal), value);
  const offset = new Uint8Array(60); offset.set(literal, 7);
  assert.deepEqual(decodeNativeVideoFeedback(offset.subarray(7, 55)), value);
  assert.throws(() => decodeNativeRtpDatagram(literal));
  assert.throws(() => decodeRtcpSenderReport(literal.subarray(8)));
  assert.throws(() => decodeNativeVideoEpoch(literal));
  for (let end = 0; end < 48; end++) assert.throws(() => decodeNativeVideoFeedback(literal.subarray(0, end)));
  assert.throws(() => decodeNativeVideoFeedback(Uint8Array.from([...literal, 0])));
  for (const index of [0, 1, 8, 9, 10, 11, 20, 21, 22, 23, 24, 28, 29, 30, 31]) {
    const bad = literal.slice(); bad[index] ^= 1; assert.throws(() => decodeNativeVideoFeedback(bad));
  }
  for (const [begin, end] of [[2, 4], [12, 16], [16, 20], [32, 48]]) {
    const bad = literal.slice(); bad.fill(0, begin, end); assert.throws(() => decodeNativeVideoFeedback(bad));
  }
  for (const [field, bad] of [['handle', 0], ['handle', 65536], ['datagramSequence', -1], ['datagramSequence', 2 ** 32], ['senderSsrc', 0], ['mediaSsrc', 1.5], ['epoch', '0'.repeat(32)], ['epoch', 'A'.repeat(32)], ['epoch', '1'.repeat(31)]])
    assert.throws(() => encodeNativeVideoFeedback({ ...value, [field]: bad }));
});

test('native browser epoch framing preserves original RTP and cannot enter ordinary or clock parsers', () => {
  // Original sequence/clock/SSRC, RTP extension and padding are independent
  // literals. Both epoch and feedback codecs must respect nonzero byte offsets.
  const header = Buffer.from('01001234fffffffe', 'hex');
  const epoch = Buffer.from('8092030405060708090a0b0c0d0e0f10', 'hex');
  const rtp = Buffer.from('b0e0fffefffffff050607080bede00011055000010000002', 'hex');
  const ordinary = Uint8Array.from(Buffer.concat([header, rtp]));
  const framed = Uint8Array.from(Buffer.concat([header, epoch, rtp])); framed[1] = 1;
  const captured = decodeNativeVideoEpoch(framed);
  assert.equal(captured.epoch, epoch.toString('hex'));
  assert.deepEqual(captured.packet, decodeNativeRtpDatagram(ordinary));
  assert.equal(captured.packet.timestamp, 0xfffffff0);
  assert.equal(captured.packet.sequence, 65534);
  assert.equal(captured.packet.datagramSequence, 0xfffffffe);
  assert.equal(captured.packet.padding, 2);
  assert.deepEqual([...captured.packet.payload], [0x10, 0]);
  const offset = new Uint8Array(framed.length + 8); offset.set(framed, 3);
  assert.deepEqual(decodeNativeVideoEpoch(offset.subarray(3, 3 + framed.length)), captured);
  assert.throws(() => decodeNativeRtpDatagram(framed));
  assert.throws(() => decodeNativeVideoEpoch(ordinary));
  assert.throws(() => decodeNativeVideoFeedback(framed));
  for (const [index, value] of [[0, 2], [1, 0], [1, 2], [24, 0], [25, 206]]) {
    const bad = framed.slice(); bad[index] = value; assert.throws(() => decodeNativeVideoEpoch(bad));
  }
  for (const [begin, end] of [[2, 4], [8, 24], [32, 36]]) {
    const bad = framed.slice(); bad.fill(0, begin, end); assert.throws(() => decodeNativeVideoEpoch(bad));
  }
  assert.throws(() => decodeNativeVideoEpoch(new Uint8Array(1180 + 25)));
  for (let end = 0; end < 36; end++) assert.throws(() => decodeNativeVideoEpoch(framed.subarray(0, end)));
});
