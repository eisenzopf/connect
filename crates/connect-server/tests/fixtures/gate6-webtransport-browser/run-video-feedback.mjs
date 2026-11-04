// Drive exactly one new recovery in a dedicated local ARM Chrome process.
import {readFile,writeFile} from 'node:fs/promises';
import {createPrivateKey} from 'node:crypto';
import path from 'node:path';
const input=JSON.parse(await readFile(process.argv[2],'utf8'));
for(const actor of input.run.actors){
 const original=Buffer.from(actor.private_pkcs8,'base64url');let der=original,publicKey;
 // ring emits RFC 5958 Ed25519 OneAsymmetricKey with the optional public
 // key. OpenSSL's PKCS#8 importer expects PrivateKeyInfo without that field.
 // Accept only the exact fixture encoding and verify the derived public key.
 if(actor.algorithm==='EdDSA'){
  if(original.length!==83||original.subarray(0,16).toString('hex')!=='3051020101300506032b657004220420'||original.subarray(48,51).toString('hex')!=='812100')throw Error('Unexpected fixture Ed25519 encoding');
  der=Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),original.subarray(16,48)]);
  publicKey=original.subarray(51).toString('base64url');
 }
 actor.private_jwk=createPrivateKey({key:der,format:'der',type:'pkcs8'}).export({format:'jwk'});
 if(publicKey&&actor.private_jwk.x!==publicKey)throw Error('Fixture Ed25519 public key changed');
 delete actor.private_pkcs8;
}
const version=await(await fetch(`http://127.0.0.1:${input.debug_port}/json/version`)).json();
if(version.Browser!=='Chrome/156.0.8078.4')throw Error('Pinned local ARM Chrome changed');
const socket=new WebSocket(version.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=()=>reject(Error('Owned CDP opening failed'));});
let serial=0;const pending=new Map();socket.onmessage=e=>{const v=JSON.parse(e.data),p=pending.get(v.id);if(!p)return;pending.delete(v.id);clearTimeout(p.timer);v.error?p.reject(Error(JSON.stringify(v.error))):p.resolve(v.result);};
function send(method,params={},sessionId,ms=15000){return new Promise((resolve,reject)=>{const id=++serial,timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP '+method+' deadline'));},ms);pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));});}
let target;
try{
 target=(await send('Target.createTarget',{url:input.page_url})).targetId;
 const session=(await send('Target.attachToTarget',{targetId:target,flatten:true})).sessionId;
 await send('Runtime.enable',{},session);
 const until=Date.now()+8000;
 for(;;){const v=await send('Runtime.evaluate',{expression:'typeof globalThis.runRecovery === "function"',returnByValue:true},session);if(v.result?.value===true)break;if(Date.now()>until)throw Error('Recovery page loading deadline');await new Promise(r=>setTimeout(r,25));}
 // This invocation is never repeated after uncertainty.
 const outcome=await send('Runtime.evaluate',{expression:`globalThis.runRecovery(${JSON.stringify(input.run)})`,awaitPromise:true,returnByValue:true},session,60000);
 if(outcome.exceptionDetails)throw Error(outcome.exceptionDetails.exception?.description??outcome.exceptionDetails.text);
 const result={...outcome.result.value,browser:version.Browser};if(!result.passed||!result.shutdown_joined)throw Error('Recovery proof incomplete');
 await writeFile(path.join(path.dirname(process.argv[2]),'browser-result.json'),JSON.stringify(result,null,2)+'\n',{mode:0o600});
 console.log(JSON.stringify(result));
}finally{
 if(target)await send('Target.closeTarget',{targetId:target}).catch(()=>{});
 for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('Owned CDP closed'));}pending.clear();socket.close();
}
