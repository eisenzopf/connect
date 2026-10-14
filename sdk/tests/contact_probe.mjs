// Source SDK contact parity; credentials/challenges are never process arguments.
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {NativeClient,HttpClient,ConnectError}=await import(process.env.CONNECT_TYPESCRIPT_SDK ? pathToFileURL(process.env.CONNECT_TYPESCRIPT_SDK).href : '../typescript/dist/index.js');
const [ownerPath,peerPath,url,helper]=process.argv.slice(2);
let input='';for await(const chunk of process.stdin)input+=chunk;
const credentials=JSON.parse(input);
const secrets=[credentials.owner,credentials.peer,'private-challenge-wrong'];
// Observe the actual helper diagnostics that NativeClient forwards to stderr.
const originalWrite=process.stderr.write;
let diagnostics='';
process.stderr.write=function(chunk,encoding,callback){
 diagnostics+=Buffer.isBuffer(chunk)?chunk.toString('utf8'):String(chunk);
 if(diagnostics.length>600000)throw new Error('Helper diagnostics exceeded bound');
 const done=typeof encoding==='function'?encoding:callback;if(done)done();return true;
};
const ownerNative=new NativeClient(ownerPath,helper),peerNative=new NativeClient(peerPath,helper);
const ownerHttp=new HttpClient(url,credentials.owner),peerHttp=new HttpClient(url,credentials.peer);
const forbidden=error=>error instanceof ConnectError&&error.code==='forbidden';
try {
 const peerId=(await ownerNative.resolve('connect:agent@local')).id;
 for(const [lane,owner,peer,retryOwner,retryPeer] of [['native',ownerNative,peerNative,ownerHttp,peerHttp],['http',ownerHttp,peerHttp,ownerNative,peerNative]]) {
  const prefix='typescript-contact-'+lane,expiry=Math.floor(Date.now()/1000)+300;
  const pairing=await owner.contactPairingStart(prefix+'-start',peerId,expiry);
  secrets.push(pairing.challenge);
  const retried=await retryOwner.contactPairingStart(prefix+'-start',peerId,expiry);
  assert(pairing.challenge===retried.challenge,'Start replay changed private challenge');
  const publicFields=({challenge,...fields})=>fields;
  assert.deepEqual(publicFields(pairing),publicFields(retried));
  assert.equal(pairing.state,'pending');assert(pairing.challenge);
  for(const client of [owner,retryOwner])await assert.rejects(()=>client.contactPairingComplete(prefix+'-wrong-actor',pairing.id,pairing.challenge),forbidden);
  const failed=await peer.contactPairingComplete(prefix+'-bad',pairing.id,'private-challenge-wrong');
  assert.deepEqual(failed,await retryPeer.contactPairingComplete(prefix+'-bad',pairing.id,'private-challenge-wrong'));
  assert.equal(failed.state,'pending');assert(!('challenge' in failed));
  const complete=await peer.contactPairingComplete(prefix+'-complete',pairing.id,pairing.challenge);
  assert.deepEqual(complete,await retryPeer.contactPairingComplete(prefix+'-complete',pairing.id,pairing.challenge));
  assert.equal(complete.state,'verified_native');assert(!('challenge' in complete));
  const contact=await owner.contactState(peerId);
  assert.deepEqual(contact,await retryOwner.contactState(peerId));
  assert.equal(contact.ownership,'verified_native');assert.equal(contact.sms_consent,false);assert.notEqual(contact.verified_at,null);
  const revoked=await owner.contactPairingRevoke(prefix+'-revoke',pairing.id);
  assert.deepEqual(revoked,await retryOwner.contactPairingRevoke(prefix+'-revoke',pairing.id));
  assert.equal(revoked.state,'revoked');assert(!('challenge' in revoked));
  for(const client of [peer,retryPeer])await assert.rejects(()=>client.contactPairingComplete(prefix+'-complete',pairing.id,pairing.challenge),forbidden);
  assert.equal((await owner.contactState(peerId)).ownership,'revoked');assert.equal((await retryOwner.contactState(peerId)).sms_consent,false);
 }
 console.log('TypeScript contact native-helper and HTTP source parity passed');
} finally {
 await ownerNative.close();await peerNative.close();
 process.stderr.write=originalWrite;
 assert(!secrets.some(secret=>diagnostics.includes(secret)),'Helper leaked a private value');
}
