// Actual production SDK, served from its built module files by the owned fixture.
import {BrowserClient} from '/sdk/browser_client.js';
import {unbase64url} from '/sdk/uctp_signing.js';
const check=(value,message)=>{if(!value)throw Error(message);};
async function options(actor){
  const a=actor.algorithm==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
  const server=actor.server_algorithm==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
  return {url:actor.url,credential:actor.credential,identityId:actor.identity_id,device:actor.device,
    signing:{key:await crypto.subtle.importKey('jwk',actor.private_jwk,a,false,['sign']),keyId:actor.keyid,algorithm:actor.algorithm},
    server:{key:await crypto.subtle.importKey('raw',unbase64url(actor.server_public_raw),server,false,['verify']),keyId:actor.server_keyid,algorithm:actor.server_algorithm,notBefore:actor.server_not_before,expiresAt:actor.server_expires_at},
    serverCertificateHashes:[{algorithm:'sha-256',value:unbase64url(actor.certificate_sha256)}]};
}
export async function run(config){
  const o=await options(config.actors[0]);let client;
  try{
    client=await BrowserClient.open(o);
    check(client.identityId===config.actors[0].identity_id,'SDK enrolled identity');
    const first=await client.createConversation('browser-create',{participants:[config.actors[1].identity_id]});
    client.close();check(client.closed,'SDK explicit owned close');
    client=await BrowserClient.open(o);
    const replay=await client.createConversation('browser-create',{participants:[config.actors[1].identity_id]});
    check(first.id===replay.id&&JSON.stringify(first.receipt)===JSON.stringify(replay.receipt),'SDK durable same-owner replay');
    const state=await client.operationStatus(first.id,'browser-create');
    check(state.state==='completed','SDK authoritative operation status');
    client.close();client=undefined;
    let credential_refused=false,trust_refused=false;
    try{client=await BrowserClient.open({...o,credential:'invalid-fixture-credential',timeoutMs:3000});}
    catch(e){credential_refused=e.code==='refused';}
    check(credential_refused,'SDK invalid credential refusal');client?.close();client=undefined;
    try{client=await BrowserClient.open({...o,server:{...o.server,keyId:'foreign-enrolled-key'},timeoutMs:3000});}
    catch(e){trust_refused=e.code==='authentication';}
    check(trust_refused,'SDK separate enrolled signing trust refusal');
    return {passed:true,production_sdk:true,created_cid:first.id,operation_state:state.state,credential_refused,trust_refused,algorithm:config.actors[0].algorithm};
  }finally{client?.close();}
}
