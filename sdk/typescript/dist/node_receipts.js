import { object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
function protocol() {
    throw new Error("Invalid typed delivery observations");
}
const natural = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const text = (v, bound) => typeof v === "string" &&
    v.length > 0 &&
    v.length <= bound &&
    /^[\x21-\x7e]+$/.test(v);
export function deliveryObservations(value, cid, mid) {
    if (!object(value, ["conversationId", "messageId", "observations"]) ||
        value.conversationId !== cid ||
        value.messageId !== mid ||
        !Array.isArray(value.observations) ||
        value.observations.length > 4096 ||
        Buffer.byteLength(JSON.stringify(value), "utf8") > 2 * 1024 * 1024)
        return protocol();
    const seen = new Set();
    const observations = value.observations.map((v) => {
        if (!object(v, [
            "participantId",
            "membershipEpoch",
            "channel",
            "deliveryId",
            "state",
            "provenance",
            "observedAt",
        ]) ||
            !identifier(v.participantId) ||
            !natural(v.membershipEpoch) ||
            v.membershipEpoch === 0 ||
            !text(v.channel, 64) ||
            !text(v.deliveryId, 512) ||
            !natural(v.observedAt) ||
            typeof v.state !== "string" ||
            ![
                "queued",
                "accepted",
                "delivered",
                "read",
                "failed",
                "unknown",
            ].includes(v.state) ||
            typeof v.provenance !== "string" ||
            ![
                "journal",
                "client_reported",
                "provider_observation",
                "local_attempt",
            ].includes(v.provenance))
            return protocol();
        const key = JSON.stringify([
            v.participantId,
            v.membershipEpoch,
            v.channel,
            v.deliveryId,
        ]);
        if (seen.has(key))
            return protocol();
        seen.add(key);
        return Object.freeze({ ...v });
    });
    return Object.freeze(observations);
}
