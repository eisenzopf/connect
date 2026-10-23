#![allow(clippy::unwrap_used)]
use connect_protocol::{Call, Limits, Operation, Outcome};
use connect_store::{Scope, Store, VoiceAction, VoiceRoute};
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    human: String,
    bot: String,
    key: String,
    operator: String,
    conversation: String,
    route: VoiceRoute,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let human = store.create_identity("human").unwrap().id;
    let bot = store.create_identity("bot").unwrap().id;
    let expires = chrono::Utc::now().timestamp() + 600;
    let operator = store.issue(&human, Scope::operator(), expires).unwrap().id;
    let key = store
        .issue(&human, Scope::communications(vec![bot.clone()]), expires)
        .unwrap()
        .id;
    let conversation = match store
        .execute(
            &key,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    bot.clone()
                ]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!(),
    };
    let route = VoiceRoute {
        id: "test-voice".into(),
        conversation_id: conversation.clone(),
        identity_id: bot.clone(),
        assistant_id: "12345678-1234-1234-1234-123456789012".into(),
        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
        destinations: vec!["+15555550101".into()],
        enabled: true,
    };
    store.configure_voice_route(&operator, &route).unwrap();
    Fixture {
        _dir: dir,
        store,
        human,
        bot,
        key,
        operator,
        conversation,
        route,
    }
}
fn start(f: &Fixture, id: &str) -> Operation {
    Operation::CallStart {
        request_id: id.into(),
        conversation_id: f.conversation.clone(),
        route_id: f.route.id.clone(),
        destination: f.route.destinations[0].clone(),
    }
}
fn call(o: Outcome) -> Call {
    if let Outcome::Call(c) = o {
        c
    } else {
        panic!()
    }
}
#[test]
fn replay_is_one_start_and_crash_is_never_redialed() {
    let f = fixture();
    let op = start(&f, "dial-1");
    let accepted = f.store.execute(&f.key, &op).unwrap();
    assert_eq!(accepted, f.store.execute(&f.key, &op).unwrap());
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(job.action, VoiceAction::Start);
    assert!(f.store.claim_voice_job().unwrap().is_none());
    f.store.recover_voice_calls().unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
    let c = call(accepted);
    let state = call(
        f.store
            .execute(
                &f.key,
                &Operation::CallStatus {
                    conversation_id: f.conversation.clone(),
                    call_id: c.id.clone(),
                },
            )
            .unwrap(),
    );
    assert_eq!(state.state, "uncertain");
    assert_eq!(
        f.store
            .execute(
                &f.key,
                &Operation::CallEnd {
                    request_id: "end".into(),
                    conversation_id: f.conversation,
                    call_id: c.id
                }
            )
            .unwrap_err()
            .code,
        "provider-uncertain"
    );
}
#[test]
fn voice_requires_grant_destination_membership_and_current_dispatch_authority() {
    let f = fixture();
    let messaging = f
        .store
        .issue(
            &f.human,
            Scope::messaging(vec![f.bot.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute(&messaging.id, &start(&f, "no-grant"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut disallowed = start(&f, "disallowed");
    if let Operation::CallStart { destination, .. } = &mut disallowed {
        *destination = "+15555550102".into();
    }
    assert_eq!(
        f.store.execute(&f.key, &disallowed).unwrap_err().code,
        "forbidden"
    );
    let op = start(&f, "revoked");
    f.store.execute(&f.key, &op).unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.operator).unwrap(), &f.key)
        .unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
    assert_eq!(
        f.store.execute(&f.key, &op).unwrap_err().code,
        "unauthorized"
    );
}
#[test]
fn pending_end_cancels_without_provider_effect_and_journal_snapshots_are_immutable() {
    let f = fixture();
    let c = call(f.store.execute(&f.key, &start(&f, "dial")).unwrap());
    let end = Operation::CallEnd {
        request_id: "end".into(),
        conversation_id: f.conversation.clone(),
        call_id: c.id.clone(),
    };
    assert_eq!(
        call(f.store.execute(&f.key, &end).unwrap()).state,
        "cancelled"
    );
    assert!(f.store.claim_voice_job().unwrap().is_none());
    let page = f
        .store
        .execute(
            &f.key,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap();
    if let Outcome::Inbox { events, .. } = page {
        let states: Vec<_> = events
            .iter()
            .filter_map(|e| e.call.as_ref().map(|c| c.state.as_str()))
            .collect();
        assert_eq!(states, vec!["pending", "cancelled"]);
    } else {
        panic!()
    }
    assert_eq!(
        call(f.store.execute(&f.key, &start(&f, "dial")).unwrap()).state,
        "pending"
    );
    let outsider = f.store.create_identity("outsider").unwrap().id;
    let key = f
        .store
        .issue(
            &outsider,
            Scope::communications(vec![f.human.clone(), f.bot.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute(
                &key.id,
                &Operation::CallStatus {
                    conversation_id: f.conversation,
                    call_id: c.id
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn call_metadata_is_covered_by_verified_backup() {
    let f = fixture();
    let c = call(f.store.execute(&f.key, &start(&f, "dial")).unwrap());
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .finish_voice_job(
            &job,
            "ringing",
            Some("12345678-1234-1234-1234-123456789015"),
            None,
            None,
        )
        .unwrap();
    let backup = f._dir.path().join("backup.sqlite3");
    let before = f.store.backup(&backup).unwrap();
    assert_eq!(before.schema, connect_store::SCHEMA_VERSION);
    let state = call(
        f.store
            .execute(
                &f.key,
                &Operation::CallStatus {
                    conversation_id: f.conversation.clone(),
                    call_id: c.id.clone(),
                },
            )
            .unwrap(),
    );
    assert_eq!(state.state, "ringing");
    assert_eq!(before, Store::inspect_snapshot(&backup).unwrap());
}

#[test]
fn inbound_is_deduplicated_scoped_recoverable_and_not_a_dial() {
    let f = fixture();
    f.store
        .configure_voice_ingress(&f.operator, &f.route.id, &f.route.phone_number_id)
        .unwrap();
    let mut incoming = connect_store::InboundCall {
        provider_id: uuid::Uuid::new_v4().to_string(),
        assistant_id: f.route.assistant_id.clone(),
        phone_number_id: f.route.phone_number_id.clone(),
        caller: f.route.destinations[0].clone(),
        created_at: chrono::Utc::now().timestamp(),
        state: "in-progress".into(),
        control_url: None,
        ended_reason: None,
    };
    assert!(f
        .store
        .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
        .unwrap());
    assert!(!f
        .store
        .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
        .unwrap());
    f.store.recover_voice_calls().unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(job.action, VoiceAction::Poll);
    assert_eq!(job.call.direction, "inbound");
    f.store
        .finish_voice_job(
            &job,
            "ended",
            Some(&incoming.provider_id),
            None,
            Some("customer-ended-call"),
        )
        .unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
    incoming.provider_id = uuid::Uuid::new_v4().to_string();
    incoming.caller = "+15555559999".into();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
            .unwrap_err()
            .code,
        "forbidden"
    );
    incoming.caller = f.route.destinations[0].clone();
    incoming.phone_number_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
            .unwrap_err()
            .code,
        "forbidden"
    );
    incoming.phone_number_id = f.route.phone_number_id.clone();
    incoming.created_at -= 3600;
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
            .unwrap_err()
            .code,
        "forbidden"
    );
    incoming.created_at += 3600;
    let mut disabled = f.route.clone();
    disabled.enabled = false;
    f.store
        .configure_voice_route(&f.operator, &disabled)
        .unwrap();
    assert!(f.store.voice_ingresses().unwrap().is_empty());
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .configure_voice_route(&f.operator, &f.route)
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.operator).unwrap(), &f.operator)
        .unwrap();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming_receipt(&incoming))
            .unwrap_err()
            .code,
        "unauthorized"
    );
}

fn ready_call(f: &Fixture) -> Call {
    let c = call(f.store.execute(&f.key, &start(f, "start-ready")).unwrap());
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .finish_voice_job(
            &job,
            "in-progress",
            Some("12345678-1234-1234-1234-123456789015"),
            None,
            None,
        )
        .unwrap();
    c
}
fn force_poll(f: &Fixture) {
    rusqlite::Connection::open(f.store.path())
        .unwrap()
        .execute("UPDATE voice_calls SET next_poll=0", [])
        .unwrap();
}
#[test]
fn end_intent_survives_a_racing_poll_and_uncertain_end_is_not_repeated() {
    let f = fixture();
    let c = ready_call(&f);
    force_poll(&f);
    let poll = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(poll.action, VoiceAction::Poll);
    let end = Operation::CallEnd {
        request_id: "end-race".into(),
        conversation_id: f.conversation.clone(),
        call_id: c.id.clone(),
    };
    assert!(call(f.store.execute(&f.key, &end).unwrap()).end_requested);
    f.store
        .finish_voice_job(&poll, "in-progress", None, None, None)
        .unwrap();
    let end_job = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(end_job.action, VoiceAction::End);
    f.store
        .finish_voice_job(&end_job, "uncertain", None, None, None)
        .unwrap();
    f.store.recover_voice_calls().unwrap();
    force_poll(&f);
    let reconcile = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(reconcile.action, VoiceAction::Poll);
    assert!(reconcile.call.end_requested);
    f.store
        .finish_voice_job(
            &reconcile,
            "ended",
            None,
            None,
            Some("assistant-ended-call"),
        )
        .unwrap();
    let current = call(
        f.store
            .execute(
                &f.key,
                &Operation::CallStatus {
                    conversation_id: f.conversation.clone(),
                    call_id: c.id,
                },
            )
            .unwrap(),
    );
    assert_eq!(current.state, "ended");
    assert!(!current.end_requested);
    assert!(call(f.store.execute(&f.key, &end).unwrap()).end_requested); // Immutable acceptance receipt.
    assert!(f.store.claim_voice_job().unwrap().is_none());
}
#[test]
fn end_dispatch_uses_the_current_authority_of_the_actual_requester() {
    let f = fixture();
    let c = ready_call(&f);
    let end_key = f
        .store
        .issue(
            &f.human,
            Scope::communications(vec![f.bot.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .execute(
            &end_key.id,
            &Operation::CallEnd {
                request_id: "revoked-end".into(),
                conversation_id: f.conversation.clone(),
                call_id: c.id.clone(),
            },
        )
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.operator).unwrap(), &end_key.id)
        .unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
    let current = call(
        f.store
            .execute(
                &f.key,
                &Operation::CallStatus {
                    conversation_id: f.conversation.clone(),
                    call_id: c.id,
                },
            )
            .unwrap(),
    );
    assert!(!current.end_requested);
    assert_eq!(current.state, "in-progress");
}
#[test]
fn inbound_scan_watermark_is_durable_and_rechecked_on_commit() {
    let f = fixture();
    f.store
        .configure_voice_ingress(&f.operator, &f.route.id, &f.route.phone_number_id)
        .unwrap();
    let mut b = f.store.voice_ingresses().unwrap().remove(0);
    assert_eq!(b.scanned_until, 0);
    b.scan_until = chrono::Utc::now().timestamp_millis();
    f.store.complete_voice_scan(&b).unwrap();
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        reopened.voice_ingresses().unwrap()[0].scanned_until,
        b.scan_until
    );
    let mut disabled = f.route.clone();
    disabled.enabled = false;
    f.store
        .configure_voice_route(&f.operator, &disabled)
        .unwrap();
    assert_eq!(
        f.store.complete_voice_scan(&b).unwrap_err().code,
        "invalid-request"
    );
    f.store
        .configure_voice_route(&f.operator, &f.route)
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.operator).unwrap(), &f.operator)
        .unwrap();
    assert!(f.store.voice_ingresses().unwrap().is_empty());
    assert_eq!(
        f.store.complete_voice_scan(&b).unwrap_err().code,
        "unauthorized"
    );
}

fn incoming_receipt(c: &connect_store::InboundCall) -> connect_connector_api::InboundVoiceReceipt {
    use connect_connector_api::lifecycle::{Verification, VoiceState};
    connect_connector_api::InboundVoiceReceipt {
        call: connect_store::InboundCall {
            provider_id: c.provider_id.clone(),
            assistant_id: c.assistant_id.clone(),
            phone_number_id: c.phone_number_id.clone(),
            caller: c.caller.clone(),
            created_at: c.created_at,
            state: c.state.clone(),
            control_url: c.control_url.clone(),
            ended_reason: c.ended_reason.clone(),
        },
        event_id: format!("discover:{}:{}", c.provider_id, c.state),
        state: match c.state.as_str() {
            "queued" | "ringing" => VoiceState::Starting,
            "in-progress" => VoiceState::Active,
            "ended" => VoiceState::Ended,
            _ => VoiceState::Uncertain,
        },
        verification: Verification::Reconciliation,
    }
}
fn inbound_fixture(f: &Fixture) -> connect_connector_api::InboundVoiceReceipt {
    f.store
        .configure_voice_ingress(&f.operator, &f.route.id, &f.route.phone_number_id)
        .unwrap();
    incoming_receipt(&connect_store::InboundCall {
        provider_id: uuid::Uuid::new_v4().to_string(),
        assistant_id: f.route.assistant_id.clone(),
        phone_number_id: f.route.phone_number_id.clone(),
        caller: f.route.destinations[0].clone(),
        created_at: chrono::Utc::now().timestamp(),
        state: "ended".into(),
        control_url: None,
        ended_reason: Some("customer-ended-call".into()),
    })
}
#[test]
fn inbound_import_atomically_binds_typed_receipt_and_rechecks_replays() {
    use connect_connector_api::lifecycle::{Projection, State, VoiceState};
    let f = fixture();
    let receipt = inbound_fixture(&f);
    assert!(f.store.import_voice_call(&f.route.id, &receipt).unwrap());
    assert!(!f.store.import_voice_call(&f.route.id, &receipt).unwrap());
    let db = rusqlite::Connection::open(f.store.path()).unwrap();
    let (id, json):(String,String)=db.query_row("SELECT c.id,o.projection_json FROM voice_calls c JOIN connector_voice_observations o ON o.call_id=c.id WHERE c.provider_id=?1",[&receipt.call.provider_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let p: Projection = serde_json::from_str(&json).unwrap();
    assert_eq!(p.mutation_id, id);
    assert_eq!(
        p.external_id.as_deref(),
        Some(receipt.call.provider_id.as_str())
    );
    assert_eq!(p.state, State::Voice(VoiceState::Ended));
    assert_eq!(
        serde_json::to_value(&p).unwrap()["events"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    let owner = f.store.revalidate(&f.operator).unwrap();
    f.store.revoke(&owner, &f.operator).unwrap();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &receipt)
            .unwrap_err()
            .code,
        "unauthorized"
    );
}
#[test]
fn inbound_import_refuses_unverified_mismatched_and_invalid_receipts_without_effects() {
    use connect_connector_api::lifecycle::{Verification, VoiceState};
    let f = fixture();
    for case in 0..5 {
        let mut r = inbound_fixture(&f);
        match case {
            0 => r.state = VoiceState::Active,
            1 => r.verification = Verification::LocalAttempt,
            2 => r.event_id.clear(),
            3 => r.event_id = "x".repeat(257),
            _ => r.event_id = "bad\nreference".into(),
        }
        assert_eq!(
            f.store.import_voice_call(&f.route.id, &r).unwrap_err().code,
            "invalid-request"
        );
    }
    let db = rusqlite::Connection::open(f.store.path()).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM connector_voice_observations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn inbound_projection_storage_failure_rolls_back_call_and_journal() {
    let f = fixture();
    let receipt = inbound_fixture(&f);
    let db = rusqlite::Connection::open(f.store.path()).unwrap();
    let before: u64 = db
        .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER refuse_voice_projection BEFORE INSERT ON connector_voice_observations BEGIN SELECT RAISE(ABORT, 'designated fixture storage failure'); END;").unwrap();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &receipt)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM journal", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        before
    );
    db.execute_batch("DROP TRIGGER refuse_voice_projection")
        .unwrap();
    assert!(f.store.import_voice_call(&f.route.id, &receipt).unwrap());
}
