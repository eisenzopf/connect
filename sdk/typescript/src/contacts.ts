/** Native ownership is independent of channel consent and conversation access. */
export type OwnershipState =
    | "unknown"
    | "pending"
    | "verified_native"
    | "expired"
    | "revoked"
    | "attempts_exceeded";
export interface ContactPairing {
    id: string;
    owner_identity_id: string;
    peer_identity_id: string;
    expires_at: number;
    state: OwnershipState;
    /** Only the initiating owner's protected receipt contains this challenge. */
    challenge?: string;
}
export interface ContactState {
    peer_identity_id: string;
    ownership: OwnershipState;
    pairing_id: string | null;
    verified_at: number | null;
    sms_consent: false;
}
