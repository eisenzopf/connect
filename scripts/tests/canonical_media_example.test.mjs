import test from "node:test";
import assert from "node:assert/strict";
import { BrowserClient } from "../../sdk/typescript/dist/browser.js";
const { canonicalMediaClient } = await import(process.env.CONNECT_CANONICAL_MEDIA_EXAMPLE_MODULE ??
    "../../examples/conversation-client/canonical-media.js");

const keys=await crypto.subtle.generateKey("Ed25519",true,["sign","verify"]);
const configuration={identityId:"synthetic-identity",credential:"synthetic-credential",
    url:"wss://localhost/connect",device:{id:"synthetic-browser",kind:"browser",platform:"fixture"},
    signing:{algorithm:"EdDSA",keyId:"fixture-key",jwk:await crypto.subtle.exportKey("jwk",keys.privateKey)},
    server:{algorithm:"EdDSA",keyId:"fixture-host",jwk:await crypto.subtle.exportKey("jwk",keys.publicKey)},
    connection:{conversationId:"original-conversation",sessionId:"original-session",connectionId:"original-connection",
        operationId:"original-offer",expectedRevision:0,participantId:"original-participant",
        audioStreamId:"original-audio",videoStreamId:"original-video"},
    end:{operationId:"original-end",expectedRevision:2},
    subscribe:{operationId:"original-receive",expectedRevision:1,subscriptions:[{strm_id:"original-source"}]}};

async function fixture(client, run, config=configuration){
    const open=BrowserClient.open;BrowserClient.open=async()=>client;
    try {await run(await canonicalMediaClient(config));}
    finally {BrowserClient.open=open;}
}

test("example observes original offer completion before one original receive submission",async()=>{
    const calls=[];let resolveOffer;
    const offer=new Promise(resolve=>{resolveOffer=resolve});
    await fixture({
        async operationStatus(cid,id,options){calls.push(["status",cid,id]);assert(options.timeoutMs>0&&options.timeoutMs<=10000);return id==="original-offer"?offer:{state:"completed"}},
        async request(type,payload,scope,options){calls.push(["request",type,payload,scope]);assert(options.timeoutMs>0&&options.timeoutMs<=10000)},
    },async control=>{
        const first=control.receive(),second=control.receive();assert.equal(first,second);
        await Promise.resolve();assert.equal(calls.filter(c=>c[0]==="request").length,0);
        resolveOffer({state:"completed"});await first;
        assert.deepEqual(calls.map(c=>[c[0],c[0]==="status"?c[2]:c[1]]),[
            ["status","original-offer"],["request","stream.subscribe"],["status","original-receive"]]);
        assert.deepEqual(calls[1][2],{subscriptions:[{strm_id:"original-source"}]});
        assert.deepEqual(calls[1][3],{cid:"original-conversation",sid:"original-session",connid:"original-connection",
            operation_id:"original-receive",expected_revision:1});
    });
});

test("failed or cancelled original media admission never submits receive selection",async()=>{
    for(const state of ["failed","cancelled"]){
        const calls=[];
        await fixture({async operationStatus(cid,id){calls.push(id);return {state}},
            async request(){assert.fail("receive cannot start before original media admission")}},async control=>{
            const original=control.receive();await assert.rejects(original,/Original media offer/);
            assert.equal(control.receive(),original);assert.deepEqual(calls,["original-offer"]);
        });
    }
});

test("current status authority refusal prevents a new receive mutation",async()=>{
    let requests=0;
    await fixture({async operationStatus(){throw Error("current authority refused")},
        async request(){requests++}},async control=>{
        await assert.rejects(control.receive(),/current authority refused/);assert.equal(requests,0);
    });
});

test("unknown receive effect only polls its original operation and never resubmits",async()=>{
    const ids=[];let requests=0,reads=0;
    await fixture({async operationStatus(cid,id){ids.push(id);return {state:id==="original-offer"||++reads>1?"completed":"unknown"}},
        async request(){requests++}},async control=>{
        await control.receive();await control.receive();assert.equal(requests,1);
        assert.deepEqual(ids,["original-offer","original-receive","original-receive"]);
    });
});
