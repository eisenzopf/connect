import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {openSync,closeSync,fstatSync,readSync,writeSync,fsyncSync,lstatSync} from 'node:fs';
export const NATIVE_PROGRAMME_PROFILE='connect/native-programme-worker/1';
export const now=()=>Number(process.hrtime.bigint())/1e9;
export const MAX_REPORT_BYTES=8*1024*1024;
const finite=value=>typeof value==='number'&&Number.isFinite(value)&&value>=0;
export function originalWorkerOwner(value){
 assert.equal(value?.profile,NATIVE_PROGRAMME_PROFILE);
 assert.ok(Number.isSafeInteger(value.original_node_pid)&&value.original_node_pid>1);
 assert.ok(Number.isSafeInteger(value.logical_worker_thread_id)&&value.logical_worker_thread_id>0);
 assert.ok(finite(value.created_monotonic_seconds)&&finite(value.started_monotonic_seconds));
 assert.ok(value.started_monotonic_seconds>=value.created_monotonic_seconds);
 assert.ok(finite(value.original_global_deadline_monotonic_seconds)&&value.started_monotonic_seconds<value.original_global_deadline_monotonic_seconds&&value.original_global_deadline_monotonic_seconds<=value.created_monotonic_seconds+300);
 assert.deepEqual(Object.keys(value).sort(),['profile','original_node_pid','logical_worker_thread_id','created_monotonic_seconds','started_monotonic_seconds','original_global_deadline_monotonic_seconds'].sort());
 return value;
}
export function workerJoinVerified(value,owner,terminal){
 try{
  originalWorkerOwner(owner);assert.deepEqual(value.owner,owner);assert.ok(finite(terminal));
  assert.equal(value.original_terminal_monotonic_seconds,terminal);
  assert.equal(value.original_exit_observed,true);assert.equal(value.exit_code,0);assert.equal(value.forced,false);
  assert.equal(value.stdio_joined,true);assert.equal(value.report_digest_verified,true);assert.equal(value.failure,null);
  assert.match(value.report_sha256,/^[a-f0-9]{64}$/);assert.ok(Number.isSafeInteger(value.report_bytes)&&value.report_bytes>0&&value.report_bytes<=MAX_REPORT_BYTES);
  assert.ok(finite(value.exit_monotonic_seconds)&&finite(value.joined_monotonic_seconds));
  assert.ok(value.exit_monotonic_seconds>=terminal&&value.joined_monotonic_seconds>=value.exit_monotonic_seconds&&value.joined_monotonic_seconds-terminal<=10&&value.joined_monotonic_seconds<=owner.original_global_deadline_monotonic_seconds);
  return true;
 }catch{return false;}
}
/** Small fixed buffer; the full work report stays in its original private file. */
export function sealedFile(path,buffer,{sync=false}={}){
 assert.ok(Buffer.isBuffer(buffer)&&buffer.length===65536);
 const link=lstatSync(path);assert.ok(link.isFile()&&!link.isSymbolicLink());
 const fd=openSync(path,'r');
 try{
  const before=fstatSync(fd,{bigint:true});assert.ok(before.isFile()&&before.size>0n&&before.size<=BigInt(MAX_REPORT_BYTES));
  assert.equal(Number(before.mode)&0o077,0);let bytes=0;const hash=createHash('sha256');
  for(;;){const size=readSync(fd,buffer,0,Math.min(buffer.length,MAX_REPORT_BYTES-bytes+1),null);if(!size)break;bytes+=size;assert.ok(bytes<=MAX_REPORT_BYTES);hash.update(buffer.subarray(0,size));}
  const after=fstatSync(fd,{bigint:true});for(const key of['dev','ino','size','mtimeNs','ctimeNs'])assert.equal(after[key],before[key]);
  assert.equal(BigInt(bytes),before.size);if(sync)fsyncSync(fd);
  return {sha256:hash.digest('hex'),bytes,identity:{dev:String(before.dev),ino:String(before.ino),size:String(before.size),mtime_ns:String(before.mtimeNs),ctime_ns:String(before.ctimeNs)}};
 }finally{closeSync(fd);}
}
/** Worker stdio goes to the inherited ordinary log file; no parent stream/pipe is opened. */
export function ownWorkerStdio(stream,fd,{limit=8*1024*1024}={}){
 assert.ok(fstatSync(fd).isFile(),'Worker logs require the existing ordinary-file descriptor');
 let bytes=0,failure=null,ended=false;
 const data=chunk=>{
  if(!Buffer.isBuffer(chunk)){failure??='non-buffer-worker-stdio';return;}
  bytes+=chunk.length;if(bytes>limit){failure??='worker-stdio-bound';return;}
  try{for(let at=0;at<chunk.length;){const n=writeSync(fd,chunk,at,Math.min(65536,chunk.length-at));assert.ok(n>0);at+=n;}}
  catch{failure??='worker-stdio-write';}
 };
 const error=()=>{failure??='worker-stdio-error';};
 stream.on('data',data);stream.on('error',error);
 const joined=new Promise(resolve=>{
  stream.once('end',()=>{ended=true;resolve();});
  stream.once('close',()=>{if(!ended){failure??='worker-stdio-incomplete';resolve();}});
 });
 return {joined,receipt:()=>({bytes,ended,failure,reference_released:stream===undefined}),release(){
  if(stream===undefined)return true;
  // A completed original stream no longer needs a diagnostic strong owner.
  // Missing end/error/bound proof remains failed and retains its reference.
  if(!ended||failure)return false;
  stream.off('data',data);stream.off('error',error);stream=undefined;return true;
 }};
}
