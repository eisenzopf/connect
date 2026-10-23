#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use connect_protocol::{conversations::CreateRequest, Limits, Operation, Outcome};
use serde_json::json;
struct F {
    _tmp: tempfile::TempDir,
    s: Store,
    root: String,
    owner: String,
    peer: String,
    candidates: Vec<Candidate>,
}
impl F {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::initialize(&tmp.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = s.create_identity("root").unwrap().id;
        let peer = s.create_identity("peer").unwrap().id;
        let root = s
            .issue(&owner, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let mut candidates = vec![];
        for n in 0..2 {
            let Outcome::Conversation(room) = s
                .execute(
                    &root,
                    &Operation::Create {
                        request_id: format!("room-{n}"),
                        request: CreateRequest::persistent(vec![peer.clone()]),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            let member =
                crate::audience::member(&s.connection().unwrap(), &peer, &room.id, true).unwrap();
            candidates.push(
                Candidate {
                    view: IncomingContactCandidate {
                        id: String::new(),
                        route_id: format!("configured-{n}"),
                        conversation_id: room.id,
                        endpoint_id: format!("fixture-endpoint-{n}"),
                        participant_id: member.participant_id,
                        membership_epoch: member.membership_epoch,
                    },
                    identity_id: peer.clone(),
                    source: json!({"policy":"original","control_url":"PRIVATE-CONTROL-CANARY"}),
                }
                .seal()
                .unwrap(),
            );
        }
        Self {
            _tmp: tmp,
            s,
            root,
            owner,
            peer,
            candidates,
        }
    }
    fn proof<'a>(&'a self, event: &'a str, resource: &'a str) -> Capture<'a> {
        Capture {
            source: Source {
                kind: IncomingContactKind::Sms,
                provider: "telnyx",
                account_reference: "SELECTED_ACCOUNT",
                credential_id: &self.root,
                binding: json!({"configured":"source-endpoint"}),
            },
            provider_id: resource,
            event_id: event,
            event_digest: event,
            payload: json!({"body":"PRIVATE-INCOMING-BODY","from":"+15555550101","to":"+15555550102"}),
            candidates: self.candidates.clone(),
            reason: IncomingContactReason::AmbiguousRoute,
        }
    }
    fn save(&self, p: &Capture<'_>) -> Result<String> {
        let mut c = self.s.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let id = capture(&tx, p, &self.s)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(id)
    }
    fn count(&self, t: &str) -> u64 {
        self.s
            .connection()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn account_custody_holding_is_private_and_does_not_create_communication() {
    let f = F::new();
    let before = f.count("journal");
    let id = f.save(&f.proof("event", "resource")).unwrap();
    assert_eq!(f.count("journal"), before);
    assert_eq!(f.count("messages"), 0);
    assert_eq!(f.count("incoming_contact_audit"), 1);
    let view = f.s.incoming_contact(&f.root, &id).unwrap();
    assert_eq!(view.state, IncomingContactState::Held);
    assert_eq!(view.candidates.len(), 2);
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(
        !encoded.contains("PRIVATE-INCOMING-BODY") && !encoded.contains("PRIVATE-CONTROL-CANARY")
    );
}
#[test]
fn event_and_resource_replay_preserve_original_candidates_and_one_hold() {
    let f = F::new();
    let p = f.proof("event", "resource");
    let id = f.save(&p).unwrap();
    assert_eq!(f.save(&p).unwrap(), id);
    let mut replay = f.proof("retry-event", "resource");
    replay.candidates.truncate(1);
    replay.candidates[0].source = json!({"policy":"later"});
    assert_eq!(f.save(&replay).unwrap(), id);
    assert_eq!(f.count("incoming_contact_holds"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 1);
    let old = f.s.incoming_contact(&f.root, &id).unwrap();
    assert_eq!(old.candidates.len(), 2);
    assert_eq!(
        old.candidates[0].id,
        f.candidates
            .iter()
            .map(|c| c.view.id.clone())
            .min()
            .unwrap()
    );
    let mut conflicting = f.proof("event", "resource");
    conflicting.event_digest = "changed-data";
    assert_eq!(f.save(&conflicting).unwrap_err().code, "conflict");
    assert_eq!(f.count("incoming_contact_receipts"), 2);
}
#[test]
fn reused_resource_content_is_quarantined_and_account_reference_scopes_ids() {
    let f = F::new();
    let first = f.save(&f.proof("event", "same-provider-id")).unwrap();
    let mut reused = f.proof("later-event", "same-provider-id");
    reused.payload["body"] = json!("different private message");
    reused.candidates.truncate(1);
    let second = f.save(&reused).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        f.s.incoming_contact(&f.root, &second).unwrap().reason,
        IncomingContactReason::ResourceConflict
    );
    let mut other = f.proof("event", "same-provider-id");
    other.source.account_reference = "DIFFERENT_ACCOUNT";
    let third = f.save(&other).unwrap();
    assert_ne!(first, third);
    assert_eq!(f.count("incoming_contact_resources"), 2);
    assert_eq!(f.count("messages"), 0);
}
#[test]
fn current_admin_owner_recipient_scope_and_revocation_precede_read_or_replay() {
    let f = F::new();
    let id = f.save(&f.proof("event", "resource")).unwrap();
    let other = f.s.create_identity("other-admin").unwrap().id;
    let other =
        f.s.issue(&other, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
    assert!(f.s.incoming_contact(&other, &id).is_err());
    assert!(f
        .s
        .incoming_contacts(&other, None, 10)
        .unwrap()
        .contacts
        .is_empty());
    let mut scope = Scope::operator();
    scope.recipients = vec![];
    let narrowed = f.s.issue(&f.owner, scope, crate::now() + 600).unwrap().id;
    assert!(f.s.incoming_contact(&narrowed, &id).is_err());
    assert!(f
        .s
        .incoming_contacts(&narrowed, None, 10)
        .unwrap()
        .contacts
        .is_empty());
    f.s.revoke(&f.s.revalidate(&f.root).unwrap(), &f.root)
        .unwrap();
    assert!(f.s.incoming_contact(&f.root, &id).is_err());
    assert!(f.save(&f.proof("event", "resource")).is_err());
}
#[test]
fn rejected_contact_has_one_attributed_current_receipt_and_no_delivery() {
    let f = F::new();
    let id = f.save(&f.proof("event", "resource")).unwrap();
    let rejected =
        f.s.reject_incoming_contact(&f.root, "reject-once", &id, 0, "No authorized destination")
            .unwrap();
    assert_eq!(rejected.state, IncomingContactState::Rejected);
    assert_eq!(rejected.revision, 1);
    assert_eq!(
        f.s.reject_incoming_contact(&f.root, "reject-once", &id, 0, "No authorized destination")
            .unwrap(),
        rejected
    );
    assert!(f
        .s
        .reject_incoming_contact(&f.root, "reject-once", &id, 0, "Changed intent")
        .is_err());
    assert!(f
        .s
        .reject_incoming_contact(
            &f.root,
            "different-decision",
            &id,
            0,
            "No authorized destination"
        )
        .is_err());
    assert_eq!(f.count("incoming_contact_rejections"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 2);
    assert_eq!(f.count("messages"), 0);
    f.s.revoke(&f.s.revalidate(&f.root).unwrap(), &f.root)
        .unwrap();
    assert!(f
        .s
        .reject_incoming_contact(&f.root, "reject-once", &id, 0, "No authorized destination")
        .is_err());
}
#[test]
fn private_cursor_is_bounded_owner_and_scope_bound() {
    let f = F::new();
    for n in 0..3 {
        f.save(&f.proof(&format!("event-{n}"), &format!("resource-{n}")))
            .unwrap();
    }
    let page = f.s.incoming_contacts(&f.root, None, 1).unwrap();
    let token = page.next.unwrap();
    assert!(token.starts_with("ev1_"));
    let next = f.s.incoming_contacts(&f.root, Some(&token), 1).unwrap();
    assert_ne!(page.contacts[0].id, next.contacts[0].id);
    let other = f.s.create_identity("other-admin").unwrap().id;
    let other =
        f.s.issue(&other, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
    assert!(f.s.incoming_contacts(&other, Some(&token), 1).is_err());
    let mut scope = Scope::operator();
    scope.recipients = vec![f.peer.clone()];
    let limited = f.s.issue(&f.owner, scope, crate::now() + 600).unwrap().id;
    assert!(f.s.incoming_contacts(&limited, Some(&token), 1).is_err());
    assert!(f.s.incoming_contacts(&f.root, None, 0).is_err());
    assert!(f.s.incoming_contacts(&f.root, None, 33).is_err());
}
#[test]
fn receipt_audit_and_hold_rows_roll_back_together_on_storage_failure() {
    let f = F::new();
    f.s.connection().unwrap().execute_batch("CREATE TRIGGER fail_incoming_audit BEFORE INSERT ON incoming_contact_audit BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(f.save(&f.proof("event", "resource")).is_err());
    for table in [
        "incoming_contact_resources",
        "incoming_contact_holds",
        "incoming_contact_receipts",
        "incoming_contact_audit",
    ] {
        assert_eq!(f.count(table), 0);
    }
    f.s.connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_incoming_audit")
        .unwrap();
    f.save(&f.proof("event", "resource")).unwrap();
    assert_eq!(f.count("incoming_contact_holds"), 1);
}
#[test]
fn original_custody_and_rejection_audit_are_immutable_and_in_snapshot_digest() {
    let f = F::new();
    let before = Store::inspect_snapshot(f.s.path()).unwrap();
    let id = f.save(&f.proof("event", "resource")).unwrap();
    let after = Store::inspect_snapshot(f.s.path()).unwrap();
    assert_ne!(before.digest, after.digest);
    assert_eq!(after.schema, crate::SCHEMA_VERSION);
    f.s.reject_incoming_contact(&f.root, "reject", &id, 0, "Unresolved ambiguity")
        .unwrap();
    let c = f.s.connection().unwrap();
    for table in [
        "incoming_contact_resources",
        "incoming_contact_holds",
        "incoming_contact_receipts",
        "incoming_contact_rejections",
        "incoming_contact_audit",
    ] {
        assert!(c.execute(&format!("DELETE FROM {table}"), []).is_err());
    }
    assert!(c
        .execute("UPDATE incoming_contact_holds SET payload_json='{}'", [])
        .is_err());
}

#[test]
fn administrative_store_cannot_capture_provider_evidence() {
    let f = F::new();
    let administration = Store::open_administration(f.s.path(), Limits::default()).unwrap();
    let mut c = administration.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(capture(&tx, &f.proof("event", "resource"), &administration).is_err());
    drop(tx);
    drop(c);
    assert_eq!(f.count("incoming_contact_holds"), 0);
}

#[test]
fn current_candidate_epoch_and_payload_budgets_refuse_without_partial_custody() {
    let f = F::new();
    let mut stale = f.proof("event", "resource");
    stale.candidates[0].view.membership_epoch += 1;
    assert_eq!(f.save(&stale).unwrap_err().code, "forbidden");
    let mut huge = f.proof("event", "resource");
    huge.payload = json!({"body":"x".repeat(65536)});
    assert!(f.save(&huge).is_err());
    let mut duplicate = f.proof("event", "resource");
    duplicate.candidates[1] = duplicate.candidates[0].clone();
    assert!(f.save(&duplicate).is_err());
    assert_eq!(f.count("incoming_contact_resources"), 0);
    assert_eq!(f.count("incoming_contact_audit"), 0);
}
#[test]
fn custody_capacity_refuses_before_reserving_a_resource_or_event() {
    let mut f = F::new();
    f.s.limits.operation_receipts = 2;
    assert_eq!(
        f.save(&f.proof("event", "resource")).unwrap_err().code,
        "capacity-exceeded"
    );
    for table in [
        "incoming_contact_resources",
        "incoming_contact_holds",
        "incoming_contact_receipts",
        "incoming_contact_audit",
    ] {
        assert_eq!(f.count(table), 0);
    }
}

#[test]
fn delivery_sql_requires_original_candidate_real_message_and_one_resource() {
    let f = F::new();
    let id = f.save(&f.proof("event", "resource")).unwrap();
    let key =
        f.s.issue(&f.peer, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
    let send = |cid: &str, request: &str| {
        let Outcome::Accepted { message_id, .. } =
            f.s.execute(
                &key,
                &Operation::Send {
                    request_id: request.into(),
                    conversation_id: cid.into(),
                    message: connect_protocol::messages::MessageDraft::text_all(
                        "PRIVATE-INCOMING-BODY",
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        message_id
    };
    let selected = &f.candidates[0];
    let other_message = send(&f.candidates[1].view.conversation_id, "wrong-room");
    let original_message = send(&selected.view.conversation_id, "right-room");
    let c = f.s.connection().unwrap();
    let resource: String = c
        .query_row(
            "SELECT resource_id FROM incoming_contact_holds WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let insert = |snapshot: &str, message: &str, credential: &str| {
        c.execute("INSERT INTO incoming_contact_deliveries(resource_id,hold_id,candidate_id,candidate_json,message_id,outcome_json,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'fixture-resolve',?9)",params![resource,id,selected.view.id,snapshot,message,"{}",f.owner,credential,crate::now()])
    };
    let snapshot = encoded(selected).unwrap();
    assert!(insert(&snapshot, &other_message, &f.root).is_err());
    assert!(insert(&snapshot, "not-a-message", &f.root).is_err());
    assert!(insert(&snapshot, &original_message, &key).is_err());
    let mut changed = selected.clone();
    changed.source = json!({"policy":"different"});
    assert!(insert(&encoded(&changed).unwrap(), &original_message, &f.root).is_err());
    insert(&snapshot, &original_message, &f.root).unwrap();
    assert!(insert(&snapshot, &original_message, &f.root).is_err());
    assert!(c
        .execute(
            "UPDATE incoming_contact_deliveries SET outcome_json='{}'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM incoming_contact_deliveries", [])
        .is_err());
    drop(c);
    assert_eq!(
        f.s.incoming_contact(&f.root, &id).unwrap().state,
        IncomingContactState::Resolved
    );
    assert!(f
        .s
        .reject_incoming_contact(&f.root, "late-reject", &id, 0, "Already delivered")
        .is_err());
}

#[test]
fn restored_custody_preserves_opaque_cursor_rejection_and_resource_replay() {
    let f = F::new();
    let id = f.save(&f.proof("event-1", "resource-1")).unwrap();
    f.save(&f.proof("event-2", "resource-2")).unwrap();
    let rejected =
        f.s.reject_incoming_contact(&f.root, "reject-once", &id, 0, "Keep unassigned")
            .unwrap();
    let cursor =
        f.s.incoming_contacts(&f.root, None, 1)
            .unwrap()
            .next
            .unwrap();
    let backup = f._tmp.path().join("custody-backup.sqlite3");
    let snapshot = f.s.backup(&backup).unwrap();
    let bytes = std::fs::read(&backup).unwrap();
    let path = f._tmp.path().join("restored.sqlite3");
    std::fs::copy(&backup, &path).unwrap();
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert_ne!(restored.runtime_epoch(), f.s.runtime_epoch());
    assert_eq!(restored.incoming_contact(&f.root, &id).unwrap(), rejected);
    assert_eq!(
        restored
            .reject_incoming_contact(&f.root, "reject-once", &id, 0, "Keep unassigned")
            .unwrap(),
        rejected
    );
    assert_eq!(
        restored
            .incoming_contacts(&f.root, Some(&cursor), 1)
            .unwrap()
            .contacts
            .len(),
        1
    );
    let mut c = restored.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        capture(&tx, &f.proof("event-1", "resource-1"), &restored).unwrap(),
        id
    );
    tx.commit().unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM incoming_contact_audit", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(c);
    assert_eq!(std::fs::read(&backup).unwrap(), bytes);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
}
