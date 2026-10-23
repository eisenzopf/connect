//! Shared membership writer, receipt status and original actor authority.
#![allow(clippy::unwrap_used)]
use connect_protocol::{
    membership::*, provisioning::IdentityKind, transitions::*, Limits, Operation, Outcome,
};
use connect_store::{Scope, Store};
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    owner: String,
    peer: String,
    owner_key: String,
    peer_key: String,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let expiry = chrono::Utc::now().timestamp() + 600;
        let owner_key = store
            .issue(&owner, Scope::collaboration(vec!["*".into()]), expiry)
            .unwrap()
            .id;
        let peer_key = store
            .issue(&peer, Scope::collaboration(vec!["*".into()]), expiry)
            .unwrap()
            .id;
        let Outcome::Conversation(c) = store
            .execute(
                &owner_key,
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        Self {
            _dir: dir,
            store,
            owner,
            peer,
            owner_key,
            peer_key,
            cid: c.id,
        }
    }
    fn req(&self, id: &str, change: MembershipAction) -> MembershipRequest {
        MembershipRequest {
            operation_id: id.into(),
            conversation_id: self.cid.clone(),
            expected_revision: self
                .store
                .membership_snapshot(&self.owner_key, &self.cid)
                .unwrap()
                .revision,
            change,
        }
    }
    fn add(&self) -> MembershipRequest {
        self.req(
            "add",
            MembershipAction::Add {
                identity_id: self.peer.clone(),
                kind: IdentityKind::Human,
                role: "guest".into(),
            },
        )
    }
}
#[test]
fn membership_add_remove_reason_share_core_receipt_and_status() {
    let f = Fixture::new();
    let add = f.add();
    let added = f
        .store
        .execute_membership(&f.owner_key, add.clone())
        .unwrap();
    assert_eq!(
        added,
        f.store
            .execute(
                &f.owner_key,
                &Operation::Membership {
                    request: add.clone()
                }
            )
            .map(|v| match v {
                Outcome::Membership(v) => v,
                _ => panic!(),
            })
            .unwrap()
    );
    assert_eq!(added.receipt().unwrap().operation_id, "add");
    let MembershipOutcome::Participant { participant, .. } = added else {
        panic!()
    };
    let remove = f.req(
        "remove",
        MembershipAction::Remove {
            participant_id: participant.participant_id,
            reason: Some("removed by application".into()),
        },
    );
    let removed = f
        .store
        .execute_membership(&f.owner_key, remove.clone())
        .unwrap();
    let MembershipOutcome::Participant { participant, .. } = &removed else {
        panic!()
    };
    assert!(!participant.active);
    assert_eq!(
        participant.removal_reason.as_deref(),
        Some("removed by application")
    );
    assert_eq!(
        removed,
        f.store.execute_membership(&f.owner_key, remove).unwrap()
    );
    let record = f
        .store
        .execute_operation_action(
            &f.owner_key,
            OperationAction::Get {
                operation_id: "remove".into(),
            },
        )
        .unwrap();
    assert_eq!(
        record.result.unwrap()["data"]["participant"]["removal_reason"],
        "removed by application"
    );
}
#[test]
fn membership_self_leave_receipt_does_not_revive_after_readmission() {
    let f = Fixture::new();
    let add = f.add();
    f.store.execute_membership(&f.owner_key, add).unwrap();
    let req = f.req("leave", MembershipAction::Leave {});
    let left = f
        .store
        .execute_membership(&f.peer_key, req.clone())
        .unwrap();
    assert_eq!(
        left,
        f.store
            .execute_membership(&f.peer_key, req.clone())
            .unwrap()
    );
    let record = f
        .store
        .execute_operation_action(
            &f.peer_key,
            OperationAction::Get {
                operation_id: "leave".into(),
            },
        )
        .unwrap();
    assert_eq!(
        record.result.unwrap()["data"]["participant"]["active"],
        false
    );
    let mut add = f.add();
    add.operation_id = "readmit".into();
    f.store.execute_membership(&f.owner_key, add).unwrap();
    assert_eq!(
        f.store
            .execute_membership(&f.peer_key, req)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn membership_original_manager_epoch_fences_old_add_receipt() {
    let f = Fixture::new();
    let add = f.add();
    f.store
        .execute_membership(&f.owner_key, add.clone())
        .unwrap();
    let owner = f
        .store
        .membership_snapshot(&f.owner_key, &f.cid)
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id == f.owner)
        .unwrap();
    f.store
        .execute_membership(
            &f.peer_key,
            f.req(
                "remove-owner",
                MembershipAction::Remove {
                    participant_id: owner.participant_id,
                    reason: None,
                },
            ),
        )
        .unwrap();
    let revision = f
        .store
        .membership_snapshot(&f.peer_key, &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.peer_key,
            MembershipRequest {
                operation_id: "restore-owner".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Add {
                    identity_id: f.owner.clone(),
                    kind: IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_membership(&f.owner_key, add)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.owner_key,
                OperationAction::Get {
                    operation_id: "add".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn membership_invitation_cursor_is_opaque_owner_bound_and_has_no_later_insertions() {
    let f = Fixture::new();
    for n in 0..3 {
        let room = match f
            .store
            .execute(
                &f.owner_key,
                &Operation::Create {
                    request_id: format!("inv-room-{n}"),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(v) => v,
            _ => panic!(),
        };
        f.store
            .execute_membership(
                &f.owner_key,
                MembershipRequest {
                    operation_id: format!("invite-{n}"),
                    conversation_id: room.id,
                    expected_revision: room.revision,
                    change: MembershipAction::Invite {
                        identity_id: f.peer.clone(),
                        kind: IdentityKind::Human,
                        role: "guest".into(),
                        expires_at: chrono::Utc::now().timestamp() + 300,
                    },
                },
            )
            .unwrap();
    }
    let page = f
        .store
        .membership_invitations(&f.peer_key, None, 1)
        .unwrap();
    assert_eq!(page.invitations.len(), 1);
    let cursor = page.next.unwrap();
    assert!(cursor.starts_with("ev1_"));
    assert!(!cursor.contains(&f.peer));
    let next = f
        .store
        .membership_invitations(&f.peer_key, Some(&cursor), 100)
        .unwrap();
    assert_eq!(next.invitations.len(), 2);
    assert!(next.next.is_none());
    assert_eq!(
        f.store
            .membership_invitations(&f.owner_key, Some(&cursor), 1)
            .unwrap_err()
            .code,
        "invalid-cursor"
    );
    let rotated = f
        .store
        .issue(
            &f.peer,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .id;
    assert_eq!(
        f.store
            .membership_invitations(&rotated, Some(&cursor), 1)
            .unwrap_err()
            .code,
        "invalid-cursor"
    );
    assert_eq!(
        f.store
            .membership_invitations(&f.peer_key, Some("tampered"), 1)
            .unwrap_err()
            .code,
        "invalid-cursor"
    );
    let add = f.req(
        "invite-after-page",
        MembershipAction::Invite {
            identity_id: f.peer.clone(),
            kind: IdentityKind::Human,
            role: "member".into(),
            expires_at: chrono::Utc::now().timestamp() + 300,
        },
    );
    f.store.execute_membership(&f.owner_key, add).unwrap();
    assert_eq!(
        f.store
            .membership_invitations(&f.peer_key, Some(&cursor), 100)
            .unwrap()
            .invitations
            .len(),
        2
    );
}
#[test]
fn membership_invitation_target_and_nonmember_administrator_receipts_are_explicit() {
    let f = Fixture::new();
    let invite = f.req(
        "invite.target",
        MembershipAction::Invite {
            identity_id: f.peer.clone(),
            kind: IdentityKind::Human,
            role: "guest".into(),
            expires_at: chrono::Utc::now().timestamp() + 300,
        },
    );
    let MembershipOutcome::Invitation { invitation, .. } =
        f.store.execute_membership(&f.owner_key, invite).unwrap()
    else {
        panic!()
    };
    let decline = MembershipRequest {
        operation_id: "decline~own".into(),
        conversation_id: f.cid.clone(),
        expected_revision: invitation.conversation_revision,
        change: MembershipAction::Decline {
            invitation_id: invitation.invitation_id,
        },
    };
    let value = f
        .store
        .execute_membership(&f.peer_key, decline.clone())
        .unwrap();
    assert_eq!(
        value,
        f.store.execute_membership(&f.peer_key, decline).unwrap()
    );
    assert!(f
        .store
        .execute_operation_action(
            &f.peer_key,
            OperationAction::Get {
                operation_id: "decline~own".into()
            }
        )
        .is_ok());
    let admin_id = f.store.create_identity("administrator").unwrap().id;
    let admin = f
        .store
        .issue(
            &admin_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .id;
    let request = f.req(
        "admin-add",
        MembershipAction::Add {
            identity_id: f.peer.clone(),
            kind: IdentityKind::Human,
            role: "guest".into(),
        },
    );
    let value = f.store.execute_membership(&admin, request.clone()).unwrap();
    assert_eq!(
        value,
        f.store.execute_membership(&admin, request.clone()).unwrap()
    );
    assert!(f
        .store
        .execute_operation_action(
            &admin,
            OperationAction::Get {
                operation_id: "admin-add".into()
            }
        )
        .is_ok());
    let narrow = f
        .store
        .issue(
            &admin_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .id;
    assert_eq!(
        f.store
            .execute_operation_action(
                &narrow,
                OperationAction::Get {
                    operation_id: "admin-add".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn membership_business_rejection_is_retained_but_receipt_storage_failure_rolls_back_every_effect() {
    let f = Fixture::new();
    let mut stale = f.add();
    stale.operation_id = "stale-add".into();
    stale.expected_revision += 1;
    assert_eq!(
        f.store
            .execute_membership(&f.owner_key, stale.clone())
            .unwrap_err()
            .code,
        "conflict"
    );
    let failed = f
        .store
        .execute_operation_action(
            &f.owner_key,
            OperationAction::Get {
                operation_id: stale.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    let connection = rusqlite::Connection::open(f.store.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_membership_receipt BEFORE INSERT ON operation_control_receipts WHEN NEW.operation_record_id IN (SELECT record_id FROM operations WHERE operation_id='add') BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END").unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let add = f.add();
    assert_eq!(
        f.store
            .execute_membership(&f.owner_key, add.clone())
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    connection
        .execute_batch("DROP TRIGGER fail_membership_receipt")
        .unwrap();
    assert!(f.store.execute_membership(&f.owner_key, add).is_ok());
    assert_eq!(
        f.store
            .execute_membership(&f.owner_key, stale)
            .unwrap_err()
            .code,
        "conflict"
    );
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='add'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
#[test]
fn membership_concurrent_retry_is_one_receipt_and_restart_keeps_removal_reason_and_epoch() {
    let f = Fixture::new();
    let add = f.add();
    let path = f.store.path().to_path_buf();
    let store = std::sync::Arc::new(f.store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut jobs = vec![];
    for _ in 0..2 {
        let store = store.clone();
        let b = barrier.clone();
        let key = f.owner_key.clone();
        let add = add.clone();
        jobs.push(std::thread::spawn(move || {
            b.wait();
            store.execute_membership(&key, add).unwrap()
        }));
    }
    barrier.wait();
    let first = jobs.remove(0).join().unwrap();
    assert_eq!(jobs.remove(0).join().unwrap(), first);
    let MembershipOutcome::Participant {
        participant,
        revision,
        ..
    } = first
    else {
        panic!()
    };
    let remove = MembershipRequest {
        operation_id: "remove-before-restart".into(),
        conversation_id: f.cid.clone(),
        expected_revision: revision,
        change: MembershipAction::Remove {
            participant_id: participant.participant_id,
            reason: Some("persisted reason".into()),
        },
    };
    let expected = store
        .execute_membership(&f.owner_key, remove.clone())
        .unwrap();
    drop(store);
    let reopened = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        reopened.execute_membership(&f.owner_key, remove).unwrap(),
        expected
    );
    reopened.integrity_check().unwrap();
}
