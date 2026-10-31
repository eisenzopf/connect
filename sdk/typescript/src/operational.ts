import { object, integer } from "./validation.js";
const poolKinds = [
    "quic-ingress",
    "native-quic",
    "webtransport",
    "websocket",
    "http-connections",
    "http-requests",
    "http-callback-connections",
    "http-callback-requests",
    "https-application-connections",
    "https-application-requests",
    "https-application-handshakes",
    "https-administration-connections",
    "https-administration-requests",
    "https-administration-handshakes",
    "https-callbacks-connections",
    "https-callbacks-requests",
    "https-callbacks-handshakes",
    "https-health-connections",
    "https-health-requests",
    "https-health-handshakes",
    "record-https-connections",
    "record-https-requests",
    "record-https-handshakes",
    "mcp-requests",
] as const;
const limitFields = [
    "message_bytes",
    "page_size",
    "journal_events",
    "operation_receipts",
    "message_storage_bytes",
    "participants",
    "wait_ms",
    "request_timeout_ms",
    "connections",
    "sessions",
    "artifact_file_bytes",
    "artifact_bytes",
    "artifact_chunk_bytes",
] as const;
const operationFields = [
    "retained",
    "accepted",
    "preparing",
    "ready",
    "committing",
    "interrupted",
    "unknown",
] as const;
export type OperationalPoolKind = (typeof poolKinds)[number];
export type OperationalLimits = Readonly<
    Record<(typeof limitFields)[number], number>
>;
export type OperationCounts = Readonly<
    Record<(typeof operationFields)[number], number>
>;
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
function closed(
    v: unknown,
    keys: readonly string[],
): v is Record<string, unknown> {
    return (
        object(v) &&
        Object.keys(v).length === keys.length &&
        Object.keys(v).every((k) => keys.includes(k))
    );
}
function counters(
    v: unknown,
    keys: readonly string[],
): v is Record<string, number> {
    return closed(v, keys) && keys.every((k) => integer(v[k]));
}
function u32(v: unknown): boolean {
    return integer(v) && Number(v) <= 4294967295;
}
function pool(v: unknown): v is ResourcePool {
    if (
        !closed(v, ["kind", "state", "usage"]) ||
        !poolKinds.includes(v.kind as OperationalPoolKind)
    )
        return false;
    if (v.state === "busy" || v.state === "retired") return v.usage === null;
    return (
        v.state === "available" &&
        closed(v.usage, [
            "used",
            "limit",
            "accepted",
            "rejected",
            "draining",
        ]) &&
        u32(v.usage.used) &&
        u32(v.usage.limit) &&
        integer(v.usage.accepted) &&
        integer(v.usage.rejected) &&
        typeof v.usage.draining === "boolean"
    );
}
export function validOperationalHealth(v: unknown): v is OperationalHealth {
    if (
        !closed(v, [
            "observed_at",
            "limits",
            "operations",
            "runtime",
            "resource_pools",
        ]) ||
        !integer(v.observed_at) ||
        !counters(v.limits, limitFields) ||
        !counters(v.operations, operationFields)
    )
        return false;
    const operations = v.operations;
    const unsettled = operationFields
        .filter((k) => k !== "retained")
        .reduce((n, k) => n + operations[k], 0);
    if (v.operations.retained < Math.min(Number.MAX_SAFE_INTEGER, unsettled))
        return false;
    if (
        v.runtime !== null &&
        !(
            closed(v.runtime, [
                "physical_owners",
                "physical_capacity",
                "owned_tasks",
            ]) && Object.values(v.runtime).every(u32)
        )
    )
        return false;
    return (
        v.resource_pools === null ||
        (Array.isArray(v.resource_pools) &&
            v.resource_pools.length <= 32 &&
            v.resource_pools.every(pool) &&
            new Set(v.resource_pools.map((p) => p.kind)).size ===
                v.resource_pools.length)
    );
}
