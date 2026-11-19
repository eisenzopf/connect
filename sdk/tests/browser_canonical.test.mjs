import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {canonicalVectors} from './fixtures/browser_canonical_vectors.mjs';
import {validateBrowserCanonical} from '../typescript/dist/browser_canonical_validation.js';
import {FROZEN_ENVELOPE_SCHEMA_JSON,FROZEN_FINITE_KINDS,FROZEN_READ_KINDS} from '../typescript/dist/browser_canonical_schema.js';
test('fixed browser schema matches all379 shared Rust schema/registry/calendar/body vectors including signed stream epochs',()=>{
 const vectors=canonicalVectors();assert.equal(vectors.length,379);assert.equal(new Set(vectors.map(vector=>vector.id)).size,379);
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
 const policy=readFileSync(new URL('../../crates/connect-client/src/sdk/raw.rs',import.meta.url),'utf8').split('fn intent(')[1],reads=policy.match(/let read = matches!\(\s*kind,([\s\S]*?)\);/)[1],finite=policy.match(/if !matches!\(\s*kind,([\s\S]*?)\) \|\| kind/)[1],names=source=>[...source.matchAll(/"([a-z.-]+)"/g)].map(v=>v[1]);assert.deepEqual([...FROZEN_READ_KINDS],names(reads));assert.deepEqual([...FROZEN_FINITE_KINDS],names(finite));assert(Object.isFrozen(FROZEN_READ_KINDS)&&Object.isFrozen(FROZEN_FINITE_KINDS));assert.equal(createHash('sha256').update(readFileSync(new URL('../../contracts/schemas/uctp-envelope-2.schema.json',import.meta.url))).digest('hex'),'81ee7f7202f67d5e778e99b84dbcc64199c4918d75a76a7ef1353329ed5c32dd');
});

test('fixed public-B schema counts all three original roles without ordering them',()=>{
 const original=canonicalVectors().find(vector=>vector.id==='public-video-ready-original-three-roles-null-native-keycounter').envelope;
 validateBrowserCanonical(original);
 const reordered=structuredClone(original);reordered.payload.streams_offered.reverse();validateBrowserCanonical(reordered);
 for(const change of [
  value=>{value.payload.streams_offered[2].direction='sendrecv';},
  value=>{value.payload.streams_offered[0].kind='video';value.payload.streams_offered[0].direction='recvonly';},
  value=>{value.payload.streams_offered.pop();},
 ]){const invalid=structuredClone(original);change(invalid);assert.throws(()=>validateBrowserCanonical(invalid));}
});
test('fixed public-B MID accepts printable ASCII and refuses whitespace controls and Unicode',()=>{
 const original=canonicalVectors().find(vector=>vector.id==='public-video-phase-negotiated-never-fed').envelope;
 for(const mid of ['0','B!~','audio-2']){const value=structuredClone(original);value.payload.webrtc_public_video_phase.binding.mid=mid;validateBrowserCanonical(value);}
 for(const mid of ['', ' ', 'mid space', 'tab\t', 'line\n', '\u007f', 'é']){const value=structuredClone(original);value.payload.webrtc_public_video_phase.binding.mid=mid;assert.throws(()=>validateBrowserCanonical(value));}
});
