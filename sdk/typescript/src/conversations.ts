import type { MutationReceipt } from "./types.js";
export interface ConversationCreate {
    tenant_id: string | null;
    policy: "persistent" | "ephemeral";
    idle_close_secs: number | null;
    metadata: unknown;
    initial_participants: { identity_id: string; role: string }[];
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
const object = (v: unknown): v is Record<string, unknown> =>
    v !== null && typeof v === "object" && !Array.isArray(v);
const integer = (v: unknown): v is number =>
    typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
function metadata(v: unknown, depth = 0): boolean {
    if (depth > 16) return false;
    if (v === null || typeof v === "string" || typeof v === "boolean")
        return true;
    if (typeof v === "number")
        return (
            Number.isFinite(v) &&
            (!Number.isInteger(v) || Number.isSafeInteger(v))
        );
    if (Array.isArray(v)) return v.every((x) => metadata(x, depth + 1));
    return object(v) && Object.values(v).every((x) => metadata(x, depth + 1));
}
export function validConversation(v: unknown): v is Conversation {
    if (!object(v)) return false;
    const keys =
        "closed,closed_at,id,idle_close_secs,metadata,opened_at,participants,policy,reason,reason_code,revision,tenant_id";
    if (
        Object.keys(v)
            .filter((k) => k !== "receipt")
            .sort()
            .join(",") !== keys
    )
        return false;
    if (
        !["id", "tenant_id"].every(
            (k) => typeof v[k] === "string" && v[k] !== "",
        ) ||
        !["persistent", "ephemeral"].includes(String(v.policy))
    )
        return false;
    if (
        (v.idle_close_secs !== null &&
            (!integer(v.idle_close_secs) ||
                v.idle_close_secs < 1 ||
                v.idle_close_secs > 604800)) ||
        (v.policy === "ephemeral" && v.idle_close_secs === null)
    )
        return false;
    if (
        !metadata(v.metadata) ||
        !integer(v.opened_at) ||
        !integer(v.revision) ||
        typeof v.closed !== "boolean"
    )
        return false;
    if (
        v.closed
            ? !integer(v.closed_at) ||
              !integer(v.reason_code) ||
              typeof v.reason !== "string"
            : [v.closed_at, v.reason_code, v.reason].some((x) => x !== null)
    )
        return false;
    if (!Array.isArray(v.participants) || v.participants.length > 128)
        return false;
    const seen = new Set<string>();
    for (const p of v.participants) {
        if (
            !object(p) ||
            Object.keys(p).sort().join(",") !==
                "display_name,identity_id,kind,participant_id,role" ||
            !["participant_id", "identity_id", "role"].every(
                (k) => typeof p[k] === "string" && p[k] !== "",
            ) ||
            !["human", "ai", "system", "external"].includes(String(p.kind)) ||
            (p.display_name !== null && typeof p.display_name !== "string") ||
            seen.has(p.participant_id as string)
        )
            return false;
        seen.add(p.participant_id as string);
    }
    if ("receipt" in v) {
        const r = v.receipt;
        if (
            !object(r) ||
            Object.keys(r).sort().join(",") !==
                "completion_boundary,conversation_id,operation_id,state,status_reference" ||
            typeof r.operation_id !== "string" ||
            r.operation_id === "" ||
            r.conversation_id !== v.id ||
            r.state !== "completed" ||
            r.completion_boundary !== "local-mutation" ||
            !object(r.status_reference) ||
            Object.keys(r.status_reference).sort().join(",") !==
                "action,operation_id" ||
            r.status_reference.action !== "get" ||
            r.status_reference.operation_id !== r.operation_id
        )
            return false;
    }
    return true;
}
