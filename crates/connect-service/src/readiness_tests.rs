#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn optional_absence_does_not_mask_failure_or_claim_operational_readiness() {
    let state = OperationalReadiness::default();
    state.unconfigured("mcp");
    assert_eq!(state.snapshot().state, ReadinessState::Uncertain);
    state.set("native-control", ReadinessState::Ready, "listener-bound");
    assert_eq!(state.snapshot().state, ReadinessState::Ready);
    let mcp = state
        .snapshot()
        .components
        .into_iter()
        .find(|c| c.component == "mcp")
        .unwrap();
    assert!(!mcp.configured);
    assert_eq!(mcp.reason, "not-configured");
    state.set("mcp", ReadinessState::Ready, "listener-bound");
    state.fail("mcp");
    assert_eq!(state.snapshot().state, ReadinessState::Unavailable);
    assert!(
        state
            .snapshot()
            .components
            .into_iter()
            .find(|c| c.component == "mcp")
            .unwrap()
            .configured
    );
}
