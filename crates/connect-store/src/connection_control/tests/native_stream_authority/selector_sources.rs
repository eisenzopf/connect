//! Ordinary Session source diagnostics only; these rows are not physical media.
use super::*;
fn fixture(kinds: Option<Vec<&str>>) -> (Fixture, PeerAuthorityRequest) {
    let (f, mut request, _) = parent(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "selector-source".into();
    wire.payload = match kinds {
        Some(kinds) => json!({"subscriptions":[{"from_participant":f.pids[1],"kinds":kinds}]}),
        None => json!({"subscriptions":[{"from_participant":f.pids[1]}]}),
    };
    (f, request)
}
#[test]
fn wildcard_source_preflight_accepts_common_kind_without_granting_unsupported_future_media() {
    let (f, request) = fixture(None);
    let proof = admit(&f, &request).unwrap();
    let before = f.count("operations");
    f.store
        .validate_native_subscription_sources_with_peer(
            &f.keys[0],
            &proof,
            &["audio".into(), "video".into()],
        )
        .unwrap();
    assert_eq!(f.count("operations"), before);
    assert_eq!(f.count("operation_effects"), 1);
    assert_eq!(
        f.store
            .validate_native_subscription_sources_with_peer(&f.keys[0], &proof, &["video".into()])
            .unwrap_err()
            .code,
        "incompatible-capabilities"
    );
}
#[test]
fn explicit_source_kind_mismatch_and_unknown_participant_are_definitive_preflight_refusals() {
    let (f, request) = fixture(Some(vec!["video"]));
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .validate_native_subscription_sources_with_peer(
                &f.keys[0],
                &proof,
                &["audio".into(), "video".into()]
            )
            .unwrap_err()
            .code,
        "incompatible-capabilities"
    );
    drop(proof);
    let mut unknown = request;
    let PeerCommand::NativeStream(wire) = &mut unknown.command else {
        panic!()
    };
    wire.payload = json!({"subscriptions":[{"from_participant":"missing-participant"}]});
    let proof = admit(&f, &unknown).unwrap();
    assert_eq!(
        f.store
            .validate_native_subscription_sources_with_peer(&f.keys[0], &proof, &["audio".into()])
            .unwrap_err()
            .code,
        "not-found"
    );
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn selector_preflight_does_not_reuse_a_foreign_credential_or_stale_named_permission() {
    let (f, request) = fixture(None);
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .validate_native_subscription_sources_with_peer(&f.keys[1], &proof, &["audio".into()])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    let raw: String = c
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&f.keys[0]],
            |r| r.get(0),
        )
        .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&raw).unwrap();
    operations.retain(|op| op != "stream.subscribe");
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![f.keys[0], serde_json::to_string(&operations).unwrap()],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .validate_native_subscription_sources_with_peer(&f.keys[0], &proof, &["audio".into()])
            .unwrap_err()
            .code,
        "forbidden"
    );
}
