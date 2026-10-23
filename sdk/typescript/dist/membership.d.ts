import type { MemberView } from "./events.js";
import type { MutationReceipt } from "./types.js";
export type MembershipAction = {
    action: "add";
    identity_id: string;
    kind: MemberView["kind"];
    role: string;
} | {
    action: "invite";
    identity_id: string;
    kind: MemberView["kind"];
    role: string;
    expires_at: number;
} | {
    action: "join" | "decline" | "cancel_invitation";
    invitation_id: string;
} | {
    action: "leave";
} | {
    action: "remove";
    participant_id: string;
    reason?: string | null;
} | {
    action: "update_role";
    participant_id: string;
    role: string;
} | {
    action: "share_history";
    resources: {
        kind: "message" | "artifact" | "session" | "call" | "sip_leg" | "context_delivery" | "record" | "transcript" | "observation";
        id: string;
    }[];
    participants: string[];
    provenance: string;
} | {
    action: "revoke_history_share";
    sharing_id: string;
};
export interface MembershipRequest {
    operation_id: string;
    conversation_id: string;
    expected_revision: number;
    change: MembershipAction;
}
export interface MembershipInvitation {
    invitation_id: string;
    conversation_id: string;
    identity_id: string;
    expires_at: number;
    state: "pending" | "joined" | "declined" | "cancelled" | "expired";
    conversation_revision: number;
}
export type MembershipOutcome = {
    kind: "participant";
    participant: MemberView;
    revision: number;
    receipt: MutationReceipt;
} | {
    kind: "invitation";
    invitation: MembershipInvitation;
    receipt: MutationReceipt;
} | {
    kind: "sharing";
    sharing_id: string;
    conversation_id: string;
    revision: number;
    active: boolean;
    receipt: MutationReceipt;
};
export interface MembershipSnapshot {
    conversation_id: string;
    revision: number;
    participants: MemberView[];
}
export interface MembershipInvitations {
    invitations: MembershipInvitation[];
    next: string | null;
}
