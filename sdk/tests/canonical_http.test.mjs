import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createServer} from 'node:http';
import {CanonicalHttpClient, CanonicalError, HttpClient, ConnectError} from '../typescript/dist/index.js';
const options = {timeoutMs: 1500};
const scope = {cid: 'room', operation_id: 'saved-send'};
const key = {operationId: 'saved-send', command: 'message.send', scope: {conversationId: 'room'}};
function frame(request, type = 'ack', payload = {}, extra = {}) {
  const value = {v: 2, id: 'reply', ts: new Date().toISOString(), type, payload, in_reply_to: request.id};
  for (const field of ['cid', 'sid', 'connid', 'operation_id']) if (request[field] !== undefined) value[field] = request[field];
  return {...value, ...extra};
}
function batch(request, envelopes = [frame(request)]) {
  return {profile: 'connect/2', request_id: request.id, envelopes};
}
function send(response, value, status = 200) {
  response.writeHead(status, {'content-type': 'application/json'});
  response.end(JSON.stringify(value));
}
async function fixture(action, run) {
  let calls = 0;
  const server = createServer(async (request, response) => {
    let raw = '';
    for await (const chunk of request) raw += chunk;
    calls++;
    await action(JSON.parse(raw), response, request);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const client = new CanonicalHttpClient({endpoint: `http://127.0.0.1:${server.address().port}`, credential: 'fixture-credential'});
  try { await run(client, () => calls); }
  finally {
    await client.close(options);
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
}
const rejects = (kind, certainty, operation) => error => {
  assert.ok(error instanceof CanonicalError);
  assert.equal(error.kind, kind); assert.equal(error.certainty, certainty);
  assert.deepEqual(error.operation, operation);
  assert.ok(!error.message.includes('private-raw-body'));
  return true;
};
test('canonical HTTP uses current path/bearer and captures intent before yielding', async () => {
  await fixture((request, response, http) => {
    assert.equal(http.url, '/v2/commands');
    assert.equal(http.headers.authorization, 'Bearer fixture-credential');
    assert.equal(request.payload.text, 'original'); assert.equal(request.cid, 'room');
    assert.equal(request.operation_id, 'saved-send');
    send(response, batch(request));
  }, async client => {
    const payload = {text: 'original'}, supplied = {...scope};
    const pending = client.request('message.send', payload, supplied, options);
    payload.text = 'changed'; supplied.cid = 'foreign';
    const replies = await pending;
    assert.equal(replies.length, 1); assert.equal(replies[0].type, 'ack');
  });
});
test('canonical HTTP configuration rejects implicit remote/credential URL destinations', () => {
  for (const endpoint of ['https://example.test', 'http://example.test', 'http://127.0.0.1/path', 'http://a:b@localhost', 'http://localhost?x=1', 'http://localhost#x'])
    assert.throws(() => new CanonicalHttpClient({endpoint, credential: 'x'}), rejects('configuration', 'not-submitted'));
  for (const credential of ['', 'bad\nheader', ' '.repeat(20000)])
    assert.throws(() => new CanonicalHttpClient({endpoint: 'http://localhost', credential}), rejects('configuration', 'not-submitted'));
});
test('canonical HTTP refuses invalid/missing mutation intent and live/native paths without writing', async () => {
  await fixture(() => assert.fail('must stay local'), async (client, calls) => {
    for (const [type, payload, supplied, kind] of [
      ['message.send', {}, {cid: 'room'}, 'validation'],
      ['message.send', {}, {...scope, connid: 'connection'}, 'validation'],
      ['message.send', {}, {...scope, in_reply_to: 'foreign'}, 'validation'],
      ['conversation.subscribe', {}, {cid: 'room'}, 'unsupported'],
      ['connection.offer', {}, scope, 'unsupported'], ['auth.refresh', {}, {}, 'unsupported'],
      ['message.send', {text: 'é'.repeat(40000)}, scope, 'validation'],
    ]) await assert.rejects(client.request(type, payload, supplied, options), error => error instanceof CanonicalError && error.kind === kind && error.certainty === 'not-submitted');
    for (const timeoutMs of [0, NaN, 1.1, 120001])
      await assert.rejects(client.request('operation.get', {}, {cid: 'room'}, {timeoutMs}), rejects('validation', 'not-submitted'));
    const signal = AbortSignal.abort();
    await assert.rejects(client.request('message.send', {}, scope, {...options, signal}), rejects('cancelled', 'not-submitted', key));
    assert.equal(calls(), 0);
  });
});
test('canonical HTTP rejects mismatched profile, correlation, scope, signatures and terminal order', async () => {
  for (const change of [
    (r, b) => ({...b, profile: 'connect/1'}), (r, b) => ({...b, request_id: 'foreign'}),
    (r, b) => ({...b, envelopes: []}), (r, b) => ({...b, unexpected: true}),
    (r, b) => batch(r, [frame(r, 'ack', {}, {cid: 'foreign'})]),
    (r, b) => batch(r, [frame(r, 'ack', {}, {operation_id: 'foreign'})]),
    (r, b) => batch(r, [frame(r, 'ack', {}, {sid: 'foreign'})]),
    (r, b) => batch(r, [frame(r, 'ack', {}, {in_reply_to: 'foreign'})]),
    (r, b) => batch(r, [frame(r, 'ack', {}, {signature: {private: 'private-raw-body'}})]),
    (r, b) => batch(r, [frame(r), frame(r, 'ack', {}, {id: 'second'})]),
    (r, b) => batch(r, [frame(r, 'operation.updated')]),
    (r, b) => batch(r, [frame(r, 'operation.updated'), frame(r)]),
    (r, b) => batch(r, Array.from({length: 129}, (_, i) => frame(r, i === 128 ? 'ack' : 'operation.updated', {}, {id: 'r' + i}))),
  ]) await fixture((r, response) => send(response, change(r, batch(r))), async (client, calls) => {
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('protocol', 'uncertain', key));
    assert.equal(calls(), 1);
  });
});
test('canonical HTTP enforces duplicate-key, UTF8, MIME and HTTP status boundaries', async () => {
  for (const action of [
    (r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.end('{"profile":"connect/1","profile":"connect/2"}'); },
    (r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.end(Buffer.from([0xff])); },
    (r, s) => { s.writeHead(200, {'content-type': 'text/html'}); s.end('private-raw-body'); },
    (r, s) => send(s, batch(r), 503),
    (r, s) => { s.writeHead(200, {'content-type': 'application/json', 'content-length': 2097153}); s.end(); },
    (r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.end(' '.repeat(2097153)); },
  ]) await fixture(action, async client => {
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('protocol', 'uncertain', key));
  });
});
test('canonical HTTP accepts a bounded multi-envelope batch larger than one envelope', async () => {
  await fixture((r, s) => send(s, batch(r, [
    frame(r, 'operation.updated', {extension: 'x'.repeat(600000)}),
    frame(r, 'ack', {details: 'x'.repeat(600000)}, {id: 'terminal'}),
  ])), async client => {
    const replies = await client.request('message.send', {}, scope, options);
    assert.equal(replies.length, 2); assert.equal(replies[0].payload.extension.length, 600000);
  });
});
test('canonical HTTP structured refusals retain current identity and never become transport retries', async () => {
  for (const wrapped of [true, false]) await fixture((r, s) => {
    const p = {code: 409, category: 'conflict', reason: 'operation_conflict'};
    send(s, wrapped ? {profile: 'connect/2', problem: p} : batch(r, [frame(r, 'error', p)]), 409);
  }, async (client, calls) => {
    await assert.rejects(client.request('message.send', {}, scope, options), error => {
      rejects('conflict', 'refused', key)(error);
      assert.equal(error.problem.code, 409); return true;
    });
    assert.equal(calls(), 1);
  });
});
test('canonical HTTP refusal code must agree with HTTP and public diagnostics stay closed', async () => {
  for (const p of [{code: 403, category: 'auth', reason: 'forbidden'}, {code: 409, category: 'state', reason: 'private-raw-body\nsecret'}])
    await fixture((r, s) => send(s, {profile: 'connect/2', problem: p}, 409), async client => {
      await assert.rejects(client.request('message.send', {}, scope, options), rejects('protocol', 'uncertain', key));
    });
});
test('canonical HTTP never follows redirects with bearer or retries an uncertain mutation', async () => {
  await fixture((r, s) => { s.writeHead(307, {location: '/secret-target'}); s.end(); }, async (client, calls) => {
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('protocol', 'uncertain', key));
    assert.equal(calls(), 1);
  });
  await fixture((r, s) => s.destroy(), async (client, calls) => {
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('transport-uncertain', 'uncertain', key));
    assert.equal(calls(), 1);
  });
});
test('canonical HTTP one absolute deadline includes a partially delivered body', async () => {
  await fixture((r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.write('{'); }, async client => {
    const started = performance.now();
    await assert.rejects(client.request('message.send', {}, scope, {timeoutMs: 80}), rejects('deadline', 'uncertain', key));
    assert.ok(performance.now() - started < 1200);
  });
});
test('canonical HTTP cancellation after admission preserves logical mutation identity', async () => {
  const abort = new AbortController();
  await fixture((r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.write('{'); abort.abort(); }, async client => {
    await assert.rejects(client.request('message.send', {}, scope, {...options, signal: abort.signal}), rejects('cancelled', 'uncertain', key));
  });
});
test('canonical HTTP finite journal responses bind every nested resource and cursor', async () => {
  for (const wrong of [false, true]) await fixture((r, s) => {
    const payload = {event_id: 'fact', cursor: 'cursor', data: {event_id: 'fact', cursor: wrong ? 'foreign' : 'cursor', conversation_id: 'room', session_id: 'session', connection_id: null, operation_id: 'original'}};
    send(s, batch(r, [frame(r, 'conversation.event', payload, {sid: 'session', operation_id: 'original'}), frame(r, 'ack', {}, {id: 'terminal'})]));
  }, async client => {
    const pending = client.request('conversation.subscribe', {live: false}, {cid: 'room'}, options);
    if (wrong) await assert.rejects(pending, rejects('protocol', 'read-only'));
    else assert.equal((await pending)[0].sid, 'session');
  });
});
test('canonical HTTP caps active requests and close joins cancelled work without retry', async () => {
  await fixture((r, s) => { s.writeHead(200, {'content-type': 'application/json'}); s.write('{'); }, async (client, calls) => {
    const pending = Array.from({length: 16}, () => client.request('operation.get', {operation_id: 'original'}, {cid: 'room'}, options).catch(error => error));
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('application', 'not-submitted', key));
    await client.close(options);
    for (const error of await Promise.all(pending)) rejects('cancelled', 'read-only')(error);
    await client.close(options);
    await assert.rejects(client.request('message.send', {}, scope, options), rejects('configuration', 'not-submitted', key));
    assert.ok(calls() <= 16);
  });
});

test('named application HTTP receipt errors preserve actual status independently of the error code',async()=>{
 const calls=[];let status=401;
 const server=createServer(async(request,response)=>{
  let body='';for await(const chunk of request)body+=chunk;
  const operation=JSON.parse(body);assert.equal(request.url,'/v1/operations');assert.equal(operation.op,'message_receipts');calls.push(operation);
  response.writeHead(status,{'content-type':'application/json'});response.end(JSON.stringify({profile:'connect-control/1',result:{Err:{code:'unauthorized',message:'synthetic refusal'}}}));
 });await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 try {
  const client=new HttpClient(`http://127.0.0.1:${server.address().port}`,'fixture',1500);
  for(const value of [401,403,500,200]){status=value;await assert.rejects(client.messageReceipts('room','message'),error=>{
   assert.ok(error instanceof ConnectError);assert.equal(error.code,'unauthorized');assert.equal(error.httpStatus,value);
   const authenticatedRefusal=['unauthorized','forbidden'].includes(error.code)&&[401,403].includes(error.httpStatus);assert.equal(authenticatedRefusal,value===401||value===403);return true;
  });}
  assert.equal(calls.length,4);
 } finally {server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
