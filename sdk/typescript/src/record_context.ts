/** A projected record reference is data, never delivery or memory authority. */
export interface ContextRecord {
    uuid: string;
    session_id: string;
    revision: number;
    state: "available" | "unavailable" | "deleted";
    content_hash: string | null;
    valid_until: number | null;
}
export function validContextRecord(value: unknown): value is ContextRecord {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
    const item = value as Record<string, unknown>;
    const keys = ["uuid", "session_id", "revision", "state", "content_hash", "valid_until"];
    return Object.keys(item).length === keys.length && keys.every((key) => key in item)
        && typeof item.uuid === "string" && typeof item.session_id === "string"
        && Number.isSafeInteger(item.revision) && Number(item.revision) > 0
        && ["available", "unavailable", "deleted"].includes(String(item.state))
        && (item.state === "available" ? typeof item.content_hash === "string" && item.content_hash.length > 0 : item.content_hash === null)
        && (item.valid_until === null || (Number.isSafeInteger(item.valid_until) && Number(item.valid_until) >= 0));
}
