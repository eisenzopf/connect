#!/usr/bin/env node
// Synthetic owned helper protocol only. This fixture does not verify signatures
// or qualify Rust/native/HTTP parity; the installed Rust helper must do that.
import {readFileSync, appendFileSync, writeFileSync} from 'node:fs';
import {dirname, join} from 'node:path';
import {createInterface} from 'node:readline';
const config = JSON.parse(readFileSync(process.argv[process.argv.indexOf('--config') + 1], 'utf8'));
const dir = dirname(process.argv[process.argv.indexOf('--config') + 1]);
writeFileSync(join(dir, 'pid'), String(process.pid), {mode: 0o600});
writeFileSync(join(dir, 'argv.json'), JSON.stringify(process.argv.slice(2)), {mode: 0o600});
process.once('exit', code => writeFileSync(join(dir, 'exit.json'), JSON.stringify({code}), {mode: 0o600}));
let counter = 0;
if (config.mode === 'hang-close') setInterval(() => {}, 1000);
createInterface({input: process.stdin}).on('line', line => {
  const request = JSON.parse(line), number = ++counter;
  appendFileSync(join(dir, 'requests.jsonl'), line + '\n', {mode: 0o600});
  if (config.mode === 'exit') return process.exit(0);
  if (config.mode === 'partial') return process.stdout.write('{');
  if (config.mode === 'utf8') return process.stdout.write(Buffer.from([255, 10]));
  if (config.mode === 'oversize') return process.stdout.write('x'.repeat(2097154) + '\n');
  const requestId = 'rust_' + number;
  const frame = {v: 2, id: 'reply_' + number, ts: new Date().toISOString(), type: 'ack', payload: {}, in_reply_to: requestId,
    signature: {keyid: 'synthetic-trusted-helper', alg: config.algorithm ?? 'EdDSA', sig: 'A'.repeat(86)}};
  for (const [native, wire] of [['conversation_id', 'cid'], ['session_id', 'sid'], ['connection_id', 'connid'], ['operation_id', 'operation_id']])
    if (request.scope[native] !== undefined) frame[wire] = request.scope[native];
  let result = {Ok: {kind: 'canonical_reply', data: {request_id: requestId, frames: [frame]}}};
  if (config.mode === 'reject' && number === 1) result = {Err: {code: config.code ?? 'conflict', message: 'private-provider-key'}};
  if (config.mode === 'wrong-scope') frame.cid = 'foreign';
  if (config.mode === 'wrong-reply') frame.in_reply_to = 'foreign';
  if (config.mode === 'signature-alg-array') frame.signature.alg = [frame.signature.alg];
  if (config.mode === 'signature') frame.signature.sig = 'A'.repeat(85) + 'B';
  if (config.mode === 'unsigned') delete frame.signature;
  const response = {id: config.mode === 'wrong-id' ? 'foreign' : request.id, response: {profile: config.mode === 'profile' ? 'foreign' : 'connect-control/1', result}};
  let raw = JSON.stringify(response);
  if (config.mode === 'duplicate') raw = '{"id":"other",' + raw.slice(1);
  if (config.mode === 'extra') raw += '\n' + raw;
  const send = () => {
    process.stdout.write(raw + '\n');
    if (config.mode === 'late-extra') setTimeout(() => process.stdout.write(raw + '\n'), 20);
  };
  if (config.delayMs) setTimeout(send, config.delayMs); else send();
});
