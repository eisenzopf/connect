//! Explicit test-only boundary snapshots. No protocol field, retained sample,
//! allocator collection, reset, pool mutation or background task.
use serde_json::{json, Value};
const KIND_LIMIT: usize = 64;
fn integer(value: &Value) -> Value {
    if value.is_i64() || value.is_u64() {
        value.clone()
    } else {
        Value::Null
    }
}
fn counter(value: &Value) -> Value {
    json!({"current":integer(&value["current"]),"peak":integer(&value["peak"]),"total":integer(&value["total"])})
}
fn allocator_scalar(raw: &Value) -> Value {
    let stats = &raw["stats"];
    json!({"active_allocator":raw.get("active_allocator").cloned(),
        "process":raw.get("process").cloned(),"stats_supported":stats.is_object(),
        "unsupported_reason":raw.get("unsupported_reason").cloned(),
        "stat_version":integer(&stats["stat_version"]),"mimalloc_version":integer(&stats["mimalloc_version"]),
        "reserved":counter(&stats["reserved"]),"committed":counter(&stats["committed"]),
        "page_committed":counter(&stats["page_committed"]),
        "raw_malloc_normal":counter(&stats["malloc_normal"]),"raw_malloc_huge":counter(&stats["malloc_huge"]),
        "detailed_in_use_bytes":Value::Null,
        "detailed_in_use_status":"Tracking level is absent from this API. Release MI_STAT0 can emit zero allocation counters while allocations remain live.",
        "coverage":"MiMalloc reservation and commitment counters. Raw allocation counters require separate tracking-level evidence. Registered owners and platform C allocator accounting are observed separately."})
}
fn registered_scalar(raw: &Value) -> Value {
    let Some(rows) = raw["kinds"].as_array() else {
        return Value::Null;
    };
    let kinds=rows.iter().take(KIND_LIMIT).map(|row| {
        let owner=row["kind"].as_str().is_some_and(|kind|
            kind.starts_with("core.")||kind.starts_with("rtc.")||kind.starts_with("native."));
        json!({"kind":row["kind"],"created":row["created"],"dropped":row["dropped"],"live":row["live"],
            "charged_bytes_live":if owner {Value::Null}else{row["bytes_live"].clone()},
            "checkouts":row["checkouts"],"returns":row["returns"],
            "bytes_scope":if owner {"owner cardinality only; dynamic bytes unmeasured"}else{"existing instrumented pool charge; not RSS"}})
    }).collect::<Vec<_>>();
    json!({"kinds":kinds,"maximum_kinds":KIND_LIMIT,"omitted_kinds":rows.len().saturating_sub(KIND_LIMIT),
        "backtrace_samples_included":false,"concurrent_snapshot":true})
}
pub(crate) fn sample() -> Option<Value> {
    let span = tracing::Span::current();
    let name = span.metadata()?.name();
    if !matches!(
        name,
        "connect-memory-baseline" | "connect-memory-idle-final"
    ) {
        return None;
    }
    let start = std::time::Instant::now();
    // Existing snapshots allocate temporary JSON. Select scalar evidence and
    // drop raw JSON here, before the single existing log and caller return.
    let allocator = allocator_scalar(&rvoip_infra_common::memory_diagnostics::allocator_snapshot());
    let c_allocator = rvoip_infra_common::memory_diagnostics::platform_c_allocator_snapshot();
    let registered = registered_scalar(&rvoip_infra_common::memory_diagnostics::snapshot());
    Some(
        json!({"format":"connect/heap-boundary/1","boundary":name,"pid":std::process::id(),
        "allocator":allocator,"objects":registered,"sampling_elapsed_micros":start.elapsed().as_micros(),
        "snapshot_allocations":"temporary JSON/log construction; released before returning to caller",
        "collection_or_reset":false,"raw_c_heap":null,"raw_c_heap_status":"block dump unobserved",
        "platform_c_allocator":c_allocator}),
    )
}
#[cfg(test)]
mod tests;
