import { isAbsolute, join } from "node:path";
import { mkdir } from "node:fs/promises";
import { ScriptedModel } from "./model.mjs";
import { SmsFixture, fixtureBearer, fixturePublicKey } from "./sms.mjs";
import { SipFixture } from "./sip.mjs";

/** Isolated simulated peers for a future public-SDK application composition. */
export async function startFixtures({ stateDirectory, scenario }) {
    if (!isAbsolute(stateDirectory)) throw Error("explicit absolute isolated state directory required");
    await mkdir(stateDirectory, { recursive: true, mode: 0o700 });
    const resources = [];
    try {
        const model = await ScriptedModel.load();
        const sms = await new SmsFixture({ ledger: join(stateDirectory, "sms-ledger.json") }).start(); resources.push(sms);
        const telephone = {};
        for (const [actor, user] of [["owner", "+12025550101"], ["organizer", "+12025550102"], ["booker", "+12025550103"]]) {
            const peer = await new SipFixture({ user, markerHz: scenario.actors[actor].marker_hz, waitForPrompt: actor === "owner", confirmationDigit: actor === "owner" ? scenario.business_policy.phone_confirmation.digit : null }).start();
            resources.push(peer); telephone[actor] = peer;
        }
        let closing;
        const close = () => closing ??= (async () => {
            const results = await Promise.allSettled(resources.map((r) => r.close()));
            const failed = results.filter((r) => r.status === "rejected");
            if (failed.length) throw new AggregateError(failed.map((r) => r.reason), "fixture cleanup");
        })();
        return { simulated: true, model, sms, telephone, close,
            // Trusted server composition only; these public test values are not user credentials.
            backend: { sms: { url: sms.url, bearer: fixtureBearer, callback_public_key: fixturePublicKey }, telephone: Object.fromEntries(Object.entries(telephone).map(([k, p]) => [k, p.uri])) } };
    } catch (error) {
        const results = await Promise.allSettled(resources.map((r) => r.close()));
        throw new AggregateError([error, ...results.filter((r) => r.status === "rejected").map((r) => r.reason)], "fixture startup failed");
    }
}
