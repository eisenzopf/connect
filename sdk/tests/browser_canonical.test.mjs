import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {canonicalVectors} from './fixtures/browser_canonical_vectors.mjs';
import {validateBrowserCanonical} from '../typescript/dist/browser_canonical_validation.js';
import {FROZEN_ENVELOPE_SCHEMA_JSON,FROZEN_FINITE_KINDS,FROZEN_READ_KINDS} from '../typescript/dist/browser_canonical_schema.js';
test('fixed browser schema matches all354 shared Rust schema/registry/calendar/body vectors including signed stream epochs',()=>{
 const vectors=canonicalVectors();assert.equal(vectors.length,354);
 const epochs=vectors.filter(vector=>vector.envelope.type==='stream.epoch');
 assert.deepEqual(epochs.map(({id,scope,valid})=>({id,scope,valid})),[
  {id:'schema-stream.epoch',scope:'schema-only',valid:true},
  {id:'schema-unsupported-version-stream.epoch',scope:'schema-only',valid:false},
 ]);
 assert.equal(epochs[0].envelope.v,2);assert.equal(epochs[1].envelope.v,1);
 assert.deepEqual(epochs[1].envelope,{...epochs[0].envelope,v:1});
 for(const vector of vectors){let valid=true;try{validateBrowserCanonical(vector.envelope);}catch{valid=false;}assert.equal(valid,vector.valid,vector.id);}
});
test('generated browser schema and finite policy remain bound to the frozen repository contract',()=>{
 const schema=JSON.parse(readFileSync(new URL('../../contracts/schemas/uctp-envelope-2.schema.json',import.meta.url)));assert.deepEqual(JSON.parse(FROZEN_ENVELOPE_SCHEMA_JSON),schema);assert.equal(schema.properties.type.enum.length,67);
 assert.deepEqual(schema.properties.type.enum.filter(kind=>kind==='stream.epoch'),['stream.epoch']);
 const epochFields=['profile','strm_id','stream_local_id','original_operation','original_offer','incarnation','stream_generation','connection_generation','revision','direction','epoch','ssrc','first_sequence','first_timestamp'];
 assert.deepEqual(schema.$defs.StreamEpoch.required,epochFields);assert.deepEqual(Object.keys(schema.$defs.StreamEpoch.properties),epochFields);assert.equal(schema.$defs.StreamEpoch.additionalProperties,false);
 assert.equal(schema.$defs.StreamEpoch.properties.profile.const,'rtcp-pli-epoch/1');assert.deepEqual(schema.$defs.StreamEpoch.properties.direction.enum,['publisher','receiver']);
 assert.deepEqual(schema.allOf.filter(rule=>rule.if?.properties?.type?.const==='stream.epoch'),[{if:{properties:{type:{const:'stream.epoch'}}},then:{properties:{payload:{$ref:'#/$defs/StreamEpoch'}},required:['cid','sid','connid']}}]);
 const policy=readFileSync(new URL('../../crates/connect-client/src/sdk/raw.rs',import.meta.url),'utf8').split('fn intent(')[1],reads=policy.match(/let read = matches!\(\s*kind,([\s\S]*?)\);/)[1],finite=policy.match(/if !matches!\(\s*kind,([\s\S]*?)\) \|\| kind/)[1],names=source=>[...source.matchAll(/"([a-z.-]+)"/g)].map(v=>v[1]);assert.deepEqual([...FROZEN_READ_KINDS],names(reads));assert.deepEqual([...FROZEN_FINITE_KINDS],names(finite));assert(Object.isFrozen(FROZEN_READ_KINDS)&&Object.isFrozen(FROZEN_FINITE_KINDS));assert.equal(createHash('sha256').update(readFileSync(new URL('../../contracts/schemas/uctp-envelope-2.schema.json',import.meta.url))).digest('hex'),'073c721d6800ed8d9d324b7a5d04b451c04ec7a8e221557d74a12831b4e1fad9');
});
