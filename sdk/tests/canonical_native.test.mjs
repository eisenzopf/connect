import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdir, mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {resolve, join} from 'node:path';
import {CanonicalNativeClient, CanonicalError} from '../typescript/dist/index.js';
const root = fileURLToPath(new URL('../../', import.meta.url));
const fixtureExecutable = fileURLToPath(new URL('./fixtures/canonical_helper.mjs', import.meta.url));
const opts = {timeoutMs: 1500};
const scope = {cid: 'room', operation_id: 'original-message'};
const key = {operationId: 'original-message', command: 'message.send', scope: {conversationId: 'room'}};
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const check = (kind, certainty, operation) => error => {
  assert.ok(error instanceof CanonicalError); assert.equal(error.kind, kind); assert.equal(error.certainty, certainty);
  assert.deepEqual(error.operation, operation); assert.ok(!error.message.includes('private-provider-key')); return true;
};
async function fixture(config, run) {
  const base = join(root, '.local/gate-6/node-canonical-helper'); await mkdir(base, {recursive: true, mode: 0o700});
  const dir = await mkdtemp(join(base, 'unit-')), path = join(dir, 'config.json');
  await writeFile(path, JSON.stringify(config), {mode: 0o600});
  const client = new CanonicalNativeClient({config: path, executable: fixtureExecutable});
  try { await run(client, dir, path); }
  finally {
    await client.close(opts);
    const pid = Number(await readFile(join(dir, 'pid'), 'utf8').catch(() => '0'));
    if (pid) assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH');
    await rm(dir, {recursive: true, force: true});
  }
}
async function requests(dir) {return (await readFile(join(dir, 'requests.jsonl'), 'utf8').catch(() => '')).trim().split('\n').filter(Boolean).map(JSON.parse);}

test('current helper captures canonical intent, independent exchange IDs and original Rust signatures', async () => {
  for (const algorithm of ['EdDSA', 'ES256']) await fixture({algorithm}, async (client, dir, path) => {
    const payload = {text: 'original'}, supplied = {...scope};
    const pending = client.request('message.send', payload, supplied, opts); payload.text = 'changed'; supplied.cid = 'foreign';
    const frames = await pending; assert.equal(frames[0].signature.alg, algorithm); assert.equal(frames[0].in_reply_to, 'rust_1');
    const calls = await requests(dir); assert.equal(calls.length, 1); assert.equal(calls[0].command.type, 'message.send');
    assert.equal(calls[0].command.payload.text, 'original'); assert.deepEqual(calls[0].scope, {conversation_id: 'room', operation_id: 'original-message'});
    assert.notEqual(calls[0].id, 'original-message'); assert.notEqual(calls[0].id, 'rust_1');
    assert.deepEqual(JSON.parse(await readFile(join(dir, 'argv.json'), 'utf8')), ['stdio', '--config', path]);
    await client.close(opts); assert.deepEqual(JSON.parse(await readFile(join(dir, 'exit.json'), 'utf8')), {code: 0});
  });
});
test('configuration is explicit and invalid intent/aborted requests never write helper input', async () => {
  for (const config of [null, undefined, {}, {config: null}, {config: ''}, {config: 'a\0b'}, {config: 'a', executable: ''}]) assert.throws(() => new CanonicalNativeClient(config), check('configuration', 'not-submitted'));
  await fixture({}, async (client, dir) => {
    for (const [type, payload, supplied, kind] of [['message.send', {}, {cid: 'room'}, 'validation'], ['conversation.subscribe', {}, {cid: 'room'}, 'unsupported'], ['connection.offer', {}, scope, 'unsupported'], ['message.send', {}, {...scope, in_reply_to: 'foreign'}, 'validation']])
      await assert.rejects(client.request(type, payload, supplied, opts), error => error instanceof CanonicalError && error.kind === kind && error.certainty === 'not-submitted');
    await assert.rejects(client.request('message.send', {}, scope, {...opts, signal: AbortSignal.abort()}), check('cancelled', 'not-submitted', key));
    assert.equal((await requests(dir)).length, 0);
  });
});
test('queued deadline removes only the unsent intent and later work uses the same original helper', async () => {
  await fixture({delayMs: 100}, async (client, dir) => {
    await client.request('conversation.get', {}, {cid: 'room'}, opts);
    const first = client.request('message.send', {}, scope, opts);
    const second = client.request('message.send', {}, {...scope, operation_id: 'queued-expired'}, {timeoutMs: 20});
    const third = client.request('message.send', {}, {...scope, operation_id: 'after-queue'}, opts);
    await assert.rejects(second, check('deadline', 'not-submitted', {...key, operationId: 'queued-expired'}));
    await first; await third; const calls = await requests(dir);
    assert.deepEqual(calls.map(x => x.scope.operation_id).filter(Boolean), ['original-message', 'after-queue']);
  });
});
test('queued cancellation keeps the helper and cannot become a submitted mutation', async () => {
  await fixture({delayMs: 100}, async client => {
    await client.request('conversation.get', {}, {cid: 'room'}, opts);
    const first = client.request('message.send', {}, scope, opts), controller = new AbortController();
    const second = client.request('message.send', {}, {...scope, operation_id: 'queued-cancel'}, {...opts, signal: controller.signal});
    controller.abort(); await assert.rejects(second, check('cancelled', 'not-submitted', {...key, operationId: 'queued-cancel'}));
    await first; await client.request('operation.get', {}, {cid: 'room'}, opts);
  });
});
test('sent deadline includes partial output, fences helper and retains the saved mutation key', async () => {
  await fixture({mode: 'partial'}, async (client, dir) => {
    await assert.rejects(client.request('message.send', {}, scope, {timeoutMs: 100}), check('deadline', 'uncertain', key));
    await assert.rejects(client.request('message.send', {}, scope, opts), check('configuration', 'not-submitted', key));
    assert.equal((await requests(dir)).length, 1);
  });
});
test('sent cancellation fences this instance without restarting or retrying', async () => {
  await fixture({delayMs: 100}, async (client, dir) => {
    await client.request('conversation.get', {}, {cid: 'room'}, opts); const controller = new AbortController();
    const pending = client.request('message.send', {}, scope, {...opts, signal: controller.signal});
    controller.abort(); await assert.rejects(pending, check('cancelled', 'uncertain', key));
    await assert.rejects(client.request('message.send', {}, scope, opts), check('configuration', 'not-submitted', key));
    assert.ok((await requests(dir)).length <= 2);
  });
});
test('corrupt, oversized, unsigned, duplicate or foreign helper replies fail closed and are never retried', async () => {
  for (const mode of ['profile', 'wrong-id', 'wrong-scope', 'wrong-reply', 'utf8', 'duplicate', 'extra', 'oversize', 'signature', 'unsigned']) await fixture({mode}, async (client, dir) => {
    await assert.rejects(client.request('message.send', {}, scope, opts), check('protocol', 'uncertain', key));
    await assert.rejects(client.request('message.send', {}, scope, opts), check('configuration', 'not-submitted', key));
    assert.equal((await requests(dir)).length, 1);
  });
});
test('late uncorrelated output fences an otherwise idle original helper', async () => {
  await fixture({mode: 'late-extra'}, async client => {
    await client.request('message.send', {}, scope, opts); await delay(50);
    await assert.rejects(client.request('message.send', {}, scope, opts), check('configuration', 'not-submitted', key));
  });
});
test('structured refusals are redacted and leave the helper usable', async () => {
  for (const [code, kind] of [['conflict', 'conflict'], ['unauthorized', 'authentication'], ['unsupported-command', 'unsupported'], ['configuration', 'configuration']]) await fixture({mode: 'reject', code}, async (client, dir) => {
    await assert.rejects(client.request('message.send', {}, scope, opts), check(kind, 'refused', key));
    await client.request('operation.get', {}, {cid: 'room'}, opts); assert.equal((await requests(dir)).length, 2);
  });
});
test('helper exit leaves sent work uncertain and no replacement helper is created', async () => {
  await fixture({mode: 'exit'}, async (client, dir) => {
    await assert.rejects(client.request('message.send', {}, scope, opts), check('transport-uncertain', 'uncertain', key));
    assert.equal((await requests(dir)).length, 1);
  });
});
test('missing executable proves no submission and still has an owned close', async () => {
  const client = new CanonicalNativeClient({config: 'unused', executable: '/no-such-connect-helper-canonical-fixture'});
  await assert.rejects(client.request('message.send', {}, scope, opts), check('configuration', 'not-submitted', key));
  await client.close(opts);
});
test('queue capacity and close settle every original wait and join the child', async () => {
  await fixture({delayMs: 200}, async client => {
    await client.request('conversation.get', {}, {cid: 'room'}, opts);
    const pending = Array.from({length: 16}, (_, index) => client.request('message.send', {}, {...scope, operation_id: 'saved-' + index}, opts).catch(error => error));
    await assert.rejects(client.request('message.send', {}, scope, opts), check('application', 'not-submitted', key));
    await client.close(opts); const results = await Promise.all(pending);
    for (const [index, error] of results.entries()) { assert.equal(error.kind, 'cancelled'); assert.equal(error.operation.operationId, 'saved-' + index); assert.equal(error.certainty, index ? 'not-submitted' : 'uncertain'); }
  });
});
test('bounded close reports unconfirmed cleanup and kills the original hanging helper', async () => {
  await fixture({mode: 'hang-close'}, async client => {
    await client.request('operation.get', {}, {cid: 'room'}, opts);
    await assert.rejects(client.close({timeoutMs: 20}), check('cleanup-incomplete', 'read-only'));
    await client.close(opts);
  });
});
