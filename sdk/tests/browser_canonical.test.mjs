import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {canonicalVectors} from './fixtures/browser_canonical_vectors.mjs';
import {validateBrowserCanonical} from '../typescript/dist/browser_canonical_validation.js';
import {FROZEN_ENVELOPE_SCHEMA_JSON,FROZEN_FINITE_KINDS,FROZEN_READ_KINDS} from '../typescript/dist/browser_canonical_schema.js';
test('fixed browser schema matches all339 shared Rust schema/registry/calendar/body vectors',()=>{
 const vectors=canonicalVectors();assert.equal(vectors.length,339);for(const vector of vectors){let valid=true;try{validateBrowserCanonical(vector.envelope);}catch{valid=false;}assert.equal(valid,vector.valid,vector.id);}
});
test('generated browser schema and finite policy remain bound to the frozen repository contract',()=>{
 const schema=JSON.parse(readFileSync(new URL('../../contracts/schemas/uctp-envelope-2.schema.json',import.meta.url)));assert.deepEqual(JSON.parse(FROZEN_ENVELOPE_SCHEMA_JSON),schema);assert.equal(schema.properties.type.enum.length,66);
 const policy=readFileSync(new URL('../../crates/connect-client/src/sdk/raw.rs',import.meta.url),'utf8').split('fn intent(')[1],reads=policy.match(/let read = matches!\(\s*kind,([\s\S]*?)\);/)[1],finite=policy.match(/if !matches!\(\s*kind,([\s\S]*?)\) \|\| kind/)[1],names=source=>[...source.matchAll(/"([a-z.-]+)"/g)].map(v=>v[1]);assert.deepEqual([...FROZEN_READ_KINDS],names(reads));assert.deepEqual([...FROZEN_FINITE_KINDS],names(finite));assert(Object.isFrozen(FROZEN_READ_KINDS)&&Object.isFrozen(FROZEN_FINITE_KINDS));assert.equal(createHash('sha256').update(readFileSync(new URL('../../contracts/schemas/uctp-envelope-2.schema.json',import.meta.url))).digest('hex'),'d10a4e7ccbd4d025ea964c5109e9a479b614ac909caf46da2efb964ec9c81f39');
});
