declare const poolKinds: readonly ["quic-ingress", "native-quic", "webtransport", "websocket", "http-connections", "http-requests", "http-callback-connections", "http-callback-requests", "https-application-connections", "https-application-requests", "https-application-handshakes", "https-administration-connections", "https-administration-requests", "https-administration-handshakes", "https-callbacks-connections", "https-callbacks-requests", "https-callbacks-handshakes", "https-health-connections", "https-health-requests", "https-health-handshakes", "record-https-connections", "record-https-requests", "record-https-handshakes", "mcp-requests", "mcp-connections"];
declare const limitFields: readonly ["message_bytes", "page_size", "journal_events", "operation_receipts", "message_storage_bytes", "participants", "wait_ms", "request_timeout_ms", "connections", "sessions", "artifact_file_bytes", "artifact_bytes", "artifact_chunk_bytes"];
declare const operationFields: readonly ["retained", "accepted", "preparing", "ready", "committing", "interrupted", "unknown"];
export type OperationalPoolKind = (typeof poolKinds)[number];
export type OperationalLimits = Readonly<Record<(typeof limitFields)[number], number>>;
export type OperationCounts = Readonly<Record<(typeof operationFields)[number], number>>;
export interface ResourceUsage {
    used: number;
    limit: number;
    accepted: number;
    rejected: number;
    draining: boolean;
}
export interface ResourcePool {
    kind: OperationalPoolKind;
    state: "available" | "busy" | "retired";
    usage: ResourceUsage | null;
}
export interface OperationalHealth {
    observed_at: number;
    limits: OperationalLimits;
    operations: OperationCounts;
    runtime: {
        physical_owners: number;
        physical_capacity: number;
        owned_tasks: number;
    } | null;
    resource_pools: ResourcePool[] | null;
}
export declare function validOperationalHealth(v: unknown): v is OperationalHealth;
export {};
