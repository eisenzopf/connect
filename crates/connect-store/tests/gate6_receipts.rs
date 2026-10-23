//! Recipient evidence cannot be forged by a sender, a route or a stale epoch.
#![allow(clippy::unwrap_used)]
use connect_protocol::{membership::*, messages::*, receipts::*, Limits, Operation, Outcome};
use connect_store::{Route, Scope, Store};
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    ids: Vec<String>,
    keys: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let mut ids = vec![];
        let mut keys = vec![];
        for alias in ["alice", "bob", "carol"] {
            let id = store.create_identity(alias).unwrap().id;
            keys.push(
                store
                    .issue(
                        &id,
                        Scope::collaboration(vec!["*".into()]),
                        chrono::Utc::now().timestamp() + 600,
                    )
                    .unwrap()
                    .id,
            );
            ids.push(id);
        }
        let Outcome::Conversation(c) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        Self {
            dir,
            store,
            ids,
            keys,
            cid: c.id,
        }
    }
    fn participant(&self, n: usize) -> String {
        self.store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id == self.ids[n])
            .unwrap()
            .participant_id
    }
    fn send(&self, id: &str, to: MessageAudience) -> String {
        let Outcome::Accepted { message_id, .. } = self
            .store
            .execute(
                &self.keys[0],
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: self.cid.clone(),
                    message: MessageDraft::text(to, "question"),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        message_id
    }
    fn private(&self) -> String {
        self.send(
            "private",
            MessageAudience::Participants(vec![self.participant(1)]),
        )
    }
    fn page(&self, n: usize, message: &str) -> ReceiptPage {
        let Outcome::MessageReceipts(p) = self
            .store
            .execute(
                &self.keys[n],
                &Operation::MessageReceipts {
                    conversation_id: self.cid.clone(),
                    message_id: message.into(),
                    after: None,
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        p
    }
    fn report(&self, n: usize, id: &str, message: &str, state: ClientReceiptState) -> Operation {
        let _ = n;
        Operation::MessageReceipt {
            request_id: id.into(),
            conversation_id: self.cid.clone(),
            message_id: message.into(),
            state,
        }
    }
    fn prefs(&self, n: usize, id: &str, delivery: bool, read: bool) {
        self.store
            .execute(
                &self.keys[n],
                &Operation::ReceiptPreferencesSet {
                    request_id: id.into(),
                    conversation_id: self.cid.clone(),
                    delivery,
                    read,
                },
            )
            .unwrap();
    }
    fn events(&self, n: usize) -> Vec<connect_protocol::Event> {
        let Outcome::Inbox { events, .. } = self
            .store
            .execute(
                &self.keys[n],
                &Operation::Inbox {
                    cursor: None,
                    limit: 100,
                    wait_ms: 0,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        events.into_iter().filter(|e| e.receipt.is_some()).collect()
    }
    fn route(&self, connector: &str) {
        let operator = self
            .store
            .issue(
                &self.ids[0],
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        self.store
            .configure_route(
                &operator.id,
                &Route {
                    id: "delivery-route".into(),
                    identity_id: self.ids[1].clone(),
                    credential_id: self.keys[1].clone(),
                    conversation_id: self.cid.clone(),
                    connector: connector.into(),
                    target: if connector == "vapi" {
                        "12345678-1234-1234-1234-123456789012".into()
                    } else {
                        "+15555550101".into()
                    },
                    source: if connector == "telnyx" {
                        Some("+15555550100".into())
                    } else {
                        None
                    },
                    enabled: true,
                    incoming_to: MessageAudience::all(),
                },
            )
            .unwrap();
    }
}
#[test]
fn queued_is_journal_acceptance_and_only_original_recipient_can_report_delivery() {
    let f = Fixture::new();
    let m = f.private();
    let initial = f.page(0, &m);
    assert_eq!(initial.receipts.len(), 1);
    assert_eq!(initial.receipts[0].state, DeliveryState::Queued);
    assert_eq!(initial.receipts[0].provenance, "journal");
    // History consumption itself proves neither delivery nor reading.
    f.store
        .execute(
            &f.keys[1],
            &Operation::History {
                conversation_id: f.cid.clone(),
                after: None,
                limit: 100,
            },
        )
        .unwrap();
    assert_eq!(f.page(0, &m), initial);
    for n in [0, 2] {
        assert_eq!(
            f.store
                .execute(
                    &f.keys[n],
                    &f.report(n, "forged", &m, ClientReceiptState::Read)
                )
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert_eq!(
        f.store
            .execute(
                &f.keys[2],
                &Operation::MessageReceipts {
                    conversation_id: f.cid.clone(),
                    message_id: m.clone(),
                    after: None,
                    limit: 100
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let op = f.report(1, "client-read-default", &m, ClientReceiptState::Read);
    let result = f.store.execute(&f.keys[1], &op).unwrap();
    assert!(matches!(
        result,
        Outcome::ReceiptAcknowledged(ReceiptAcknowledgement {
            recorded: Some(DeliveryState::Delivered),
            ..
        })
    ));
    assert_eq!(f.store.execute(&f.keys[1], &op).unwrap(), result);
    assert_eq!(f.page(0, &m).receipts[0].state, DeliveryState::Delivered);
    assert_eq!(f.events(0).len(), 1);
    assert_eq!(f.events(1).len(), 1);
    assert!(f.events(2).is_empty());
    assert_eq!(f.page(1, &m).receipts, f.page(0, &m).receipts);
    let Outcome::History { messages, .. } = f
        .store
        .execute(
            &f.keys[0],
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
    assert_eq!(messages.len(), 1);
    f.prefs(1, "later-read-opt-in", true, true);
    assert_eq!(
        f.page(0, &m).receipts[0].state,
        DeliveryState::Delivered,
        "enabling read must not backfill a previously suppressed read report"
    );
}
#[test]
fn read_opt_in_is_explicit_monotonic_and_preferences_filter_old_events_before_paging() {
    let f = Fixture::new();
    let m = f.send("all", MessageAudience::all());
    let Outcome::Inbox {
        cursor: checkpoint, ..
    } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.prefs(1, "read-on", true, true);
    let op = f.report(1, "read-report", &m, ClientReceiptState::Read);
    f.store.execute(&f.keys[1], &op).unwrap();
    f.store
        .execute(
            &f.keys[1],
            &f.report(1, "late-delivered", &m, ClientReceiptState::Delivered),
        )
        .unwrap();
    assert_eq!(f.events(0).len(), 1);
    assert_eq!(
        f.page(0, &m)
            .receipts
            .iter()
            .find(|r| r.participant_id == f.participant(1))
            .unwrap()
            .state,
        DeliveryState::Read
    );
    f.prefs(1, "all-off", false, false);
    assert!(f.events(0).is_empty());
    assert!(f.events(1).is_empty());
    let p = f.page(0, &m);
    assert_eq!(p.receipts.len(), 1);
    assert_eq!(p.receipts[0].participant_id, f.participant(2));
    // A hidden Bob event must not consume the single visible page slot.
    let Outcome::Accepted {
        message_id: new_message,
        ..
    } = f
        .store
        .execute(
            &f.keys[2],
            &Operation::Send {
                request_id: "later-message".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.participant(0)]),
                    "visible later message",
                ),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Inbox {
                cursor: checkpoint,
                limit: 1,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message.as_ref().unwrap().id, new_message);
    let suppressed = f
        .store
        .execute(
            &f.keys[1],
            &f.report(1, "suppressed", &m, ClientReceiptState::Read),
        )
        .unwrap();
    assert!(matches!(
        suppressed,
        Outcome::ReceiptAcknowledged(ReceiptAcknowledgement { recorded: None, .. })
    ));
    f.prefs(1, "delivery-only", true, false);
    assert_eq!(f.page(1, &m).receipts[0].state, DeliveryState::Delivered);
    assert!(f.events(0).is_empty());
}
#[test]
fn receipts_and_their_cursors_recheck_recipient_ceiling_and_current_epoch() {
    let f = Fixture::new();
    let m = f.send("all", MessageAudience::all());
    let narrow = f
        .store
        .issue(
            &f.ids[0],
            Scope::collaboration(vec![f.ids[1].clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let Outcome::MessageReceipts(p) = f
        .store
        .execute(
            &narrow.id,
            &Operation::MessageReceipts {
                conversation_id: f.cid.clone(),
                message_id: m.clone(),
                after: None,
                limit: 1,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(p.receipts.len(), 1);
    assert_eq!(p.receipts[0].participant_id, f.participant(1));
    let Outcome::MessageReceipts(next) = f
        .store
        .execute(
            &narrow.id,
            &Operation::MessageReceipts {
                conversation_id: f.cid.clone(),
                message_id: m.clone(),
                after: p.next,
                limit: 1,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(next.receipts.is_empty());
    f.prefs(2, "carol-read", true, true);
    f.store
        .execute(
            &f.keys[2],
            &f.report(2, "carol-report", &m, ClientReceiptState::Read),
        )
        .unwrap();
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            &narrow.id,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(events.iter().all(|e| e.receipt.is_none()));
    let bob = f.participant(1);
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "remove".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Remove {
                    participant_id: bob.clone(),
                    reason: None,
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute(
                &f.keys[1],
                &f.report(1, "stale", &m, ClientReceiptState::Delivered)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f
        .page(0, &m)
        .receipts
        .iter()
        .all(|r| r.participant_id != bob));
}
#[test]
fn provider_acceptance_delivery_and_local_uncertainty_stay_separate_from_native_read() {
    use connect_connector_api::lifecycle::*;
    let f = Fixture::new();
    f.route("vapi");
    let m = f.private();
    assert_eq!(f.page(0, &m).receipts.len(), 2);
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(f
        .page(0, &m)
        .receipts
        .iter()
        .any(|r| r.channel == "provider" && r.state == DeliveryState::Unknown));
    let observation = |id: &str, state| MessageReceipt {
        observation: Observation {
            mutation_id: job.id.clone(),
            external_id: Some("external-one".into()),
            event_id: id.into(),
            state: State::Message(state),
            verification: Verification::VerifiedCallback,
        },
        reply: None,
        reply_sources: vec![],
    };
    let accepted = observation("accepted", MessageState::Accepted);
    f.store.apply_connector_receipt("vapi", &accepted).unwrap();
    assert_eq!(
        f.page(0, &m)
            .receipts
            .iter()
            .find(|r| r.channel == "provider")
            .unwrap()
            .state,
        DeliveryState::Accepted
    );
    let delivered = observation("delivered", MessageState::Delivered);
    f.store.apply_connector_receipt("vapi", &delivered).unwrap();
    f.store.apply_connector_receipt("vapi", &delivered).unwrap();
    for (id, state) in [
        ("late-accepted", MessageState::Accepted),
        ("late-failed", MessageState::Failed),
        ("late-unknown", MessageState::Uncertain),
    ] {
        let result = f
            .store
            .apply_connector_receipt("vapi", &observation(id, state));
        if state == MessageState::Failed {
            assert_eq!(result.unwrap_err().code, "conflict");
        } else {
            assert_eq!(result.unwrap(), Apply::Older);
        }
    }
    let p = f.page(0, &m);
    assert_eq!(
        p.receipts
            .iter()
            .find(|r| r.channel == "provider")
            .unwrap()
            .state,
        DeliveryState::Delivered
    );
    assert_eq!(
        p.receipts
            .iter()
            .find(|r| r.channel == "native")
            .unwrap()
            .state,
        DeliveryState::Queued
    );
    assert!(p.receipts.iter().all(|r| r.state != DeliveryState::Read));
    let restored_path = f.dir.path().join("backup.sqlite3");
    f.store.backup(&restored_path).unwrap();
    let restored = Store::open(&restored_path, Limits::default()).unwrap();
    let Outcome::MessageReceipts(after) = restored
        .execute(
            &f.keys[0],
            &Operation::MessageReceipts {
                conversation_id: f.cid.clone(),
                message_id: m,
                after: None,
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(p, after);
}
#[test]
fn completed_provider_answer_is_not_read_and_suppressed_sms_stays_failed_without_dispatch() {
    let f = Fixture::new();
    f.route("vapi");
    let m = f.private();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(&job, "completed", Some("provider-answer"), Some("Answer"))
        .unwrap();
    assert_eq!(
        f.page(0, &m)
            .receipts
            .iter()
            .find(|r| r.channel == "provider")
            .unwrap()
            .state,
        DeliveryState::Accepted
    );
    let sms = Fixture::new();
    sms.route("telnyx");
    let message = sms.private();
    assert!(sms.store.claim_provider_job().unwrap().is_none());
    assert_eq!(
        sms.page(0, &message)
            .receipts
            .iter()
            .find(|r| r.channel == "provider")
            .unwrap()
            .state,
        DeliveryState::Failed
    );
    assert!(sms.store.claim_provider_job().unwrap().is_none());
}
#[test]
fn failed_receipt_journal_rolls_back_facts_and_same_id_can_retry_after_recovery() {
    let f = Fixture::new();
    let m = f.private();
    let c = rusqlite::Connection::open(f.dir.path().join("state.sqlite3")).unwrap();
    c.execute_batch("CREATE TRIGGER deny_receipt BEFORE INSERT ON journal WHEN NEW.kind='message_receipt' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    let op = f.report(1, "recoverable-report", &m, ClientReceiptState::Delivered);
    assert_eq!(
        f.store.execute(&f.keys[1], &op).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(f.page(0, &m).receipts[0].state, DeliveryState::Queued);
    assert!(f.events(0).is_empty());
    c.execute_batch("DROP TRIGGER deny_receipt;").unwrap();
    f.store.execute(&f.keys[1], &op).unwrap();
    assert_eq!(f.page(0, &m).receipts[0].state, DeliveryState::Delivered);
    assert_eq!(f.events(0).len(), 1);
}
#[test]
fn task_and_invocation_receipt_queries_use_task_recipient_ceiling_and_cannot_report_as_a_client() {
    use connect_protocol::tasks::*;
    let mut f = Fixture::new();
    f.keys[0] = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id;
    let m = f.send("all", MessageAudience::all());
    let TaskOutcome::Record(task) = f
        .store
        .execute_task(
            &f.keys[0],
            &TaskRequest::new(TaskCommand::Create {
                request_id: "receipt-task".into(),
                definition: TaskDefinition {
                    conversation_id: f.cid.clone(),
                    scope: TaskScope {
                        operations: vec![TaskPermission::History],
                        recipients: vec![f.ids[1].clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: chrono::Utc::now().timestamp() + 120,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!()
    };
    let binding = InvocationBinding {
        kind: InvocationKind::Native,
        adapter: "receipt-fixture".into(),
        external_id: "one".into(),
    };
    f.store
        .register_task_binding(&f.keys[0], &task.id, &binding)
        .unwrap();
    let grant = f
        .store
        .issue_task_grant(
            &f.keys[0],
            "receipt-grant",
            &GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::History],
                expires_at: chrono::Utc::now().timestamp() + 60,
            },
        )
        .unwrap();
    let handle = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &binding)
        .unwrap();
    let invoke = |id: &str, op| {
        TaskRequest::new(TaskCommand::Invoke {
            request_id: id.into(),
            task_id: task.id.clone(),
            step_id: id.into(),
            operation: Box::new(op),
            reply_to: None,
            wait_for_reply: false,
        })
    };
    let query = Operation::MessageReceipts {
        conversation_id: f.cid.clone(),
        message_id: m.clone(),
        after: None,
        limit: 100,
    };
    for result in [
        f.store
            .execute_task(&f.keys[0], &invoke("owner-query", query.clone()))
            .unwrap(),
        f.store
            .execute_task_invocation(&handle, &invoke("grant-query", query))
            .unwrap(),
    ] {
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = result else {
            panic!()
        };
        let Outcome::MessageReceipts(page) = *outcome else {
            panic!()
        };
        assert_eq!(page.receipts.len(), 1);
        assert_eq!(page.receipts[0].participant_id, f.participant(1));
    }
    let report = f.report(0, "proxy-read", &m, ClientReceiptState::Read);
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &invoke("proxy-read", report))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .revoke_task_grant(&f.keys[0], &grant.grant.id)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(
                &handle,
                &invoke(
                    "after-revoke",
                    Operation::ReceiptPreferencesGet {
                        conversation_id: f.cid.clone()
                    }
                )
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
