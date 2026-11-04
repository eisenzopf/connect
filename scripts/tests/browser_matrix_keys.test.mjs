import {test} from "node:test";
import assert from "node:assert/strict";
import {generateKeyPairSync,createPrivateKey,sign,verify} from "node:crypto";
import {fixturePrivateJwk,fixtureServerJwk} from "../lib/browser_matrix_keys.mjs";
test("RFC5958 Ed25519 fixture normalization preserves the actual signing key and enrolled public binding",async()=>{
    const {privateKey,publicKey}=generateKeyPairSync("ed25519"),original=privateKey.export({format:"jwk"});
    const raw=Buffer.from(original.x,"base64url"),seed=Buffer.from(original.d,"base64url");
    const v2=Buffer.concat([Buffer.from("3051020101300506032b657004220420","hex"),seed,Buffer.from("812100","hex"),raw]);
    const normalized=fixturePrivateJwk(v2,"EdDSA",raw);
    assert.equal(normalized.d,original.d);assert.equal(normalized.x,original.x);
    const restored=createPrivateKey({key:normalized,format:"jwk"});
    const body=Buffer.from("original synthetic browser command");assert.equal(verify(null,body,publicKey,sign(null,body,restored)),true);
    assert.equal((await fixtureServerJwk(raw,"EdDSA")).x,original.x);
    assert.throws(()=>fixturePrivateJwk(v2,"EdDSA",Buffer.alloc(32)));
    assert.throws(()=>fixturePrivateJwk(Buffer.concat([v2,Buffer.from([0])]),"EdDSA",raw));
});
test("standard ES256 fixture import checks the declared cohort and exact enrolled public key",async()=>{
    const {privateKey}=generateKeyPairSync("ec",{namedCurve:"P-256"}),original=privateKey.export({format:"jwk"});
    const raw=Buffer.concat([Buffer.from([4]),Buffer.from(original.x,"base64url"),Buffer.from(original.y,"base64url")]);
    const encoded=privateKey.export({format:"der",type:"pkcs8"});
    const imported=fixturePrivateJwk(encoded,"ES256",raw);assert.equal(imported.d,original.d);
    assert.equal((await fixtureServerJwk(raw,"ES256")).y,original.y);
    assert.throws(()=>fixturePrivateJwk(encoded,"EdDSA",raw));
    const changed=Buffer.from(raw);changed[1]^=1;assert.throws(()=>fixturePrivateJwk(encoded,"ES256",changed));
});
