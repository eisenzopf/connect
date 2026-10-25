// Actual TLS/Origin/text-frame SDK control. This Node lane is not a browser/WebRTC rehearsal.
import {readFile} from 'node:fs/promises';
import {createPrivateKey} from 'node:crypto';
import {BrowserClient} from '../../../../../sdk/typescript/dist/browser_client.js';
import {unbase64url} from '../../../../../sdk/typescript/dist/uctp_signing.js';
const config=JSON.parse(await readFile(process.argv[2],'utf8'));
const NativeWebSocket=globalThis.WebSocket;
// Node exposes explicit request headers. A browser supplies its actual page Origin.
globalThis.WebSocket=class extends NativeWebSocket {constructor(url){super(url,{headers:{Origin:config.origin}});}};
Object.defineProperty(globalThis,'isSecureContext',{value:true,configurable:true});
const algorithm=config.algorithm==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
const jwk=createPrivateKey({key:Buffer.from(config.private_pkcs8,'base64url'),format:'der',type:'pkcs8'}).export({format:'jwk'});
const options={transport:'websocket',url:config.url,credential:config.credential,identityId:config.identity_id,device:config.device,
  signing:{key:await crypto.subtle.importKey('jwk',jwk,algorithm,false,['sign']),keyId:config.keyid,algorithm:config.algorithm},
  server:{key:await crypto.subtle.importKey('raw',unbase64url(config.server_public_raw),algorithm,false,['verify']),keyId:config.server_keyid,algorithm:config.algorithm,notBefore:config.server_not_before,expiresAt:config.server_expires_at},timeoutMs:5000};
let first,second;
try {
  first=await BrowserClient.open(options);
  const created=await first.createConversation('wss-sdk-original',{participants:[config.other_identity],tenantId:'tnt_default'});
  if(created.receipt.operation_id!=='wss-sdk-original')throw Error('Original receipt missing');
  first.close();
  second=await BrowserClient.open(options);
  const status=await second.operationStatus(created.id,'wss-sdk-original');
  if(status.state!=='completed'||status.operation_id!=='wss-sdk-original')throw Error('Original status missing');
  console.log(JSON.stringify({passed:true,algorithm:config.algorithm,canonical_wss:true,platform_tls:true,original_receipt:true,fresh_status_only:true,scope:'Node actual TLS SDK control; no browser or WebRTC qualification'}));
} finally {first?.close();second?.close();}
