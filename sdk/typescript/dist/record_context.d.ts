/** A projected record reference is data, never delivery or memory authority. */
export interface ContextRecord {
    uuid: string;
    session_id: string;
    revision: number;
    state: "available" | "unavailable" | "deleted";
    content_hash: string | null;
    valid_until: number | null;
}
export declare function validContextRecord(value: unknown): value is ContextRecord;
