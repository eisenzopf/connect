import type { MutationReceipt } from "./types.js";
export interface ConversationCreate {
    tenant_id: string | null;
    policy: "persistent" | "ephemeral";
    idle_close_secs: number | null;
    metadata: unknown;
    initial_participants: {
        identity_id: string;
        role: string;
    }[];
}
export interface ConversationClose {
    expected_revision: number | null;
    reason_code: number | null;
    reason: string | null;
}
export interface ConversationListQuery {
    filter?: {
        tenant_id?: string | null;
        participant_id?: string | null;
        identity_id?: string | null;
        state?: "open" | "closed" | "all";
        since?: string | null;
        until?: string | null;
    };
    cursor?: string | null;
    limit?: number | null;
}
export interface Conversation {
    id: string;
    tenant_id: string;
    policy: "persistent" | "ephemeral";
    idle_close_secs: number | null;
    metadata: unknown;
    participants: {
        participant_id: string;
        identity_id: string;
        kind: "human" | "ai" | "system" | "external";
        role: string;
        display_name: string | null;
    }[];
    opened_at: number;
    revision: number;
    closed: boolean;
    closed_at: number | null;
    reason_code: number | null;
    reason: string | null;
    receipt?: MutationReceipt;
}
export interface ConversationPage {
    conversations: Conversation[];
    next_cursor: string | null;
}
export declare function validConversation(v: unknown): v is Conversation;
