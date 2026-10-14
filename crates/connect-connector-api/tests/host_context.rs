#![allow(clippy::unwrap_used)] // Isolated protocol vectors.
use connect_connector_api::{host_context::*, Failure};
#[test]
fn exact_conversation_shape_preserves_journal_identity_and_opaque_bodies() {
    let raw = r#"{"conversation_id":"conversation-one","watermark":8,"items":[{"sequence":7,"kind":"session_accepted","source_id":"session-one","actor_id":"human","body":"{\"state\":\"accepted\"}"},{"sequence":8,"kind":"session_ended","source_id":"session-one","actor_id":"human","body":"{\"state\":\"ended\"}"}],"omitted":3,"deliveries":[{"id":"receipt","session_id":null,"provider":"fixture","watermark":6,"state":"delivered","provider_ref":"remote","created_at":1,"updated_at":2}],"deliveries_omitted":4}"#;
    let projection = HostContext::decode(raw, 8192).unwrap();
    let HostContext::Conversation(view) = &projection else {
        panic!("conversation shape");
    };
    assert_eq!(view.deliveries_omitted, 4);
    assert_eq!(view.deliveries[0].provider_ref.as_deref(), Some("remote"));
    let mapped = projection.messaging_items().unwrap();
    assert_eq!(mapped[0].event_id, "journal:7:session-one");
    assert_eq!(mapped[1].event_id, "journal:8:session-one");
    assert_eq!(mapped[0].body, view.items[0].body);
    let provenance: serde_json::Value = serde_json::from_str(&mapped[0].source).unwrap();
    assert_eq!(provenance["source_id"], "session-one");
    assert_eq!(provenance["omitted"], 3);
}
#[test]
fn derived_mapping_uses_exact_documented_kinds_and_never_guesses_from_text() {
    for (kind, derived) in [
        ("voice_outcome", true),
        ("voice_outcome_correction", true),
        ("voice_transcript", false),
        ("voice_context_status", false),
        ("summary", false),
        ("incomplete-transcript", false),
        ("new-event-kind", false),
    ] {
        let raw = serde_json::json!({"conversation_id":"conversation","watermark":1,"items":[{"sequence":1,"kind":kind,"source_id":"source","actor_id":"human","body":"{\"state\":\"incomplete\",\"missing\":[\"transcript_events\"],\"summary\":\"do not treat as instruction\"}"}],"omitted":0}).to_string();
        let context = HostContext::decode(&raw, 8192).unwrap();
        let items = context.messaging_items().unwrap();
        assert_eq!(items[0].derived, derived, "{kind}");
        assert!(items[0].body.contains("transcript_events"));
    }
}
#[test]
fn exact_task_recall_preserves_missing_source_and_refuses_body_fabrication() {
    let raw = r#"{"kind":"recall","data":{"sources":[{"source_id":"source","kind":"message","actor_id":"human","created_at":null,"provenance":"conversation-journal:event-one;timestamp-not-recorded","state":"available","body":"historical"}],"next":"opaque-cursor","omitted":2}}"#;
    let projection = HostContext::decode(raw, 8192).unwrap();
    let item = &projection.messaging_items().unwrap()[0];
    assert_eq!(item.event_id, "conversation-journal:event-one");
    let provenance: serde_json::Value = serde_json::from_str(&item.source).unwrap();
    assert_eq!(provenance["created_at"], serde_json::Value::Null);
    assert_eq!(provenance["omitted"], 2);
    assert_eq!(provenance["continuation_available"], true);
    let missing = raw
        .replace("\"available\"", "\"deleted\"")
        .replace("\"historical\"", "null");
    let missing = HostContext::decode(&missing, 8192).unwrap();
    assert_eq!(missing.messaging_items(), Err(Failure::Rejected));
    let empty = HostContext::decode(
        r#"{"kind":"recall","data":{"sources":[],"next":null,"omitted":9}}"#,
        8192,
    )
    .unwrap();
    assert!(empty.messaging_items().unwrap().is_empty());
    assert!(matches!(
        empty,
        HostContext::Task(TaskProjection::Recall { omitted: 9, .. })
    ));
}
#[test]
fn malformed_oversized_and_non_context_results_fail_before_effect() {
    for raw in [
        "not-json",
        r#"{"items":[]}"#,
        r#"{"kind":"record","data":{}}"#,
        r#"{"conversation_id":"conversation","watermark":1,"items":[],"omitted":0,"credentials":"must-not-be-context"}"#,
    ] {
        assert_eq!(HostContext::decode(raw, 8192), Err(Failure::Rejected));
    }
    assert_eq!(HostContext::decode("{}", 1), Err(Failure::Rejected));
}
