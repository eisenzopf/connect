// Normalize synthetic enrolled fixture keys in memory, before passing them to a page.
// No credentials, key bytes or JWKs belong in public evidence.
import {createPrivateKey,webcrypto} from "node:crypto";
import assert from "node:assert/strict";
const edAlgorithm=Buffer.from("300506032b6570","hex");
function field(bytes,offset) {
    assert.ok(offset+2<=bytes.length,"truncated fixture DER");
    const start=offset,tag=bytes[offset++];let size=bytes[offset++];
    if(size&128){const count=size&127;assert.ok(count>=1&&count<=2&&offset+count<=bytes.length,"bounded definite fixture DER");
        size=0;for(let n=0;n<count;n++)size=size*256+bytes[offset++];assert.ok(size>=128,"minimal fixture DER length");}
    assert.ok(offset+size<=bytes.length,"truncated fixture DER body");
    return {tag,body:bytes.subarray(offset,offset+size),raw:bytes.subarray(start,offset+size),end:offset+size};
}
function edV1(bytes) {
    const outer=field(bytes,0);assert.equal(outer.tag,48);assert.equal(outer.end,bytes.length);
    const version=field(outer.body,0),algorithm=field(outer.body,version.end),privateKey=field(outer.body,algorithm.end);
    assert.equal(version.tag,2);assert.deepEqual(version.body,Buffer.from([1]));
    assert.deepEqual(algorithm.raw,edAlgorithm);assert.equal(privateKey.tag,4);
    const seed=field(privateKey.body,0);assert.equal(seed.tag,4);assert.equal(seed.body.length,32);assert.equal(seed.end,privateKey.body.length);
    const publicKey=field(outer.body,privateKey.end);assert.equal(publicKey.tag,129);assert.equal(publicKey.body.length,33);
    assert.equal(publicKey.body[0],0);assert.equal(publicKey.end,outer.body.length);
    const body=Buffer.concat([Buffer.from([2,1,0]),algorithm.raw,privateKey.raw]);
    return {encoded:Buffer.concat([Buffer.from([48,body.length]),body]),publicRaw:publicKey.body.subarray(1)};
}
export function fixturePrivateJwk(encoded,algorithm,publicRaw) {
    const bytes=Buffer.from(encoded),expected=Buffer.from(publicRaw);
    assert.ok(bytes.length>=32&&bytes.length<=1024,"bounded fixture private key");
    assert.ok(["EdDSA","ES256"].includes(algorithm),"explicit signing cohort");
    const outer=field(bytes,0);assert.equal(outer.tag,48);assert.equal(outer.end,bytes.length,"no trailing fixture DER");
    if(algorithm==="EdDSA"&&field(outer.body,0).body.equals(Buffer.from([1])))
        assert.deepEqual(edV1(bytes).publicRaw,expected,"embedded enrolled public key");
    let key;
    try {key=createPrivateKey({key:bytes,format:"der",type:"pkcs8"});}
    catch(error){
        if(algorithm!=="EdDSA")throw error;
        const normalized=edV1(bytes);assert.deepEqual(normalized.publicRaw,expected,"embedded enrolled public key");
        key=createPrivateKey({key:normalized.encoded,format:"der",type:"pkcs8"});
    }
    const jwk=key.export({format:"jwk"});
    if(algorithm==="EdDSA"){
        assert.equal(jwk.kty,"OKP");assert.equal(jwk.crv,"Ed25519");
        assert.deepEqual(Buffer.from(jwk.x,"base64url"),expected,"private key must derive the enrolled public key");
    }else{
        assert.equal(jwk.kty,"EC");assert.equal(jwk.crv,"P-256");
        assert.deepEqual(Buffer.concat([Buffer.from([4]),Buffer.from(jwk.x,"base64url"),Buffer.from(jwk.y,"base64url")]),expected,
            "private key must derive the enrolled public key");
    }
    return jwk;
}
export async function fixtureServerJwk(raw,algorithm) {
    const parameters=algorithm==="EdDSA"?{name:"Ed25519"}:algorithm==="ES256"?{name:"ECDSA",namedCurve:"P-256"}:null;
    assert.ok(parameters,"explicit server signing cohort");
    const key=await webcrypto.subtle.importKey("raw",Buffer.from(raw),parameters,true,["verify"]);
    return webcrypto.subtle.exportKey("jwk",key);
}
