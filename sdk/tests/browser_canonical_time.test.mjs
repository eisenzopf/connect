import assert from 'node:assert/strict';
import {test} from 'node:test';
import {identifier,envelope,encodeEnvelope,EnvelopeReader} from '../typescript/dist/uctp_wire.js';
import {canonicalTimestamp} from '../typescript/dist/uctp_datetime.js';
import {signEnvelope,verifyEnvelope} from '../typescript/dist/uctp_signing.js';
test('resource identifiers reject a trailing newline before canonical framing',()=>{
 for(const value of ['resource\n','resource\r','resource\r\n','x\u2028','x\u2029']){assert.equal(identifier(value),false);assert.throws(()=>envelope('operation.get',{operation_id:'saved'},{cid:value}),{code:'protocol'});}assert(identifier('x'.repeat(128)));assert.equal(identifier('x'.repeat(129)),false);
});
test('canonical framing rejects a rolled invalid calendar date instead of normalizing it',()=>{
 const frame={v:2,id:'calendar',ts:'2026-02-30T00:00:00Z',type:'ack',payload:{}};assert.throws(()=>encodeEnvelope(frame),{code:'protocol'});assert.equal(Number.isNaN(canonicalTimestamp(frame.ts)),true);
});
test('signed canonical verification rejects a normalized invalid calendar date at the matching rolled clock',async()=>{
 const pair=await crypto.subtle.generateKey({name:'Ed25519'},false,['sign','verify']),now=Date.UTC(2026,2,2),trust={key:pair.publicKey,keyId:'calendar-key',algorithm:'EdDSA',notBefore:now/1000-30,expiresAt:now/1000+300},frame=await signEnvelope({v:2,id:'signed-calendar',ts:'2026-02-30T00:00:00Z',type:'ack',payload:{}},{key:pair.privateKey,keyId:trust.keyId,algorithm:'EdDSA'});await assert.rejects(()=>verifyEnvelope(frame,trust,now),{code:'authentication'});
});
for(const algorithm of ['EdDSA','ES256'])test('canonical '+algorithm+' framing and verification preserve a signed Unicode-minus RFC3339 offset',async()=>{
 const pair=await crypto.subtle.generateKey(algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'},false,['sign','verify']),now=Date.UTC(2026,9,29,12),ts='2026-10-29T11:30:00\u221200:30',trust={key:pair.publicKey,keyId:'offset-key',algorithm,notBefore:now/1000-30,expiresAt:now/1000+300},frame=await signEnvelope({v:2,id:'signed-offset',ts,type:'ack',payload:{}},{key:pair.privateKey,keyId:trust.keyId,algorithm});assert.equal(canonicalTimestamp(ts),now);const [decoded]=[...new EnvelopeReader().push(encodeEnvelope(frame))];assert.equal(decoded.ts,ts);await verifyEnvelope(decoded,trust,now);
});
