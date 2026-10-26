import test from "node:test";
import assert from "node:assert/strict";
import { createPublicKey, verify } from "node:crypto";
import { mkdtemp, mkdir, writeFile, readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { SmsFixture, fixtureBearer, fixturePublicKey } from "../fixtures/sms.mjs";
const root = fileURLToPath(new URL("../../../.local/gate-6/reference-scenario/", import.meta.url)); await mkdir(root, { recursive: true });
const ledger = async () => join(await mkdtemp(join(root, "sms-")), "ledger.json");
const message = { from: "+12025550100", to: "+12025550102", text: "SIMULATED: Can I call you?", type: "SMS" };
const post = (fixture, value = message, token = fixtureBearer) => fetch(fixture.url + "/messages", { method: "POST", headers: { Authorization: `Bearer ${token}` }, body: JSON.stringify(value) });
const until = async (fn) => { const deadline = Date.now() + 2000; while (!fn()) { if (Date.now() > deadline) throw Error("fixture timeout"); await new Promise((r) => setTimeout(r, 5)); } };

test("HTTP SMS fixture refuses unauthorized, non-fixture and oversized input", async (t) => {
    const fixture = await new SmsFixture({ ledger: await ledger() }).start(); t.after(() => fixture.close());
    assert.equal((await post(fixture, message, "wrong")).status, 401);
    for (const value of [{ ...message, to: "+14155551212" }, { ...message, text: "x".repeat(1601) }, { ...message, id: "forged" }, { ...message, type: "MMS" }]) assert.equal((await post(fixture, value)).status, 422);
    assert.equal((await post(fixture, { ...message, text: "x".repeat(9000) })).status, 413);
    assert.equal(fixture.attempts, 0); assert.equal(fixture.messages.length, 0);
});
test("concurrent equal bodies are distinct durable accepted effects, never claimed delivery", async (t) => {
    const path = await ledger(); const fixture = await new SmsFixture({ ledger: path }).start(); t.after(() => fixture.close());
    const responses = await Promise.all(Array.from({ length: 12 }, () => post(fixture)));
    for (const response of responses) { assert.equal(response.status, 200); assert.equal((await response.json()).data.to[0].status, "queued"); }
    assert.equal(fixture.attempts, 12); assert.equal(new Set(fixture.messages.map((m) => m.id)).size, 12);
    assert.equal(JSON.parse(await readFile(path)).length, 12);
    await fixture.close(); await fixture.close();
    const resumed = await new SmsFixture({ ledger: path }).start(); t.after(() => resumed.close());
    assert.equal(resumed.messages.length, 12); assert.equal((await post(resumed)).status, 200);
    assert.equal(resumed.messages[12].id, resumed.messageId(13));
});
test("signed callbacks cover timestamp and exact bytes; delivery is an explicit fixture action", async (t) => {
    const fixture = await new SmsFixture({ ledger: await ledger() }).start(); t.after(() => fixture.close());
    const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(fixturePublicKey, "base64")]), format: "der", type: "spki" });
    const event = fixture.incoming("permission-1", "Yes, call me");
    const signed = Buffer.concat([Buffer.from(event.headers["telnyx-timestamp"] + "|"), event.body]);
    const signature = Buffer.from(event.headers["telnyx-signature-ed25519"], "base64");
    assert.ok(verify(null, signed, key, signature)); assert.equal(verify(null, Buffer.concat([signed, Buffer.from(" ")]), key, signature), false);
    const id = (await (await post(fixture)).json()).data.id;
    assert.equal(fixture.messages[0].state, "accepted");
    assert.equal(JSON.parse(fixture.delivery(id, "delivered").body).data.payload.to[0].status, "delivered");
    assert.throws(() => fixture.delivery(id, "probably-delivered"));
});
test("accepted effect survives withheld acknowledgement; close drains sockets without a retry", async () => {
    const path = await ledger(); const fixture = await new SmsFixture({ ledger: path, holdAcknowledgement: true }).start();
    const request = post(fixture).then(() => "ack", () => "closed");
    await until(() => fixture.pending.size === 1);
    await fixture.close(); assert.equal(await request, "closed");
    assert.equal(fixture.pending.size, 0); assert.equal(fixture.responses.size, 0);
    assert.equal(JSON.parse(await readFile(path)).length, 1); assert.equal(fixture.attempts, 1);
});
test("corrupt retained fixture ledger refuses and cannot reset original effects", async () => {
    const path = await ledger(); await writeFile(path, "{}");
    await assert.rejects(new SmsFixture({ ledger: path }).start(), /corrupt/);
    assert.throws(() => new SmsFixture({ ledger: "relative.json" }));
});
