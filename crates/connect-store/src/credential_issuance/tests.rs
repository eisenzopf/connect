#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use connect_protocol::{provisioning::ProvisioningCommand, Limits};
use std::sync::{mpsc, Barrier};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    admin: Actor,
    request: CredentialIssueRequest,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let owner = store.create_identity("owner").unwrap().id;
        let target = store.create_identity("agent").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let key = store
            .issue(&owner, Scope::operator(), crate::now() + 600)
            .unwrap();
        let admin = store.authenticate(&key.token).unwrap();
        Self {
            _dir: dir,
            store,
            admin,
            request: CredentialIssueRequest {
                operation_id: "issue-original".into(),
                identity_id: target,
                operations: vec!["session.handoff".into(), "operation.get".into()],
                recipients: vec![peer],
                expires_at: crate::now() + 300,
            },
        }
    }
    fn issue(&self) -> IssuedScopedCredential {
        self.store
            .issue_scoped_credential(&self.admin.credential_id, self.request.clone())
            .unwrap()
    }
    fn counts(&self) -> (u64, u64) {
        self.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM credentials),(SELECT COUNT(*) FROM operations)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
}

#[test]
fn lost_delivery_reopens_original_credential_without_secret_receipt_or_consent() {
    let f = Fixture::new();
    let original = f.issue().outcome().unwrap().clone();
    let before = f.counts();
    let mut reordered = f.request.clone();
    reordered.operations.reverse();
    reordered.operations.push("operation.get".into());
    reordered.recipients.push(reordered.recipients[0].clone());
    // Reopen on-disk state as an administrator after the first response owner is gone.
    let reopened = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let held = reopened
        .issue_scoped_credential(&f.admin.credential_id, reordered)
        .unwrap();
    assert_eq!(held.outcome().unwrap(), &original);
    assert_eq!(f.counts(), before);
    let actor = f.store.authenticate(&original.credential).unwrap();
    assert_eq!(actor.identity_id, f.request.identity_id);
    assert!(actor.permits("session.handoff"));
    assert!(!actor.permits("send"));
    assert!(!actor.permits("admin"));
    assert!(!actor.permits("credential.delegate"));
    assert_eq!(actor.expires_at, f.request.expires_at);
    assert!(f
        .store
        .credential_metadata(&actor.credential_id)
        .unwrap()
        .user_authorization
        .is_none());
    let (intent, receipt): (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT intent_json,result_json FROM operations WHERE operation_id=?1",
            [&f.request.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!intent.contains(&original.credential));
    assert!(!receipt.contains(&original.credential));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&receipt)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(format!("{held:?}"), "IssuedScopedCredential(redacted)");
    assert_eq!(format!("{original:?}"), "CredentialIssueOutcome(redacted)");
}

#[test]
fn changed_original_intent_or_same_identity_issuer_cannot_mint_replacement() {
    let f = Fixture::new();
    drop(f.issue());
    let other = f
        .store
        .issue(&f.admin.identity_id, Scope::operator(), crate::now() + 600)
        .unwrap();
    let before = f.counts();
    for change in ["scope", "recipient", "expiry", "target", "issuer"] {
        let mut request = f.request.clone();
        let mut issuer = f.admin.credential_id.as_str();
        match change {
            "scope" => request.operations = vec!["operation.get".into()],
            "recipient" => request.recipients.clear(),
            "expiry" => request.expires_at += 1,
            "target" => request.identity_id = f.admin.identity_id.clone(),
            _ => issuer = &other.id,
        }
        assert_eq!(
            f.store
                .issue_scoped_credential(issuer, request)
                .unwrap_err()
                .code,
            "conflict",
            "{change}"
        );
        assert_eq!(f.counts(), before);
    }
}

#[test]
fn admission_refuses_escalation_tenant_expiry_and_unknown_authority_fields_atomically() {
    let f = Fixture::new();
    let limited = f
        .store
        .issue(
            &f.admin.identity_id,
            Scope {
                operations: vec!["admin".into(), "operation.get".into()],
                recipients: vec![f.request.identity_id.clone()],
            },
            crate::now() + 500,
        )
        .unwrap();
    let nonadmin = f
        .store
        .issue(
            &f.admin.identity_id,
            Scope::messaging(vec!["*".into()]),
            crate::now() + 500,
        )
        .unwrap();
    let before = f.counts();
    for change in [
        "admin",
        "delegate",
        "wildcard",
        "empty",
        "expired",
        "issuer-expiry",
        "year",
        "target",
        "recipient",
        "scope-ceiling",
        "recipient-ceiling",
        "nonadmin",
    ] {
        let mut request = f.request.clone();
        let mut issuer = f.admin.credential_id.as_str();
        match change {
            "admin" => request.operations = vec!["admin".into()],
            "delegate" => request.operations = vec!["credential.delegate".into()],
            "wildcard" => request.operations = vec!["*".into()],
            "empty" => request.operations.clear(),
            "expired" => request.expires_at = crate::now() - 1,
            "issuer-expiry" => request.expires_at = crate::now() + 601,
            "year" => request.expires_at = crate::now() + 366 * 86400,
            "target" => request.identity_id = "missing".into(),
            "recipient" => request.recipients = vec!["missing".into()],
            "scope-ceiling" => {
                request.recipients.clear();
                issuer = &limited.id;
            }
            "recipient-ceiling" => {
                request.operations = vec!["operation.get".into()];
                issuer = &limited.id;
            }
            _ => issuer = &nonadmin.id,
        }
        assert!(
            f.store.issue_scoped_credential(issuer, request).is_err(),
            "{change}"
        );
        assert_eq!(f.counts(), before);
    }
    for field in [
        "issuer_credential_id",
        "installation_id",
        "user_id",
        "user_authorization",
    ] {
        let mut value = serde_json::to_value(&f.request).unwrap();
        value[field] = serde_json::json!("untrusted");
        assert!(
            serde_json::from_value::<CredentialIssueRequest>(value).is_err(),
            "{field}"
        );
    }
    let c = f.store.connection().unwrap();
    c.execute("INSERT INTO tenants(id) VALUES('other')", [])
        .unwrap();
    c.execute(
        "UPDATE identities SET tenant_id='other' WHERE id=?1",
        [&f.request.identity_id],
    )
    .unwrap();
    drop(c);
    assert!(f
        .store
        .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
        .is_err());
    assert_eq!(f.counts(), before);
}

#[test]
fn concurrent_original_requests_commit_one_credential_and_one_receipt() {
    let f = Fixture::new();
    let before = f.counts();
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let store = f.store.clone();
            let request = f.request.clone();
            let issuer = f.admin.credential_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .issue_scoped_credential(&issuer, request)
                    .unwrap()
                    .outcome()
                    .unwrap()
                    .clone()
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results[0], results[1]);
    assert_eq!(f.counts(), (before.0 + 1, before.1 + 1));
}

#[test]
fn receipt_capacity_and_transaction_failure_leave_no_unrecoverable_credential() {
    let f = Fixture::new();
    let before = f.counts();
    let mut limits = Limits::default();
    limits.operation_receipts = 0;
    let bounded = Store::open_administration(f.store.path(), limits).unwrap();
    assert_eq!(
        bounded
            .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(f.counts(), before);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_issuance_receipt BEFORE INSERT ON operations WHEN NEW.command_type='credential.issue' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
        .is_err());
    assert_eq!(f.counts(), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_issuance_receipt")
        .unwrap();
    drop(f.issue());
    assert_eq!(f.counts(), (before.0 + 1, before.1 + 1));
    // Existing original remains recoverable at capacity; a different ID is refused.
    drop(
        bounded
            .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
            .unwrap(),
    );
    let mut next = f.request.clone();
    next.operation_id = "overflow".into();
    assert_eq!(
        bounded
            .issue_scoped_credential(&f.admin.credential_id, next)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
}

#[test]
fn shared_provisioning_operation_namespace_refuses_cross_command_reuse() {
    let f = Fixture::new();
    f.store
        .execute_provisioning(
            &f.admin.credential_id,
            ProvisioningCommand::RevokeCredential {
                operation_id: f.request.operation_id.clone(),
                credential_id: f
                    .store
                    .issue(
                        &f.request.identity_id,
                        Scope::messaging(vec![]),
                        crate::now() + 100,
                    )
                    .unwrap()
                    .id,
            },
        )
        .unwrap();
    let before = f.counts();
    assert_eq!(
        f.store
            .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.counts(), before);
}

#[test]
fn delivery_owner_blocks_supported_revocation_then_replay_refuses_current_loss() {
    for subject in ["issuer", "child", "target", "recipient"] {
        let f = Fixture::new();
        let held = f.issue();
        let outcome = held.outcome().unwrap().clone();
        let command = match subject {
            "issuer" => ProvisioningCommand::RevokeCredential {
                operation_id: "revoke-issuer".into(),
                credential_id: f.admin.credential_id.clone(),
            },
            "child" => ProvisioningCommand::RevokeCredential {
                operation_id: "revoke-child".into(),
                credential_id: outcome.credential_id.clone(),
            },
            _ => ProvisioningCommand::RevokeIdentity {
                operation_id: "revoke-identity".into(),
                identity_id: if subject == "target" {
                    f.request.identity_id.clone()
                } else {
                    f.request.recipients[0].clone()
                },
                expected_revision: 0,
            },
        };
        let store = f.store.clone();
        let admin = f.admin.credential_id.clone();
        let (tx, rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            tx.send(store.execute_provisioning(&admin, command))
                .unwrap();
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "{subject}"
        );
        assert!(held.current());
        drop(held);
        rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        writer.join().unwrap();
        let before = f.counts();
        assert!(
            f.store
                .issue_scoped_credential(&f.admin.credential_id, f.request.clone())
                .is_err(),
            "{subject}"
        );
        assert_eq!(f.counts(), before);
        if subject == "issuer" {
            // Administrative issuance is independent authority, not human consent.
            assert!(f.store.authenticate(&outcome.credential).is_ok());
        } else if subject != "recipient" {
            assert!(f.store.authenticate(&outcome.credential).is_err());
        }
    }
}
