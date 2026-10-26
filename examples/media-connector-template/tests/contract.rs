#[path = "../conformance/fixture.rs"]
mod fixture;

#[tokio::test]
async fn reusable_media_harness_exercises_all_eight_transport_behaviors() {
    let report = connect_media_conformance::run::<fixture::Template>().await;
    assert!(report.passed(), "{report:?}");
}
#[tokio::test]
async fn reusable_media_harness_rejects_lost_current_and_readonly_custody() {
    let report = connect_media_conformance::run::<fixture::BrokenCustody>().await;
    assert!(!report.passed());
    for expected in [
        "staged_admission_uses_current_effect_custody_and_exact_original_intent",
        "revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health",
    ] {
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == expected && !check.passed));
    }
}
