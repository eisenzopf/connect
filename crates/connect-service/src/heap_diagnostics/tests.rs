use super::*;
use std::sync::Arc;
use tracing::{instrument::WithSubscriber, Instrument};
#[path = "../../tests/support/heap_log.rs"]
mod heap_log;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_health_named_span_survives_original_blocking_branch_and_ordinary_polls_emit_no_heap_snapshot(
) {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("heap.log");
    let capture = heap_log::Capture::new(&log);
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_env_filter("connect_service::runtime_retention=debug")
        .with_writer(capture.clone())
        .finish();
    async {
        let store=Arc::new(connect_store::Store::initialize(&directory.path().join("store.sqlite3"),connect_protocol::Limits::default()).unwrap());
        let identity=store.create_identity("memory-owner").unwrap();
        let credential=store.issue(&identity.id,connect_store::Scope::operator(),chrono::Utc::now().timestamp()+60).unwrap();
        let service=crate::Service::new(store);
        for _ in 0..3 {
            let outcome=service.execute(credential.id.clone(),connect_protocol::Operation::Health{}).await.unwrap();
            assert!(matches!(outcome,connect_protocol::Outcome::Health{operational:Some(_),..}));
        }
        service.execute(credential.id.clone(),connect_protocol::Operation::Health{})
            .instrument(tracing::info_span!(target:"connect_service::runtime_retention","connect-memory-baseline"))
            .await.unwrap();
        // An ordinary Health after the diagnostic must not inherit its boundary.
        service.execute(credential.id,connect_protocol::Operation::Health{}).await.unwrap();
    }.with_subscriber(subscriber).await;
    assert!(capture.within_bound());
    drop(capture);
    let rows = heap_log::rows(&log);
    assert_eq!(rows.len(), 5);
    let samples = rows
        .iter()
        .filter(|row| !row["heap_boundary"].is_null())
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 1);
    let sample = samples[0];
    assert_eq!(
        sample["heap_boundary"]["boundary"],
        "connect-memory-baseline"
    );
    assert_eq!(sample["heap_boundary"]["pid"], std::process::id());
    assert_eq!(sample["heap_boundary"]["collection_or_reset"], false);
    assert!(sample["heap_boundary"]["raw_c_heap"].is_null());
    assert!(sample["heap_boundary"]["platform_c_allocator"]["resident_bytes"].is_null());
    assert!(sample["heap_boundary"]["platform_c_allocator"]["application_live_bytes"].is_null());
    #[cfg(any(
        all(target_arch = "aarch64", target_os = "macos"),
        all(target_arch = "aarch64", target_os = "linux", target_env = "gnu")
    ))]
    assert_eq!(
        sample["heap_boundary"]["platform_c_allocator"]["supported"],
        true
    );
    assert!(
        sample["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["calls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|call| call["source_callsite"] == "service.run:health"
                    && call["returned"].as_u64().unwrap_or(0) > 0)),
        "positive sample did not traverse actual original blocking Health work"
    );
    assert_eq!(
        sample["heap_boundary"]["standalone_store"]["events"]["snapshot_cache"]["entries"],
        0
    );
}
#[test]
fn missing_allocator_fields_and_unmeasured_owner_bytes_stay_unknown() {
    let value = allocator_scalar(
        &json!({"active_allocator":"system","stats":null,"process":null,"unsupported_reason":"disabled"}),
    );
    assert_eq!(value["stats_supported"], false);
    assert!(value["committed"]["current"].is_null());
    let value = allocator_scalar(&json!({"active_allocator":"mimalloc","stats":{
        "committed":{"current":4096,"peak":4096,"total":4096},
        "malloc_normal":{"current":0,"peak":0,"total":0},
        "malloc_huge":{"current":0,"peak":0,"total":0}}}));
    assert_eq!(value["committed"]["current"], 4096);
    assert_eq!(value["raw_malloc_normal"]["current"], 0);
    assert!(value["detailed_in_use_bytes"].is_null());
    assert!(value["detailed_in_use_status"]
        .as_str()
        .unwrap()
        .contains("MI_STAT0"));
    let value = registered_scalar(
        &json!({"kinds":[{"kind":"core.media_scene","created":2,"dropped":1,"live":1,"bytes_live":0}]}),
    );
    assert_eq!(value["kinds"][0]["live"], 1);
    assert!(value["kinds"][0]["charged_bytes_live"].is_null());
    assert!(registered_scalar(&Value::Null).is_null());
}
