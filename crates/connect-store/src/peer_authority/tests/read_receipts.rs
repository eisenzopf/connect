use super::*;
use connect_protocol::{membership::*, receipts::ClientReceiptState, transitions::OperationState};
struct ReadFixture {
    f: Fixture,
    sender: String,
    sender_credential: String,
    command: PeerReadReceiptCommand,
}
impl ReadFixture {
    fn new() -> Self {
        let f = Fixture::new(300);
        let sender = f.store.create_identity("receipt-sender").unwrap().id;
        let sender_credential = f
            .store
            .issue(
                &sender,
                Scope::collaboration(vec!["*".into()]),
                crate::now() + 300,
            )
            .unwrap()
            .id;
        let snapshot = f.store.membership_snapshot(&f.user, &f.cid).unwrap();
        f.store
            .execute_membership(
                &f.user,
                MembershipRequest {
                    operation_id: "add-receipt-sender".into(),
                    conversation_id: f.cid.clone(),
                    expected_revision: snapshot.revision,
                    change: MembershipAction::Add {
                        identity_id: sender.clone(),
                        kind: IdentityKind::Human,
                        role: "participant".into(),
                    },
                },
            )
            .unwrap();
        let mut message = MessageDraft::text_all("read report source");
        message.msg_id = Some("receipt-source".into());
        f.store
            .execute(
                &sender_credential,
                &Operation::Send {
                    request_id: "receipt-source-send".into(),
                    conversation_id: f.cid.clone(),
                    message,
                },
            )
            .unwrap();
        f.store
            .execute(
                &f.user,
                &Operation::ReceiptPreferencesSet {
                    request_id: "receipt-preferences".into(),
                    conversation_id: f.cid.clone(),
                    delivery: true,
                    read: true,
                },
            )
            .unwrap();
        let participant = f
            .store
            .membership_snapshot(&f.user, &f.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id == f.identity)
            .unwrap()
            .participant_id;
        let command = PeerReadReceiptCommand {
            operation_id: "canonical-read".into(),
            conversation_id: f.cid.clone(),
            message_id: "receipt-source".into(),
            by_participant: Some(participant),
            read_at: chrono::DateTime::from_timestamp(1, 0).unwrap(),
        };
        Self {
            f,
            sender,
            sender_credential,
            command,
        }
    }
    fn request(&self, command: &PeerReadReceiptCommand) -> PeerAuthorityRequest {
        let mut request = self.f.request.clone();
        request.command = PeerCommand::ReadReceipt(command.clone());
        request
    }
    fn counts(&self) -> (i64, i64) {
        let c = self.f.store.connection().unwrap();
        (
            c.query_row(
                "SELECT COUNT(*) FROM message_receipts WHERE message_id='receipt-source'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
            c.query_row(
                "SELECT COUNT(*) FROM journal WHERE kind='message_receipt'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        )
    }
}
#[test]
fn canonical_read_receipt_exact_claim_and_original_recipient_share_control_ledger() {
    let r = ReadFixture::new();
    let f = &r.f;
    let native = f.store.execute(&f.user, &r.command.operation()).unwrap();
    let counts = r.counts();
    // Acceptance retained its queued fact; the report adds delivered and read.
    assert_eq!(counts, (3, 2));
    let proof = f.admit(&r.request(&r.command)).unwrap();
    let record = f
        .store
        .execute_read_receipt_with_peer(&f.user, &r.command, &proof)
        .unwrap();
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(record.result, Some(serde_json::to_value(native).unwrap()));
    assert_eq!(r.counts(), counts);
    for field in 0..5 {
        let mut changed = r.command.clone();
        match field {
            0 => changed.operation_id = "other".into(),
            1 => changed.conversation_id = "other".into(),
            2 => changed.message_id = "other".into(),
            3 => changed.by_participant = None,
            _ => changed.read_at += chrono::Duration::seconds(1),
        };
        assert_eq!(
            f.store
                .execute_read_receipt_with_peer(&f.user, &changed, &proof)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert_eq!(r.counts(), counts);
    drop(proof);
    // A fresh signed report can change its diagnostic time while keeping the
    // same logical read intent; this never appends another receipt fact.
    let mut later = r.command.clone();
    later.read_at = chrono::Utc::now();
    later.by_participant = None;
    let proof = f.admit(&r.request(&later)).unwrap();
    assert_eq!(
        f.store
            .execute_read_receipt_with_peer(&f.user, &later, &proof)
            .unwrap()
            .record_id,
        record.record_id
    );
    assert_eq!(r.counts(), counts);
    drop(proof);
    let mut spoofed = r.command.clone();
    spoofed.by_participant = Some("other-participant".into());
    assert_eq!(f.admit(&r.request(&spoofed)).unwrap_err().code, "forbidden");
    let foreign = Fixture::new(300);
    let foreign_proof = foreign.admit(&foreign.request).unwrap();
    assert_eq!(
        f.store
            .execute_read_receipt_with_peer(&f.user, &r.command, &foreign_proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    // Being the sender or a manager is not original recipient authority.
    assert_eq!(
        f.store
            .execute(
                &r.sender_credential,
                &Operation::MessageReceipt {
                    request_id: "sender-spoof".into(),
                    conversation_id: f.cid.clone(),
                    message_id: r.command.message_id.clone(),
                    state: ClientReceiptState::Read
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn canonical_read_receipt_keeps_departed_sender_fenced_without_reader_reentry() {
    let r = ReadFixture::new();
    let f = &r.f;
    let snapshot = f
        .store
        .membership_snapshot(&r.sender_credential, &f.cid)
        .unwrap();
    f.store
        .execute_membership(
            &r.sender_credential,
            MembershipRequest {
                operation_id: "sender-leave".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::Leave {},
            },
        )
        .unwrap();
    let proof = f.admit(&r.request(&r.command)).unwrap();
    assert!(f.store.authority_covers(
        &proof._scopes,
        &[AuthorityScope::identity(&r.sender, AuthorityMode::Read).unwrap()]
    ));
    let store = f.store.clone();
    let admin = f.admin.clone();
    let sender = r.sender.clone();
    let writer = std::thread::spawn(move || {
        store.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeIdentity {
                operation_id: "retire-receipt-sender".into(),
                identity_id: sender,
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&r.sender, AuthorityMode::Write).unwrap(),
    );
    let started = Instant::now();
    let record = f
        .store
        .execute_read_receipt_with_peer(&f.user, &r.command, &proof)
        .unwrap();
    assert_eq!(record.state, OperationState::Completed);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(!writer.is_finished());
    drop(proof);
    writer.join().unwrap().unwrap();
}
#[test]
fn canonical_read_receipt_source_expiring_behind_sql_wait_leaves_no_fact_or_ledger_effect() {
    let r = ReadFixture::new();
    let f = &r.f;
    let mut proof = f.admit(&r.request(&r.command)).unwrap();
    proof.limit_expiry(crate::now() + 1).unwrap();
    let before = r.counts();
    let c = f.store.connection().unwrap();
    let store = f.store.clone();
    let credential = f.user.clone();
    let command = r.command.clone();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let worker_entered = entered.clone();
    let worker = std::thread::spawn(move || {
        worker_entered.wait();
        store.execute_read_receipt_with_peer(&credential, &command, &proof)
    });
    entered.wait();
    std::thread::sleep(Duration::from_millis(1100));
    drop(c);
    assert_eq!(worker.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(r.counts(), before);
    let c = f.store.connection().unwrap();
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='canonical-read'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn canonical_read_receipt_readmission_and_explicit_history_do_not_restore_report_authority() {
    let r = ReadFixture::new();
    let f = &r.f;
    let proof = f.admit(&r.request(&r.command)).unwrap();
    f.store
        .execute_read_receipt_with_peer(&f.user, &r.command, &proof)
        .unwrap();
    drop(proof);
    let before = r.counts();
    let snapshot = f.store.membership_snapshot(&f.user, &f.cid).unwrap();
    f.store
        .execute_membership(
            &f.user,
            MembershipRequest {
                operation_id: "reporter-leave".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::Leave {},
            },
        )
        .unwrap();
    let snapshot = f
        .store
        .membership_snapshot(&r.sender_credential, &f.cid)
        .unwrap();
    f.store
        .execute_membership(
            &f.admin,
            MembershipRequest {
                operation_id: "reporter-readmit".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::Add {
                    identity_id: f.identity.clone(),
                    kind: IdentityKind::Human,
                    role: "reviewer".into(),
                },
            },
        )
        .unwrap();
    let snapshot = f
        .store
        .membership_snapshot(&r.sender_credential, &f.cid)
        .unwrap();
    let subject = snapshot
        .participants
        .iter()
        .find(|p| p.identity_id == f.identity)
        .unwrap();
    assert_eq!(subject.membership_epoch, 2);
    f.store
        .execute_membership(
            &r.sender_credential,
            MembershipRequest {
                operation_id: "share-prior-report-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: r.command.message_id.clone(),
                    }],
                    participants: vec![subject.participant_id.clone()],
                    provenance: "explicit retained history".into(),
                },
            },
        )
        .unwrap();
    let Outcome::History { messages, .. } = f
        .store
        .execute(
            &f.user,
            &Operation::History {
                conversation_id: f.cid.clone(),
                after: None,
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(messages
        .iter()
        .any(|message| message.id == r.command.message_id));
    assert_eq!(
        f.admit(&r.request(&r.command)).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute(&f.user, &r.command.operation())
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.user,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: r.command.operation_id.clone()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(r.counts(), before);
}

#[test]
fn canonical_read_receipt_storage_failure_rolls_back_fact_and_ledger_then_same_id_recovers() {
    let r = ReadFixture::new();
    let f = &r.f;
    let before = r.counts();
    let proof = f.admit(&r.request(&r.command)).unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_receipt_journal BEFORE INSERT ON journal WHEN NEW.kind='message_receipt' BEGIN SELECT RAISE(ABORT,'fixture receipt storage failure'); END;").unwrap();
    assert_eq!(
        f.store
            .execute_read_receipt_with_peer(&f.user, &r.command, &proof)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(r.counts(), before);
    let c = f.store.connection().unwrap();
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='canonical-read'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    c.execute_batch("DROP TRIGGER refuse_receipt_journal;")
        .unwrap();
    drop(c);
    let result = f
        .store
        .execute_read_receipt_with_peer(&f.user, &r.command, &proof)
        .unwrap();
    assert_eq!(result.state, OperationState::Completed);
    assert_eq!(r.counts(), (3, 2));
}
