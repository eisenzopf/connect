// CDP client for an isolated Chrome process owned by the Rust fixture.
import {readFile} from 'node:fs/promises';
import {createPrivateKey} from 'node:crypto';
const config=JSON.parse(await readFile(process.argv[2],'utf8'));
if(process.env.CONNECT_GATE6_BROWSER_CLEAN60==='1'&&config.run.capture===true)config.run.clean60=true;
const captureBoundary=process.env.CONNECT_GATE6_BROWSER_CAPTURE_BOUNDARY;
if(captureBoundary!==undefined){
  if(!['permission-denied','unsupported-api-codec','background-resume','media-loss-recovery'].includes(captureBoundary)||config.run.capture!==true||config.run.clean60||config.run.native)throw Error('Select an explicit short browser/browser capture boundary');
  config.run.captureBoundary=captureBoundary;
  if(captureBoundary==='permission-denied')config.capture_permissions='denied';
  if(captureBoundary==='background-resume')config.visibility_probe=true;
}

// Normalize the existing enrolled private key into WebCrypto's JWK input;
// ring-generated Ed25519 PKCS8 v2 is not accepted by Chrome's DER importer.
for(const actor of config.run.actors.filter(actor=>actor.private_pkcs8)){actor.private_jwk=createPrivateKey({key:Buffer.from(actor.private_pkcs8,'base64url'),format:'der',type:'pkcs8'}).export({format:'jwk'});delete actor.private_pkcs8;}
const versionResponse=await fetch(`http://127.0.0.1:${config.debug_port}/json/version`);
if(!versionResponse.ok)throw Error('owned Chrome debugger unavailable');
const version=await versionResponse.json();const socket=new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=()=>reject(Error('owned CDP connection failed'));});
let serial=0;const pending=new Map();
socket.onmessage=event=>{const message=JSON.parse(event.data);if(!message.id)return;const request=pending.get(message.id);if(!request)return;pending.delete(message.id);clearTimeout(request.timer);message.error?request.reject(Error('CDP '+request.method+' refused')):request.resolve(message.result);};
function send(method,params={},sessionId){return new Promise((resolve,reject)=>{const id=++serial;const timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP '+method+' deadline'));},config.run.clean60===true?90000:70000);pending.set(id,{resolve,reject,timer,method});socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));});}
let target,cover;
try {
  const browser=await send('Browser.getVersion');if(config.run.clean60===true&&browser.product!=='Chrome/155.0.8059.85')throw Error('Frozen clean60 Chrome patch version mismatch');target=(await send('Target.createTarget',{url:config.page_url})).targetId;
  const session=(await send('Target.attachToTarget',{targetId:target,flatten:true})).sessionId;
  if(config.capture_permissions!==undefined){
    if(!['granted','denied'].includes(config.capture_permissions))throw Error('bounded fixture permission setting');
    for(const name of ['microphone','camera'])await send('Browser.setPermission',{permission:{name},setting:config.capture_permissions,origin:new URL(config.page_url).origin});
  }
  await send('Runtime.enable',{},session);
  const until=Date.now()+8000;
  for(;;){const ready=await send('Runtime.evaluate',{expression:"typeof globalThis.runBrowserFixture === 'function'",returnByValue:true},session);if(ready.result.value)break;if(Date.now()>until)throw Error('browser fixture page readiness deadline');await new Promise(r=>setTimeout(r,50));}
  const evaluation=send('Runtime.evaluate',{expression:'globalThis.runBrowserFixture('+JSON.stringify(config.run)+')',awaitPromise:true,returnByValue:true,...(config.user_gesture===true?{userGesture:true}:{})},session);
  void evaluation.catch(()=>{});
  if(config.visibility_probe===true){
    const phase=async expected=>{
      const until=Date.now()+10000;
      for(;;){const current=await send('Runtime.evaluate',{expression:'globalThis.captureVisibilityPhase',returnByValue:true},session);if(current.result.value===expected)return;if(Date.now()>until)throw Error('actual page visibility phase '+expected+' deadline');await new Promise(r=>setTimeout(r,25));}
    };
    await phase('hide');cover=(await send('Target.createTarget',{url:'about:blank',background:false})).targetId;
    await send('Target.activateTarget',{targetId:cover});await phase('hidden');await send('Target.activateTarget',{targetId:target});
  }
  const result=await evaluation;
  if(result.exceptionDetails||!result.result?.value?.passed)throw Error(result.exceptionDetails?.exception?.description||'browser fixture returned no success');
  console.log(JSON.stringify({browser:{product:browser.product,protocol_version:browser.protocolVersion,user_agent:browser.userAgent,js_version:browser.jsVersion},result:result.result.value}));
} finally {
  if(cover)await send('Target.closeTarget',{targetId:cover}).catch(()=>{});
  if(target)await send('Target.closeTarget',{targetId:target}).catch(()=>{});
  for(const item of pending.values()){clearTimeout(item.timer);item.reject(Error('owned CDP closed'));}pending.clear();socket.close();
}
