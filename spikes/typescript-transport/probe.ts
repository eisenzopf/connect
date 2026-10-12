// Runtime experiment: explicit native Rust helper, never a WebSocket substitute.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { performance } from 'node:perf_hooks';
const run = promisify(execFile);
const [binary, directory] = process.argv.slice(2);
if (!binary || !directory) throw new Error('Pass the built quic_dispatch helper and its fixture directory');
const started = performance.now();
const { stdout } = await run(binary, ['client', directory], { timeout: 15_000 });
if (!stdout.includes('PASS native QUIC')) throw new Error('Native exchange did not complete');
console.log(JSON.stringify({ transport: 'quic', architecture: 'explicit Rust helper process', exchange: 'authenticated correlated request/response', elapsedMs: +(performance.now() - started).toFixed(2), platform: process.platform, arch: process.arch }));
