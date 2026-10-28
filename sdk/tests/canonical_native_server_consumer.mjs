// Copy into a fresh installed external Node project, then supply the private
// fixture spec. Uses only exported SDK methods and an owned PID-writing exec
// wrapper; no private SDK fields, operator grants, providers or model accounts.
import assert from 'node:assert/strict';
import {readFile, writeFile, realpath} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {CanonicalNativeClient, CanonicalHttpClient, CanonicalError} from '@rudeless/connect';
const spec = JSON.parse(await readFile(process.argv[2], 'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project + '/node_modules/'));
const options = {timeoutMs: 4000}, scope = {cid: spec.cid, operation_id: 'node-original-send'};
const body = {msg_id: 'node-original-message', to: 'all', content_type: 'text/plain', body: 'synthetic installed Node boundary', body_encoding: 'utf8', label: 'connect-messages', attachments: [], in_reply_to_msg: null};
const lines = createInterface({input: process.stdin})[Symbol.asyncIterator]();
async function resume(expected) {
  let timer;
  try {
    const next = await Promise.race([lines.next(), new Promise((_, reject) => {timer = setTimeout(() => reject(new Error('owned fixture phase deadline')), 10000);})]);
    assert.equal(next.value, expected);
  } finally {clearTimeout(timer);}
}
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');
const stopped = async () => {
  const pids = (await readFile(spec.pid_file, 'utf8')).trim().split('\n').filter(Boolean).map(Number);
  for (const pid of pids) assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH');
  return pids.length;
};
const record = frames => {const value = frames.find(frame => frame.type === 'operation.updated')?.payload; assert.ok(value); return value;};
let native = new CanonicalNativeClient({config: spec.config, executable: spec.executable});
const http = new CanonicalHttpClient({endpoint: spec.http, credential: spec.credential});
try {
  const sent = await native.request('message.send', body, scope, options);
  assert.ok(sent.every(frame => frame.signature?.alg === spec.algorithm));
  await writeFile(spec.config, 'intentionally invalid TOML after original helper loaded\n', {mode: 0o600});
  const original = record(await native.request('operation.get', {operation_id: scope.operation_id}, {cid: spec.cid}, options));
  const viaHttp = record(await http.request('operation.get', {operation_id: scope.operation_id}, {cid: spec.cid}, options));
  assert.equal(original.state, 'completed'); assert.equal(viaHttp.record_id, original.record_id); assert.deepEqual(viaHttp.result, original.result);
  await native.request('message.send', body, scope, options);
  await http.request('message.send', body, scope, options);
  const history = await native.request('message.history', {limit: 100}, {cid: spec.cid}, options);
  assert.equal(history.filter(frame => frame.type === 'message.send' && frame.payload.msg_id === body.msg_id).length, 1);
  const events = await native.request('conversation.subscribe', {live: false, limit: 100}, {cid: spec.cid}, options);
  assert.ok(events.some(frame => frame.type === 'conversation.snapshot'));
  await native.close(options); assert.equal(await stopped(), 1);
  emit({phase: 'initial', record_id: original.record_id, state: original.state, saved_replay_once: true, fixed_configuration: true, native_http_same_record: true, first_helper_joined: true});
  await resume('restart');
  native = new CanonicalNativeClient({config: spec.config, executable: spec.executable});
  const recovered = record(await native.request('operation.get', {operation_id: scope.operation_id}, {cid: spec.cid}, options));
  assert.equal(recovered.record_id, original.record_id); assert.deepEqual(recovered.result, original.result);
  emit({phase: 'recovered', record_id: recovered.record_id, state: recovered.state, resubmitted: false});
  await resume('revoked');
  let nativeFailure, httpFailure;
  await assert.rejects(native.request('message.send', body, scope, options), error => {
    assert.ok(error instanceof CanonicalError); assert.ok(['authentication', 'transport-uncertain'].includes(error.kind));
    assert.ok(['refused', 'uncertain'].includes(error.certainty)); assert.equal(error.operation.operationId, scope.operation_id); nativeFailure = {kind: error.kind, certainty: error.certainty}; return true;
  });
  await assert.rejects(http.request('message.send', body, scope, options), error => {
    assert.ok(error instanceof CanonicalError); assert.equal(error.kind, 'authentication'); assert.equal(error.certainty, 'refused'); assert.equal(error.operation.operationId, scope.operation_id); httpFailure = {kind: error.kind, certainty: error.certainty}; return true;
  });
  await native.close(options); await http.close(options); assert.equal(await stopped(), 2);
  emit({phase: 'completed', algorithm: spec.algorithm, recovered_original: true, revoked_native_replay_not_accepted: true, native_failure: nativeFailure, revoked_http_replay_refused: true, http_failure: httpFailure, helpers_joined: 2});
} finally {
  await native.close(options); await http.close(options); process.stdin.destroy();
}
