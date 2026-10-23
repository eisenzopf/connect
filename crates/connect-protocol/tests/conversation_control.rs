#![allow(clippy::unwrap_used)]
use connect_protocol::{conversations::*, Operation};
use serde_json::json;
#[test]
fn conversation_metadata_duplicate_keys_and_old_intent_are_refused() {
    let input = r#"{"tenant_id":null,"policy":"persistent","idle_close_secs":null,"metadata":{"nested":{"same":1,"same":2}},"initial_participants":[]}"#;
    assert!(serde_json::from_str::<CreateRequest>(input).is_err());
    assert!(serde_json::from_value::<Operation>(
        json!({"op":"create","request_id":"create","participants":[]})
    )
    .is_err());
    assert!(serde_json::from_value::<Operation>(
        json!({"op":"close","request_id":"close","conversation_id":"conversation"})
    )
    .is_err());
    assert!(
        serde_json::from_value::<ListFilter>(json!({"state":"open","unexpected":true})).is_err()
    );
}
#[test]
fn conversation_metadata_uses_jcs_unicode_order_and_numeric_identity() {
    let mut request = CreateRequest::persistent(vec![]);
    request.metadata = Some(json!({"\u{e000}":1.0,"\u{10000}":1e-7,"one":1.0}));
    request.normalize().unwrap();
    assert_eq!(request.metadata.as_ref().unwrap()["one"], 1);
    assert_eq!(
        serde_jcs::to_string(request.metadata.as_ref().unwrap()).unwrap(),
        "{\"one\":1,\"𐀀\":1e-7,\"\":1}"
    );
    let mut request = CreateRequest::persistent(vec![]);
    request.metadata = Some(json!([[[[[[[[[[[[[[[[[0]]]]]]]]]]]]]]]]]));
    assert!(request.normalize().is_err());
}
#[test]
fn conversation_policy_bounds_are_shared_and_metadata_is_redacted() {
    let mut request = CreateRequest::persistent(vec![]);
    request.policy = ConversationPolicy::Ephemeral;
    request.metadata = Some(json!({"private":"private-value"}));
    request.normalize().unwrap();
    assert_eq!(request.idle_close_secs, Some(60));
    assert!(!format!("{request:?}").contains("private-value"));
    let mut unsafe_metadata = request.clone();
    unsafe_metadata.metadata = Some(json!(1e30));
    assert!(unsafe_metadata.normalize().is_err());
    for idle in [0, MAX_IDLE_SECONDS + 1] {
        request.idle_close_secs = Some(idle);
        assert!(request.normalize().is_err());
    }
    assert!(CloseOptions {
        reason: Some("x\nprivate".into()),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(ListQuery {
        limit: Some(101),
        ..Default::default()
    }
    .validate()
    .is_err());
}
