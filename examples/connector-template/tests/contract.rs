use connect_connector_api::{
    conformance::{self, Fixture},
    lifecycle::*,
    manifest::*,
    Connector, Failure,
};
use connect_connector_template::LocalProvider;
#[tokio::test]
async fn common_contract_passes_real_fixture_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let mut provider = LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap();
    let report = conformance::run(&mut provider).await;
    assert!(report.passed(), "{report:?}");
    assert!(report.checks.len() >= 30);
}
#[test]
fn manifest_configuration_and_restart_contract_are_explicit() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap();
    let manifest = Connector::manifest(&provider);
    assert!(manifest
        .validate_configuration(&Default::default())
        .is_err());
    assert!(manifest
        .validate_configuration(&[("callback-key".into(), "env:CONNECT_FIXTURE_KEY".into())].into())
        .is_ok());
    assert!(manifest
        .validate_configuration(&[("callback-key".into(), "actual-secret-value".into())].into())
        .is_err());
    let mut registry = Registry::default();
    registry.register(manifest.clone()).unwrap();
    assert!(registry.register(manifest.clone()).is_err());
    let restored = Registry::restore(&registry.snapshot().unwrap()).unwrap();
    restored.require_compatible(&manifest).unwrap();
    let mut drift = manifest;
    drift.limits.message_bytes += 1;
    assert!(restored.require_compatible(&drift).is_err());
}
#[tokio::test]
async fn tampered_context_and_media_input_are_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap();
    let bad = MessageRequest {
        mutation_id: "too-big".into(),
        target: "fixture-peer".into(),
        body: "x".repeat(4097),
        context: vec![],
    };
    assert_eq!(provider.send(&bad).await, Err(Failure::Rejected));
    let event = Observation {
        mutation_id: "a".into(),
        external_id: Some("b".into()),
        event_id: "c".into(),
        state: State::Message(MessageState::Accepted),
        verification: Verification::AuthenticatedResponse,
    };
    let (body, signature) = provider.signed_callback(&event);
    assert!(provider.authenticate_callback(&body, &signature).is_err());
}
