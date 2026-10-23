#![allow(clippy::unwrap_used)]
use super::*;
use crate::provider_dispatch::tests::Fixture;
use connect_connector_api::{
    manifest::{ConnectorOperation, Manifest},
    VAPI,
};
use rusqlite::params;
fn fixture() -> (Fixture, ProviderJob) {
    let f = Fixture::new();
    let mut manifest = Manifest::messaging(VAPI);
    manifest
        .operations
        .insert(ConnectorOperation::MessageStatus);
    f.store
        .register_connector_manifest(VAPI, &manifest)
        .unwrap();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &job).unwrap();
    permit.admit_effect().unwrap();
    drop(permit);
    drop(role);
    f.store
        .finish_provider_job(&job, "uncertain", None, None)
        .unwrap();
    (f, job)
}
fn admit(f: &Fixture, job: &ProviderJob) -> Result<ProviderReconciliation> {
    f.store.admit_provider_reconciliation(
        &job.id,
        "vapi",
        &AuthorityWait::new(Duration::from_secs(1)),
    )
}
#[test]
fn reconciliation_is_read_only_exact_current_and_cannot_be_forwarded_into_creation() {
    let (f, job) = fixture();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let read = admit(&f, &job).unwrap();
    assert_eq!(read.job.id, job.id);
    assert!(read.permit.check().is_ok());
    assert_eq!(
        read.permit.clone().admit_effect(),
        Err(connect_connector_api::Failure::Rejected)
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        f.store
            .admit_provider_reconciliation(
                &job.id,
                "unrelated",
                &AuthorityWait::new(Duration::from_millis(10))
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn observation_requires_registered_status_capability_existing_effect_and_runtime_owner() {
    for mode in ["missing", "undeclared", "sending", "administration"] {
        let f = Fixture::new();
        if mode != "missing" {
            let mut manifest = Manifest::messaging(VAPI);
            if mode != "undeclared" {
                manifest
                    .operations
                    .insert(ConnectorOperation::MessageStatus);
            }
            f.store
                .register_connector_manifest(VAPI, &manifest)
                .unwrap();
        }
        let job = f.claim();
        if mode != "sending" {
            f.store
                .finish_provider_job(&job, "uncertain", None, None)
                .unwrap();
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let code = if mode == "administration" {
            let admin =
                Store::open_administration(f.store.path(), connect_protocol::Limits::default())
                    .unwrap();
            admin
                .admit_provider_reconciliation(
                    &job.id,
                    "vapi",
                    &AuthorityWait::new(Duration::from_millis(10)),
                )
                .err()
                .unwrap()
                .code
        } else {
            admit(&f, &job).err().unwrap().code
        };
        assert_eq!(
            code,
            match mode {
                "missing" | "undeclared" => "unsupported-connector-operation",
                "sending" => "conflict",
                _ => "forbidden",
            }
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
#[test]
fn status_custody_blocks_actual_source_retirement_until_transport_releases_it() {
    let (f, job) = fixture();
    let custody = admit(&f, &job).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let actor = f.store.revalidate(&f.admin).unwrap();
    assert_eq!(
        f.store.revoke(&actor, &f.key).unwrap_err().code,
        "authority-busy"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(custody);
    f.store.revoke(&actor, &f.key).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(admit(&f, &job).err().unwrap().code, "unauthorized");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn status_transport_retains_retired_runtime_kernel_ownership_until_cleanup() {
    let (f, job) = fixture();
    let custody = admit(&f, &job).unwrap();
    let path = f.store.path().to_path_buf();
    f.store.require_runtime_owner().unwrap().retire();
    assert!(custody.permit.check().is_err());
    let directory = f.dir;
    drop(f.store);
    assert_eq!(
        InstallationOwnership::acquire(&path).unwrap_err().code,
        "installation-busy"
    );
    drop(custody);
    let fresh = InstallationOwnership::acquire(&path).unwrap();
    drop(fresh);
    drop(directory);
}
#[test]
fn status_deadline_uses_original_source_credential_expiry_without_changing_the_attempt() {
    let (f, job) = fixture();
    let expiry = crate::now() + 3;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.key, expiry],
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let read = admit(&f, &job).unwrap();
    assert!(
        read.permit
            .deadline()
            .saturating_duration_since(Instant::now())
            <= Duration::from_secs(3)
    );
    std::thread::sleep(
        read.permit
            .deadline()
            .saturating_duration_since(Instant::now())
            + Duration::from_millis(10),
    );
    assert!(read.permit.check().is_err());
    drop(read);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.state(&job.id), "uncertain");
}
