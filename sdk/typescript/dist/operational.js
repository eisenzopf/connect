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
];
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
];
const operationFields = [
    "retained",
    "accepted",
    "preparing",
    "ready",
    "committing",
    "interrupted",
    "unknown",
];
function closed(v, keys) {
    return (object(v) &&
        Object.keys(v).length === keys.length &&
        Object.keys(v).every((k) => keys.includes(k)));
}
function counters(v, keys) {
    return closed(v, keys) && keys.every((k) => integer(v[k]));
}
function u32(v) {
    return integer(v) && Number(v) <= 4294967295;
}
function pool(v) {
    if (!closed(v, ["kind", "state", "usage"]) ||
        !poolKinds.includes(v.kind))
        return false;
    if (v.state === "busy" || v.state === "retired")
        return v.usage === null;
    return (v.state === "available" &&
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
        typeof v.usage.draining === "boolean");
}
export function validOperationalHealth(v) {
    if (!closed(v, [
        "observed_at",
        "limits",
        "operations",
        "runtime",
        "resource_pools",
    ]) ||
        !integer(v.observed_at) ||
        !counters(v.limits, limitFields) ||
        !counters(v.operations, operationFields))
        return false;
    const operations = v.operations;
    const unsettled = operationFields
        .filter((k) => k !== "retained")
        .reduce((n, k) => n + operations[k], 0);
    if (v.operations.retained < Math.min(Number.MAX_SAFE_INTEGER, unsettled))
        return false;
    if (v.runtime !== null &&
        !(closed(v.runtime, [
            "physical_owners",
            "physical_capacity",
            "owned_tasks",
        ]) && Object.values(v.runtime).every(u32)))
        return false;
    return (v.resource_pools === null ||
        (Array.isArray(v.resource_pools) &&
            v.resource_pools.length <= 32 &&
            v.resource_pools.every(pool) &&
            new Set(v.resource_pools.map((p) => p.kind)).size ===
                v.resource_pools.length));
}
