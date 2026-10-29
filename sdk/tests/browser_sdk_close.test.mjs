import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserSdkClient} from '../typescript/dist/browser.js';
import {fixture} from './fixtures/browser_control_peer.mjs';

const options={timeoutMs:500};
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return{promise,resolve};};
const tick=()=>new Promise(setImmediate);
async function scenario(path,run,{handler}={}) {
 return fixture(async f=>{
  let resolver=async()=>f.options;
  const client=await BrowserSdkClient.connect({tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect','wss-webrtc':'wss://fixture.invalid/connect'},preferredTransport:path,resolveEnrollment:context=>resolver(context),timeoutMs:500});
  try{await run({client,f,peer:()=>path==='webtransport'?f.transports.at(-1):f.socket(),setResolver:value=>resolver=value});}
  finally{await client.close(options);}
 },{wtMode:'ok',handler});
}
function delayClose(peer) {
 const gate=deferred(),original=peer.close.bind(peer);let calls=0;
 peer.close=()=>{if(!calls++)void gate.promise.then(original);};
 return{release:gate.resolve,calls:()=>calls};
}
for(const path of ['webtransport','wss-webrtc']) {
 test(`bounded SDK close joins delayed ${path} closure before resolving`,()=>scenario(path,async({client,f,peer})=>{
  const gate=delayClose(peer()),sent=f.sent.length;let joined=false;
  const closing=client.close(options).then(()=>joined=true);
  assert.equal(client.transport.state,'closed');await tick();assert.equal(joined,false);assert(gate.calls()>0);
  gate.release();await closing;assert.equal(joined,true);assert.equal(f.sent.length,sent);
  await client.close(options);await assert.rejects(()=>client.reconnect(options),{kind:'transport-uncertain',certainty:'not-submitted'});
 }));
 for(const kind of ['deadline','cancelled','pre-aborted'])test(`bounded SDK ${path} ${kind} stops waiting while cleanup continues`,()=>scenario(path,async({client,f,peer})=>{
  const gate=delayClose(peer()),abort=new AbortController(),sent=f.sent.length;
  if(kind==='pre-aborted')abort.abort();
  const started=performance.now(),closing=client.close({timeoutMs:kind==='deadline'?20:500,signal:abort.signal});
  if(kind==='cancelled')abort.abort();
  await assert.rejects(()=>closing,{kind:kind==='deadline'?'deadline':'cancelled',certainty:'read-only'});
  assert(performance.now()-started<400);assert.equal(client.transport.state,'closed');
  gate.release();await client.close(options);assert.equal(f.sent.length,sent);
 }));
 test(`SDK ${path} invalid shutdown options refuse before retiring a usable client`,()=>scenario(path,async({client,f})=>{
  const sent=f.sent.length;
  for(const wait of [undefined,{}, {timeoutMs:0},{timeoutMs:60001},{timeoutMs:NaN},{timeoutMs:100,signal:{aborted:false}}])await assert.rejects(()=>client.close(wait),{kind:'validation'});
  assert.equal(client.transport.state,'connected');assert.equal(f.sent.length,sent);
 }));
 test(`SDK ${path} shutdown fences a late enrollment resolver without waiting for application work`,()=>scenario(path,async({client,f,setResolver})=>{
  const entered=deferred(),late=deferred();setResolver(async()=>{entered.resolve();return late.promise;});
  const reconnecting=client.reconnect(options);void reconnecting.catch(()=>{});await entered.promise;
  const peers=f.sockets.length+f.transports.length,sent=f.sent.length;
  await client.close(options);await assert.rejects(()=>reconnecting,{kind:'cancelled'});
  late.resolve(f.options);await tick();assert.equal(client.transport.state,'closed');assert.equal(f.sockets.length+f.transports.length,peers);assert.equal(f.sent.length,sent);
 }));
 test(`SDK ${path} shutdown joins retired peers after reconnect`,()=>scenario(path,async({client,peer})=>{
  const original=peer(),gate=delayClose(original);
  await client.reconnect(options);assert.notEqual(peer(),original);
  let joined=false;const closing=client.close(options).then(()=>joined=true);await tick();assert.equal(joined,false);
  gate.release();await closing;
 }));
 test(`SDK ${path} bounds retained physical cleanup owners`,()=>scenario(path,async({client,f,peer})=>{
  const gates=[];
  try{
   for(let i=0;i<15;i++){gates.push(delayClose(peer()));await client.reconnect(options);}
   const peers=f.sockets.length+f.transports.length;
   await assert.rejects(()=>client.reconnect(options),{kind:'transport-uncertain',certainty:'not-submitted'});
   assert.equal(f.sockets.length+f.transports.length,peers);
  }finally{for(const gate of gates)gate.release();}
 }));
}
test('SDK WebTransport shutdown joins an already admitted write and its stream abort',async()=>{
 const entered=deferred(),release=deferred();
 await scenario('webtransport',async({client,f})=>{
  const reading=client.conversations.byId('room').refresh(options);void reading.catch(()=>{});await entered.promise;
  let joined=false;const closing=client.close(options).then(()=>joined=true);await tick();assert.equal(joined,false);
  release.resolve();await closing;await assert.rejects(()=>reading);assert.equal(f.sent.filter(v=>v.type==='conversation.get').length,1);
 },{handler:async({frame})=>{if(frame.type==='conversation.get'){entered.resolve();await release.promise;return true;}return false;}});
});
