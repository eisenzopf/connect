// CDP client for an isolated Chrome process owned by the Rust fixture.
import {readFile} from 'node:fs/promises';
import {createPrivateKey} from 'node:crypto';
const config=JSON.parse(await readFile(process.argv[2],'utf8'));
// Normalize the existing enrolled private key into WebCrypto's JWK input;
// ring-generated Ed25519 PKCS8 v2 is not accepted by Chrome's DER importer.
for(const actor of config.run.actors){actor.private_jwk=createPrivateKey({key:Buffer.from(actor.private_pkcs8,'base64url'),format:'der',type:'pkcs8'}).export({format:'jwk'});delete actor.private_pkcs8;}
const versionResponse=await fetch(`http://127.0.0.1:${config.debug_port}/json/version`);
if(!versionResponse.ok)throw Error('owned Chrome debugger unavailable');
const version=await versionResponse.json();const socket=new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=()=>reject(Error('owned CDP connection failed'));});
let serial=0;const pending=new Map();
socket.onmessage=event=>{const message=JSON.parse(event.data);if(!message.id)return;const request=pending.get(message.id);if(!request)return;pending.delete(message.id);clearTimeout(request.timer);message.error?request.reject(Error('CDP '+request.method+' refused')):request.resolve(message.result);};
function send(method,params={},sessionId){return new Promise((resolve,reject)=>{const id=++serial;const timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP '+method+' deadline'));},70000);pending.set(id,{resolve,reject,timer,method});socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));});}
let target;
try {
  const browser=await send('Browser.getVersion');target=(await send('Target.createTarget',{url:config.page_url})).targetId;
  const session=(await send('Target.attachToTarget',{targetId:target,flatten:true})).sessionId;
  await send('Runtime.enable',{},session);
  const until=Date.now()+8000;
  for(;;){const ready=await send('Runtime.evaluate',{expression:"typeof globalThis.runBrowserFixture === 'function'",returnByValue:true},session);if(ready.result.value)break;if(Date.now()>until)throw Error('browser fixture page readiness deadline');await new Promise(r=>setTimeout(r,50));}
  const result=await send('Runtime.evaluate',{expression:'globalThis.runBrowserFixture('+JSON.stringify(config.run)+')',awaitPromise:true,returnByValue:true},session);
  if(result.exceptionDetails||!result.result?.value?.passed)throw Error(result.exceptionDetails?.exception?.description||'browser fixture returned no success');
  console.log(JSON.stringify({browser:{product:browser.product,protocol_version:browser.protocolVersion,user_agent:browser.userAgent,js_version:browser.jsVersion},result:result.result.value}));
} finally {
  if(target)await send('Target.closeTarget',{targetId:target}).catch(()=>{});
  for(const item of pending.values()){clearTimeout(item.timer);item.reject(Error('owned CDP closed'));}pending.clear();socket.close();
}
