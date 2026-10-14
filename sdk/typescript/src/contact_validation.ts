import { fields, integer } from "./validation.js";
const states = [
    "unknown",
    "pending",
    "verified_native",
    "expired",
    "revoked",
    "attempts_exceeded",
];
export function validContact(kind: string, value: unknown): boolean {
    if (kind === "contact_pairing")
        return (
            fields(value, [
                "id",
                "owner_identity_id",
                "peer_identity_id",
                "state",
            ]) &&
            states.includes(String(value.state)) &&
            integer(value.expires_at) &&
            (!("challenge" in value) || typeof value.challenge === "string")
        );
    return (
        fields(value, ["peer_identity_id", "ownership"], ["pairing_id"]) &&
        states.includes(String(value.ownership)) &&
        value.sms_consent === false &&
        "verified_at" in value &&
        (value.verified_at === null || integer(value.verified_at))
    );
}
