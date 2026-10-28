import type { ConversationId, MessageId, ParticipantId } from "./node_sdk.js";
import type { Json } from "./uctp_json.js";
/** Per-recipient/channel evidence, separately authorized from message acceptance. */
export interface SdkDeliveryObservation {
    readonly participantId: ParticipantId;
    readonly membershipEpoch: number;
    readonly channel: string;
    readonly deliveryId: string;
    readonly state: "queued" | "accepted" | "delivered" | "read" | "failed" | "unknown";
    readonly provenance: "journal" | "client_reported" | "provider_observation" | "local_attempt";
    readonly observedAt: number;
}
export declare function deliveryObservations(value: Json, cid: ConversationId, mid: MessageId): readonly SdkDeliveryObservation[];
