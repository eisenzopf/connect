import assert from 'node:assert/strict';
import {writeFile, readFile, realpath} from 'node:fs/promises';
import {createServer} from 'node:http';
import {fileURLToPath} from 'node:url';
import {CanonicalNativeClient, CanonicalHttpClient} from '@rudeless/connect';
const fixture = process.argv[2], dir = fileURLToPath(new URL('./', import.meta.url)), options = {timeoutMs: 1500};
const packageFile = await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')));
assert.ok(packageFile.startsWith(dir + 'node_modules/'));
const config = dir + 'synthetic.json'; await writeFile(config, JSON.stringify({}), {mode: 0o600});
const native = new CanonicalNativeClient({config, executable: fixture});
const scope = {cid: 'external-room', operation_id: 'persisted-mutation'};
const frame = (request, id) => ({v: 2, id, ts: new Date().toISOString(), type: 'ack', payload: {}, cid: request.cid, operation_id: request.operation_id, in_reply_to: request.id});
let calls = 0;
const server = createServer(async (request, response) => {
  let data = ''; for await (const chunk of request) data += chunk;
  const value = JSON.parse(data); assert.equal(request.url, '/v2/commands'); assert.equal(request.headers.authorization, 'Bearer synthetic-current-authority'); calls++;
  response.writeHead(200, {'content-type': 'application/json'}); response.end(JSON.stringify({profile: 'connect/2', request_id: value.id, envelopes: [frame(value, 'http_' + calls)]}));
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const http = new CanonicalHttpClient({endpoint: 'http://127.0.0.1:' + server.address().port, credential: 'synthetic-current-authority'});
try {
  const n = await native.request('message.send', {text: 'synthetic package consumer'}, scope, options);
  const h = await http.request('message.send', {text: 'synthetic package consumer'}, scope, options);
  assert.equal(n[0].operation_id, scope.operation_id); assert.equal(h[0].operation_id, scope.operation_id); assert.equal(n[0].cid, h[0].cid);
  await native.request('operation.get', {operation_id: 'persisted-mutation'}, {cid: scope.cid}, options);
} finally {
  await native.close(options); await http.close(options); server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
}
const lines = (await readFile(dir + 'requests.jsonl', 'utf8')).trim().split('\n').map(JSON.parse);
assert.deepEqual(lines.map(x => x.command.type), ['message.send', 'operation.get']); assert.equal(lines[0].scope.operation_id, scope.operation_id); assert.deepEqual(JSON.parse(await readFile(dir + 'exit.json', 'utf8')), {code: 0});
const pid = Number(await readFile(dir + 'pid', 'utf8')); assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH');
console.log(JSON.stringify({passed: true, installed_package: true, native_commands: lines.length, http_commands: calls, original_child_exit: 0, original_child_joined: true, scope: 'Synthetic helper and HTTP only; no real-server authority/parity qualification'}));
