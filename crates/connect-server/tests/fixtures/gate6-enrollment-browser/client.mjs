import {BrowserEnrollmentKey,BrowserEnrollmentStore} from '/sdk/browser.js';
import {enrollmentProofMessage} from '/sdk/browser_enrollment_contract.js';
const assert=(value,message)=>{if(!value)throw Error(message);};
const refuse=async(work,message)=>{let refused=false;try{await work();}catch{refused=true;}assert(refused,message);};
const decode=s=>Uint8Array.from(atob(s.replaceAll('-','+').replaceAll('_','/')),v=>v.charCodeAt(0));
const digest=async s=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(s))),b=>b.toString(16).padStart(2,'0')).join('');
async function post(path,value){const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(value)});assert(r.ok,'fixture backend route refused');return r.json();}
async function approve(vault,name){const original=await vault.request(name),response=await post('/approve',original);assert(response.profile==='connect-enrollment/1'&&response.result.Ok,'protected offer response');return response.result.Ok;}
async function trust(cfg){const key=await crypto.subtle.importKey('raw',decode(cfg.server_public_raw),cfg.server_algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'},true,['verify']);return {key,keyId:cfg.server_keyid,algorithm:cfg.server_algorithm,notBefore:cfg.server_not_before,expiresAt:cfg.server_expires_at};}
function contact(cfg,transport,server){return {url:transport==='webtransport'?cfg.wt_url:cfg.wss_url,server,...(transport==='webtransport'?{serverCertificateHashes:[{algorithm:'sha-256',value:decode(cfg.certificate_sha256)}]}:{})};}
async function record(vault,name,s,value){return {name,id:s.offer.enrollment_id,credentialId:value.credential.credential_id,expiry:value.credential.expires_at,challengeExpiry:s.offer.request.challenge_expires_at,bearerDigest:await digest(value.credential.credential),proof:await (await BrowserEnrollmentKey.restore(s.snapshot().key)).claim(s.offer)};}
async function open(session,value,where){const client=await session.open(value,where,{timeoutMs:5000});assert(client.identityId===session.offer.request.identity_id,'installed actor');client.close();}
globalThis.runBrowserFixture=async cfg=>{
 assert(isSecureContext&&location.protocol==='https:','actual secure browser Origin');
 const server=await trust(cfg),vault=new BrowserEnrollmentStore(cfg.database);
 if(cfg.phase==='foreign'){
  let refused=0;
  for(const item of cfg.entries){const response=await fetch(`${cfg.api_origin}/v2/enrollments/${item.id}/claim`,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(item.proof)});assert(response.status===403,'actual separately-allowed foreign Origin must refuse');refused++;}
  return {passed:true,foreign_origin_refusals:refused};
 }
 const entries=[];
 if(cfg.phase==='prepare'){
  for(const algorithm of ['EdDSA','ES256'])for(const transport of ['webtransport','wss-webrtc']){
   const name=`${cfg.suffix}-${algorithm}-${transport}`,key=await BrowserEnrollmentKey.generate(algorithm),now=Math.floor(Date.now()/1000);
   const request={operation_id:`approve-${name}`,completion_operation_id:`claim-${name}`,identity_id:cfg.identity_id,device:{id:`device-${name}`,kind:'browser',platform:'owned-Chrome'},key_id:`key-${name}`,algorithm,public_key:key.publicKey,transport,endpoint_address:`${location.origin}/device/${name}`,capabilities:['data','control'],browser_origin:location.origin,operations:['operation.get'],recipients:[cfg.recipient_id],credential_expires_at:now+300,challenge_expires_at:now+10};
   await vault.prepare(name,cfg.api_origin,request,key);await refuse(()=>vault.prepare(name,cfg.api_origin,request,key),'pending name cannot replace original');
   const offer=await approve(vault,name),broader=structuredClone(offer);broader.request.operations.push('send');await refuse(()=>vault.acceptOffer(name,broader),'broader approved offer');
   const saved=await vault.acceptOffer(name,offer);assert(saved.snapshot().key.privateKey.extractable===false,'nonextractable persisted key');
   const changed=structuredClone(offer);changed.challenge='f'.repeat(64);await refuse(()=>vault.acceptOffer(name,changed),'approved challenge cannot be replaced');
   const cancelled=new AbortController();cancelled.abort();await refuse(()=>saved.claim({signal:cancelled.signal}),'pre-cancelled claim');
   const value=await saved.claim();await open(saved,value,contact(cfg,transport,server));
   // A correct issued bearer/key cannot be installed through the other path.
   const wrong=contact(cfg,transport==='webtransport'?'wss-webrtc':'webtransport',server);await refuse(async()=>saved.clientOptions(value,wrong),'wrong transport installation');
   entries.push(await record(vault,name,saved,value));
  }
  const expired=[];
  for(const item of entries){
   const original=await vault.load(item.name),key=await BrowserEnrollmentKey.generate(original.offer.request.algorithm),name=item.name+'-expired',request=structuredClone(original.offer.request);
   request.operation_id='approve-'+name;request.completion_operation_id='claim-'+name;request.key_id='key-'+name;request.device.id='device-'+name;request.endpoint_address=location.origin+'/device/'+name;request.public_key=key.publicKey;request.challenge_expires_at=Math.floor(Date.now()/1000)+3;
   await vault.prepare(name,cfg.api_origin,request,key);const session=await vault.acceptOffer(name,await approve(vault,name));
   const proof=await key.claim(session.offer);proof.endpoint_id='foreign-endpoint';const wrong=await fetch(`${cfg.api_origin}/v2/enrollments/${session.offer.enrollment_id}/claim`,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(proof)});assert(wrong.status===403,'wrong Endpoint claim');expired.push(name);
  }
  const victim=await vault.load(entries[0].name),key=await BrowserEnrollmentKey.generate(victim.offer.request.algorithm),name=entries[0].name+'-stolen',request=structuredClone(victim.offer.request);
  request.operation_id='approve-'+name;request.completion_operation_id='claim-'+name;request.key_id='key-'+name;request.device.id='device-'+name;request.endpoint_address=location.origin+'/device/'+name;request.public_key=key.publicKey;request.challenge_expires_at=Math.floor(Date.now()/1000)+120;
  await vault.prepare(name,cfg.api_origin,request,key);const stolen=await vault.acceptOffer(name,await approve(vault,name)),attacker=await BrowserEnrollmentKey.generate(key.algorithm),claim=await key.claim(stolen.offer);
  const signature=new Uint8Array(await crypto.subtle.sign(key.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',hash:'SHA-256'},attacker.snapshot().privateKey,await enrollmentProofMessage(stolen.offer)));
  claim.signature=btoa(String.fromCharCode(...signature)).replaceAll('+','-').replaceAll('/','_').replaceAll('=','');
  for(let i=0;i<5;i++){const denied=await fetch(`${cfg.api_origin}/v2/enrollments/${stolen.offer.enrollment_id}/claim`,{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify(claim)});assert(denied.status===403,'stolen challenge without approved key');}
  await refuse(()=>stolen.claim(),'exhausted challenge is terminal');
  return {passed:true,phase:'prepare',entries,expired,nonextractable_keys:4,signed_connections:4,wrong_endpoint_refusals:4,stolen_proof_refusals:5};
 }
 assert(cfg.phase==='resume','known fixture phase');
 for(const name of cfg.expired){const saved=await vault.load(name);let code;try{await saved.claim();}catch(error){code=error.code;}assert(code==='gone','fresh expired challenge cannot grant');}
 for(const original of cfg.entries){
  // New page/process context, original IndexedDB state only; no issued bearer saved.
  const saved=await vault.load(original.name),replayed=await saved.claim();
  assert(replayed.credential.credential_id===original.credentialId&&replayed.credential.expires_at===original.expiry&&await digest(replayed.credential.credential)===original.bearerDigest,'exact original recovery after actual server/page restart');
  const transport=saved.offer.request.transport;await open(saved,replayed,contact(cfg,transport,server));
  const replacementKey=await BrowserEnrollmentKey.generate(saved.offer.request.algorithm),replacementName=original.name+'-rotation';
  const request=structuredClone(saved.offer.request);request.operation_id='approve-'+replacementName;request.completion_operation_id='claim-'+replacementName;request.key_id='key-'+replacementName;request.public_key=replacementKey.publicKey;request.challenge_expires_at=Math.floor(Date.now()/1000)+120;
  await vault.prepare(replacementName,cfg.api_origin,request,replacementKey);const replacement=await vault.acceptOffer(replacementName,await approve(vault,replacementName)),issued=await replacement.claim();
  assert(issued.device_id===replayed.device_id&&issued.endpoint_id===replayed.endpoint_id&&issued.key_id!==replayed.key_id,'rotation exact shared Device/Endpoint and fresh key');
  await post('/revoke',{enrollment_id:saved.offer.enrollment_id,operation_id:'revoke-'+original.name});
  await refuse(()=>saved.claim(),'old original recovery after revoke');await refuse(()=>saved.open(replayed,contact(cfg,transport,server),{timeoutMs:3000}),'retired actual signed peer');
  await open(replacement,issued,contact(cfg,transport,server));
  await post('/revoke',{enrollment_id:replacement.offer.enrollment_id,operation_id:'revoke-'+replacementName});await refuse(()=>replacement.claim(),'replacement cleanup current revocation');
  entries.push({name:original.name,recovered:true,rotated:true,retired:true});
 }
 return {passed:true,phase:'resume',entries,expired_fresh_refusals:4,recovered_after_challenge_expiry:4,recovered:4,rotated:4,revoked:8,signed_connections:8};
};
