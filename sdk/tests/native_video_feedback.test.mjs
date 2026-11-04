import test from 'node:test';
import assert from 'node:assert/strict';
import { captureNativeVideoFeedback } from '../typescript/dist/native_video_feedback.js';

test('native video allocation metadata preserves original identity and refuses malformed IPC descriptors', () => {
  const base = {profile: 'rtcp-pli-epoch/1', original_operation: 'operation', original_offer: 'offer', stream_generation: 1, connection_generation: 2};
  const captured = captureNativeVideoFeedback(base);
  base.original_offer = 'changed';
  assert.equal(captured.originalOffer, 'offer');
  assert.equal(captured.originalOperation, 'operation');
  assert.equal(captured.streamGeneration, 1);
  assert.equal(captured.connectionGeneration, 2);
  assert.ok(Object.isFrozen(captured));
  for (const [field, value] of [['profile', 'other/1'], ['original_offer', 'invalid offer'], ['original_operation', null], ['stream_generation', 0], ['stream_generation', 1.5], ['stream_generation', Number.MAX_SAFE_INTEGER + 1], ['connection_generation', -1], ['extra', true]]) {
    assert.throws(() => captureNativeVideoFeedback({...base, [field]: value}));
  }
  for (const key of Object.keys(base)) {
    const value = {...base}; delete value[key];
    assert.throws(() => captureNativeVideoFeedback(value));
  }
  for (const value of [null, [], 'profile', true]) assert.throws(() => captureNativeVideoFeedback(value));
});
