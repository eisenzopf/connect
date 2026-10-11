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
                participants: vec![bot.clone()],
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
fn schema_three_upgrade_requires_backup_and_preserves_conversations() {
    let f = fixture();
    let path = f.store.path().to_owned();
    let key = f.key.clone();
    let conv = f.conversation.clone();
    drop(f.store);
    let c = rusqlite::Connection::open(&path).unwrap();
    remove_gate_four_fixture_tables(&c);
    c.execute_batch("DROP TABLE vapi_session_setups; DROP TABLE sms_authorities; DROP TABLE sms_grants; DROP TABLE sms_controls; DROP TABLE sms_control_events; ALTER TABLE provider_jobs DROP COLUMN sms_decision; ALTER TABLE provider_jobs DROP COLUMN sms_grant_id; DROP TABLE sip_legs; DROP TABLE context_deliveries; DROP TABLE session_bindings; DROP TABLE sessions; DROP TABLE availability; DROP TABLE artifacts; ALTER TABLE journal DROP COLUMN artifact_json; ALTER TABLE journal DROP COLUMN session_json; DROP TABLE voice_ingress; DROP TABLE voice_calls; DROP TABLE voice_routes; ALTER TABLE journal DROP COLUMN call_json; PRAGMA user_version=3;").unwrap();
    drop(c);
    assert_eq!(
        Store::open(&path, Limits::default()).err().unwrap().code,
        "unsupported-schema"
    );
    let backup = f._dir.path().join("schema3-backup.sqlite3");
    let old = Store::backup_database(&path, &backup).unwrap();
    assert_eq!(old.schema, 3);
    assert_eq!(
        Store::migrate(&path, &backup, &old).unwrap().schema,
        connect_store::SCHEMA_VERSION
    );
    let upgraded = Store::open(&path, Limits::default()).unwrap();
    let page = upgraded
        .execute(
            &key,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap();
    if let Outcome::Inbox { events, .. } = page {
        assert!(events.iter().any(|e| e.conversation_id == conv));
    } else {
        panic!()
    }
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), old);
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
    assert!(f.store.import_voice_call(&f.route.id, &incoming).unwrap());
    assert!(!f.store.import_voice_call(&f.route.id, &incoming).unwrap());
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
            .import_voice_call(&f.route.id, &incoming)
            .unwrap_err()
            .code,
        "forbidden"
    );
    incoming.caller = f.route.destinations[0].clone();
    incoming.phone_number_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming)
            .unwrap_err()
            .code,
        "forbidden"
    );
    incoming.phone_number_id = f.route.phone_number_id.clone();
    incoming.created_at -= 3600;
    assert_eq!(
        f.store
            .import_voice_call(&f.route.id, &incoming)
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
            .import_voice_call(&f.route.id, &incoming)
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
            .import_voice_call(&f.route.id, &incoming)
            .unwrap_err()
            .code,
        "unauthorized"
    );
}

#[test]
fn schema_four_upgrade_preserves_voice_and_requires_snapshot() {
    let f = fixture();
    let accepted = call(
        f.store
            .execute(&f.key, &start(&f, "pre-migration"))
            .unwrap(),
    );
    let path = f.store.path().to_owned();
    drop(f.store);
    let c = rusqlite::Connection::open(&path).unwrap();
    remove_gate_four_fixture_tables(&c);
    c.execute_batch("DROP TABLE vapi_session_setups; DROP TABLE sms_authorities; DROP TABLE sms_grants; DROP TABLE sms_controls; DROP TABLE sms_control_events; ALTER TABLE provider_jobs DROP COLUMN sms_decision; ALTER TABLE provider_jobs DROP COLUMN sms_grant_id; DROP TABLE sip_legs; DROP TABLE context_deliveries; DROP TABLE session_bindings; DROP TABLE sessions; DROP TABLE availability; DROP TABLE artifacts; ALTER TABLE journal DROP COLUMN artifact_json; ALTER TABLE journal DROP COLUMN session_json; DROP TABLE voice_ingress; DROP INDEX voice_provider_id; ALTER TABLE voice_calls DROP COLUMN direction; ALTER TABLE voice_calls DROP COLUMN end_credential_id; PRAGMA user_version=4;").unwrap();
    drop(c);
    let backup = f._dir.path().join("schema4.sqlite3");
    let snapshot = Store::backup_database(&path, &backup).unwrap();
    assert_eq!(snapshot.schema, 4);
    assert_eq!(
        Store::migrate(&path, &backup, &snapshot).unwrap().schema,
        connect_store::SCHEMA_VERSION
    );
    let store = Store::open(&path, Limits::default()).unwrap();
    let current = call(
        store
            .execute(
                &f.key,
                &Operation::CallStatus {
                    conversation_id: f.conversation,
                    call_id: accepted.id,
                },
            )
            .unwrap(),
    );
    assert_eq!(current.direction, "outbound");
    assert_eq!(current.state, "pending");
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
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
#[test]
fn schema_five_upgrade_retains_inbound_bindings_and_reconciles_legacy_end_intent() {
    let f = fixture();
    let c = ready_call(&f);
    f.store
        .configure_voice_ingress(&f.operator, &f.route.id, &f.route.phone_number_id)
        .unwrap();
    f.store
        .execute(
            &f.key,
            &Operation::CallEnd {
                request_id: "old-end".into(),
                conversation_id: f.conversation.clone(),
                call_id: c.id.clone(),
            },
        )
        .unwrap();
    let path = f.store.path().to_owned();
    drop(f.store);
    let db = rusqlite::Connection::open(&path).unwrap();
    remove_gate_four_fixture_tables(&db);
    db.execute_batch("DROP TABLE vapi_session_setups; DROP TABLE sms_authorities; DROP TABLE sms_grants; DROP TABLE sms_controls; DROP TABLE sms_control_events; ALTER TABLE provider_jobs DROP COLUMN sms_decision; ALTER TABLE provider_jobs DROP COLUMN sms_grant_id; DROP TABLE sip_legs; DROP TABLE context_deliveries; DROP TABLE session_bindings; DROP TABLE sessions; DROP TABLE availability; DROP TABLE artifacts; ALTER TABLE journal DROP COLUMN artifact_json; ALTER TABLE journal DROP COLUMN session_json; ALTER TABLE voice_calls DROP COLUMN end_credential_id; ALTER TABLE voice_ingress DROP COLUMN scanned_until; PRAGMA user_version=5;").unwrap();
    drop(db);
    let backup = f._dir.path().join("schema5.sqlite3");
    let snapshot = Store::backup_database(&path, &backup).unwrap();
    assert_eq!(snapshot.schema, 5);
    assert_eq!(
        Store::migrate(&path, &backup, &snapshot).unwrap().schema,
        connect_store::SCHEMA_VERSION
    );
    let store = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(store.voice_ingresses().unwrap().len(), 1);
    let job = store.claim_voice_job().unwrap().unwrap();
    assert_eq!(job.action, VoiceAction::Poll);
    assert!(job.call.end_requested);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
}

// These legacy migration fixtures deliberately reconstruct old schemas in
// temporary databases. Remove Gate 4 tables too; they never existed at v3–5.
fn remove_gate_four_fixture_tables(connection: &rusqlite::Connection) {
    connection.execute_batch("DROP INDEX session_bindings_task_grant; ALTER TABLE session_bindings DROP COLUMN task_grant_id; DROP TABLE media_end_intents; DROP TABLE media_start_admissions; DROP INDEX task_effect_media_resource; DROP TABLE task_effect_links; DROP TABLE task_effect_authority; DROP TABLE task_grants; DROP TABLE task_bindings; DROP TABLE task_claims; DROP TABLE task_effects; DROP TABLE task_replies; DROP TABLE task_requests; DROP TABLE task_events; DROP TABLE tasks;").unwrap();
}
