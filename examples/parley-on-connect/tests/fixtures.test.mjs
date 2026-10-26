import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { loadScenario } from "../scenario.mjs";
import { startFixtures } from "../fixtures/index.mjs";
const root = fileURLToPath(new URL("../../../.local/gate-6/reference-scenario/", import.meta.url));
await mkdir(root, { recursive: true });
test("composed model/SIP/SMS fixtures bind only loopback and drain all owned peers", async (t) => {
    const fixtures = await startFixtures({ stateDirectory: await mkdtemp(join(root, "composition-")), scenario: await loadScenario() });
    t.after(() => fixtures.close());
    assert.equal(fixtures.simulated, true); assert.match(fixtures.backend.sms.url, /^http:\/\/127\.0\.0\.1:\d+\/v2$/);
    assert.deepEqual(Object.keys(fixtures.telephone), ["owner", "organizer", "booker"]);
    for (const peer of Object.values(fixtures.telephone)) assert.equal(peer.socket.address().address, "127.0.0.1");
    assert.equal(fixtures.telephone.owner.digit, "1"); assert.equal(fixtures.telephone.owner.waitForPrompt, true);
    assert.equal(fixtures.model.respond("proposal").simulated, true);
    await fixtures.close(); await fixtures.close();
    assert.equal(fixtures.sms.server.listening, false);
    for (const peer of Object.values(fixtures.telephone)) assert.throws(() => peer.socket.address(), /Not running/);
});
test("composition failure closes already-started SMS and SIP resources", async () => {
    const scenario = structuredClone(await loadScenario()); scenario.actors.organizer.marker_hz = -1;
    await assert.rejects(startFixtures({ stateDirectory: await mkdtemp(join(root, "failed-composition-")), scenario }), AggregateError);
    await assert.rejects(startFixtures({ stateDirectory: "relative-state", scenario }), /absolute/);
});
