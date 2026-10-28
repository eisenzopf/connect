import type { ConversationId, MessageId, ParticipantId } from "./node_sdk.js";
import { object } from "./sdk_helper.js";
import type { Json } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";

/** Per-recipient/channel evidence, separately authorized from message acceptance. */
export interface SdkDeliveryObservation {
    readonly participantId: ParticipantId;
    readonly membershipEpoch: number;
    readonly channel: string;
    readonly deliveryId: string;
    readonly state:
        "queued" | "accepted" | "delivered" | "read" | "failed" | "unknown";
    readonly provenance:
        | "journal"
        | "client_reported"
        | "provider_observation"
        | "local_attempt";
    readonly observedAt: number;
}
function protocol(): never {
    throw new Error("Invalid typed delivery observations");
}
const natural = (v: Json): v is number =>
    typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const text = (v: Json, bound: number): v is string =>
    typeof v === "string" &&
    v.length > 0 &&
    v.length <= bound &&
    /^[\x21-\x7e]+$/.test(v);

export function deliveryObservations(
    value: Json,
    cid: ConversationId,
    mid: MessageId,
): readonly SdkDeliveryObservation[] {
    if (
        !object(value, ["conversationId", "messageId", "observations"]) ||
        value.conversationId !== cid ||
        value.messageId !== mid ||
        !Array.isArray(value.observations) ||
        value.observations.length > 4096 ||
        Buffer.byteLength(JSON.stringify(value), "utf8") > 2 * 1024 * 1024
    )
        return protocol();
    const seen = new Set<string>();
    const observations = value.observations.map((v) => {
        if (
            !object(v, [
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
            ].includes(v.provenance)
        )
            return protocol();
        const key = JSON.stringify([
            v.participantId,
            v.membershipEpoch,
            v.channel,
            v.deliveryId,
        ]);
        if (seen.has(key)) return protocol();
        seen.add(key);
        return Object.freeze({ ...v }) as unknown as SdkDeliveryObservation;
    });
    return Object.freeze(observations);
}
