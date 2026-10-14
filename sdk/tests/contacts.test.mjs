import assert from 'node:assert/strict';
import {test} from 'node:test';
import {ConnectError} from '../typescript/dist/index.js';
import {decodeResponse} from '../typescript/dist/protocol.js';
const decode=(kind,data)=>decodeResponse({profile:'connect-control/1',result:{Ok:{kind,data}}},kind);
const invalid=error=>error instanceof ConnectError&&error.code==='protocol'&&!String(error).includes('private-response-secret');
test('native ownership DTO cannot infer SMS consent or echo malformed secrets',()=>{
 const contact={peer_identity_id:'peer',ownership:'verified_native',pairing_id:'pairing',verified_at:1,sms_consent:false};
 assert.deepEqual(decode('contact',contact).data,contact);
 for(const [field,values] of Object.entries({sms_consent:[true,0,null,'false'],ownership:['verified_phone',null,{}],verified_at:[true,-1,'now'],pairing_id:[0,{}]}))
  for(const value of values)assert.throws(()=>decode('contact',{...contact,[field]:value,private:'private-response-secret'}),invalid);
 const pairing={id:'pairing',owner_identity_id:'owner',peer_identity_id:'peer',expires_at:1,state:'pending',challenge:'private-response-secret'};
 assert.deepEqual(decode('contact_pairing',pairing).data,pairing);
 for(const [field,value] of [['state','verified_phone'],['expires_at',true],['expires_at',-1],['challenge',{}]])
  assert.throws(()=>decode('contact_pairing',{...pairing,[field]:value}),invalid);
});
