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

#[test]
fn resource_probes_are_weak_nonblocking_and_not_sampled_by_public_readiness() {
    use connect_protocol::{
        ResourcePoolKind, ResourceSampleState, ResourceUsage, MAX_OPERATIONAL_COUNTER,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    let state = OperationalReadiness::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let owner = state
        .register_resource_pool(ResourcePoolKind::NativeQuic, move || {
            count.fetch_add(1, Ordering::Relaxed);
            Some(ResourceUsage {
                used: 2,
                limit: 1,
                accepted: u64::MAX,
                rejected: 0,
                draining: false,
            })
        })
        .unwrap();
    let weak = Arc::downgrade(&owner);
    state.snapshot();
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    {
        let _held = state.resources.lock().unwrap();
        assert!(state.resource_pool_snapshot().is_none());
    }
    let snapshot = state.resource_pool_snapshot().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(snapshot[0].state, ResourceSampleState::Available);
    let usage = snapshot[0].usage.as_ref().unwrap();
    assert_eq!(usage.accepted, MAX_OPERATIONAL_COUNTER);
    assert_eq!(usage.used, 2); // An over-limit observation must not be hidden.
    assert!(state
        .register_resource_pool(ResourcePoolKind::NativeQuic, || None)
        .is_err());
    drop(owner);
    assert!(weak.upgrade().is_none());
    let retired = state.resource_pool_snapshot().unwrap();
    assert_eq!(retired[0].state, ResourceSampleState::Retired);
    assert!(retired[0].usage.is_none());
    let _busy = state
        .register_resource_pool(ResourcePoolKind::NativeQuic, || None)
        .unwrap();
    assert_eq!(
        state.resource_pool_snapshot().unwrap()[0].state,
        ResourceSampleState::Busy
    );
}

#[tokio::test]
async fn ordinary_and_revoked_health_do_not_sample_administrator_probes() {
    use connect_protocol::{Operation, Outcome, ResourcePoolKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(&temp.path().join("state.sqlite3"), Default::default())
            .unwrap(),
    );
    let identity = store.create_identity("health-probe-owner").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 60;
    let admin = store
        .issue(&identity.id, connect_store::Scope::operator(), expiry)
        .unwrap();
    let reader = store
        .issue(
            &identity.id,
            connect_store::Scope::messaging(vec![]),
            expiry,
        )
        .unwrap();
    let service = crate::Service::new(store.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let _owner = service
        .readiness
        .register_resource_pool(ResourcePoolKind::NativeQuic, move || {
            count.fetch_add(1, Ordering::Relaxed);
            None
        })
        .unwrap();
    assert!(matches!(
        service
            .execute_authenticated(reader.token, Operation::Health {})
            .await
            .unwrap(),
        Outcome::Health {
            operational: None,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        service
            .execute_authenticated(admin.token.clone(), Operation::Health {})
            .await
            .unwrap(),
        Outcome::Health {
            operational: Some(_),
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let actor = store.authenticate(&admin.token).unwrap();
    store.revoke(&actor, &admin.id).unwrap();
    assert_eq!(
        service
            .execute_authenticated(admin.token, Operation::Health {})
            .await
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
