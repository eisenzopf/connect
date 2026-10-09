#![allow(clippy::unwrap_used)]
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Route, Scope, Store};
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    human: String,
    bot: String,
    human_key: String,
    bot_key: String,
    operator: String,
    conversation: String,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let human = store.create_identity("human").unwrap().id;
    let bot = store.create_identity("bot").unwrap().id;
    let expires = chrono::Utc::now().timestamp() + 600;
    let operator = store.issue(&human, Scope::operator(), expires).unwrap().id;
    let human_key = store
        .issue(&human, Scope::messaging(vec![bot.clone()]), expires)
        .unwrap()
        .id;
    let bot_key = store
        .issue(&bot, Scope::messaging(vec![human.clone()]), expires)
        .unwrap()
        .id;
    let conversation = match store
        .execute(
            &human_key,
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
    Fixture {
        _dir: dir,
        store,
        human,
        bot,
        human_key,
        bot_key,
        operator,
        conversation,
    }
}
impl Fixture {
    fn route(&self, sms: bool) -> Route {
        Route {
            id: if sms { "sms" } else { "vapi" }.into(),
            identity_id: if sms {
                self.human.clone()
            } else {
                self.bot.clone()
            },
            credential_id: if sms {
                self.human_key.clone()
            } else {
                self.bot_key.clone()
            },
            conversation_id: self.conversation.clone(),
            connector: if sms { "telnyx" } else { "vapi" }.into(),
            target: if sms {
                "+15555550101"
            } else {
                "12345678-1234-1234-1234-123456789012"
            }
            .into(),
            source: if sms {
                Some("+15555550102".into())
            } else {
                None
            },
            enabled: true,
        }
    }
    fn bind(&self, sms: bool) {
        self.store
            .configure_route(&self.operator, &self.route(sms))
            .unwrap();
    }
    fn send(&self, id: &str, body: &str) -> Outcome {
        self.store
            .execute(
                &self.human_key,
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: self.conversation.clone(),
                    body: body.into(),
                },
            )
            .unwrap()
    }
    fn status(&self) -> Vec<connect_protocol::ProviderDelivery> {
        match self
            .store
            .execute(
                &self.human_key,
                &Operation::ProviderStatus {
                    conversation_id: self.conversation.clone(),
                },
            )
            .unwrap()
        {
            Outcome::ProviderStatus { deliveries } => deliveries,
            _ => panic!(),
        }
    }
}

#[test]
fn routed_voice_accepts_enabled_vapi_connector_in_the_same_conversation() {
    let f = fixture();
    let caller = f
        .store
        .issue(
            &f.human,
            Scope::communications(vec![f.bot.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .admit_routed_inbound_session(&caller.id, &f.conversation, &f.bot)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.bind(false);
    let admitted = f
        .store
        .admit_routed_inbound_session(&caller.id, &f.conversation, &f.bot)
        .unwrap();
    assert_eq!(admitted.session.state, "accepted");
    assert_eq!(admitted.session.conversation_id, f.conversation);
    let resolved = f
        .store
        .vapi_session_route(&caller.id, &admitted.session.id)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.assistant_id, f.route(false).target);
    assert_eq!(
        f.store
            .admit_routed_inbound_session(&f.human_key, &f.conversation, &f.bot)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f
        .store
        .admit_routed_inbound_session(&caller.id, "other-conversation", &f.bot)
        .is_err());
    let mut route = f.route(false);
    route.enabled = false;
    f.store.configure_route(&f.operator, &route).unwrap();
    assert_eq!(
        f.store
            .admit_routed_inbound_session(&caller.id, &f.conversation, &f.bot)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn routed_voice_rejects_ambiguous_assistants_before_creating_a_session() {
    let f = fixture();
    let caller = f
        .store
        .issue(
            &f.human,
            Scope::communications(vec![f.bot.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.bind(false);
    let second = connect_store::VoiceRoute {
        id: "vapi-second".into(),
        conversation_id: f.conversation.clone(),
        identity_id: f.bot.clone(),
        assistant_id: "12345678-1234-1234-1234-123456789013".into(),
        phone_number_id: "12345678-1234-1234-1234-123456789014".into(),
        destinations: vec!["+15555550101".into()],
        enabled: true,
    };
    f.store.configure_voice_route(&f.operator, &second).unwrap();
    assert_eq!(
        f.store
            .admit_routed_inbound_session(&caller.id, &f.conversation, &f.bot)
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn replay_recovery_and_chat_context_do_not_duplicate_provider_work() {
    let f = fixture();
    f.bind(false);
    f.bind(true);
    let first = f.send("one", "hello");
    assert_eq!(first, f.send("one", "hello"));
    assert_eq!(f.status().len(), 1);
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(job.previous_chat_id, None);
    f.store
        .finish_provider_job(&job, "completed", Some("chat-1"), Some("hello human"))
        .unwrap();
    assert_eq!(f.status().len(), 2);
    let sms = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(sms.route.connector, "telnyx");
    f.store
        .finish_provider_job(&sms, "accepted", Some("sms-1"), None)
        .unwrap();
    f.send("two", "continue");
    let next = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(next.previous_chat_id.as_deref(), Some("chat-1"));
    assert_eq!(f.store.recover_provider_jobs().unwrap(), 1);
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert!(f.status().iter().any(|j| j.state == "uncertain"));
    f.send("three", "later");
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert!(f.status().iter().any(|j| j.state == "blocked"));
}
#[test]
fn signed_event_admission_is_atomic_deduplicated_and_scoped() {
    let f = fixture();
    f.bind(false);
    f.bind(true);
    assert!(f
        .store
        .receive_sms(
            "incoming",
            "digest",
            "+15555550101",
            "+15555550102",
            "question"
        )
        .unwrap());
    assert!(!f
        .store
        .receive_sms(
            "incoming",
            "digest",
            "+15555550101",
            "+15555550102",
            "question"
        )
        .unwrap());
    assert_eq!(
        f.store
            .receive_sms(
                "incoming",
                "different",
                "+15555550101",
                "+15555550102",
                "bad"
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .receive_sms("other", "digest", "+15555550103", "+15555550102", "bad")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.status().len(), 1);
    let vapi = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(&vapi, "completed", Some("chat"), Some("answer"))
        .unwrap();
    let sms = f.store.claim_provider_job().unwrap().unwrap();
    assert!(f
        .store
        .delivery_sms("early", "early-digest", "sms-early", "delivered")
        .unwrap());
    f.store
        .finish_provider_job(&sms, "accepted", Some("sms-early"), None)
        .unwrap();
    assert!(f.status().iter().any(|j| j.state == "delivered"));
    f.store
        .delivery_sms("late-failure", "failed-digest", "sms-early", "failed")
        .unwrap();
    assert!(f.status().iter().any(|j| j.state == "delivered")); // terminal state never regresses
}
#[test]
fn revoked_authority_cancels_pending_work_and_routes_cannot_retarget() {
    let f = fixture();
    f.bind(false);
    f.send("one", "hello");
    let mut changed = f.route(false);
    changed.target = "22345678-1234-1234-1234-123456789012".into();
    assert_eq!(
        f.store
            .configure_route(&f.operator, &changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let actor = f.store.revalidate(&f.operator).unwrap();
    f.store.revoke(&actor, &f.human_key).unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let Outcome::ProviderStatus { deliveries } = f
        .store
        .execute(
            &f.operator,
            &Operation::ProviderStatus {
                conversation_id: f.conversation.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(deliveries[0].state, "cancelled");
}
#[test]
fn provider_state_is_included_in_backup_and_v2_migration_preserves_messages() {
    let f = fixture();
    f.bind(false);
    f.send("one", "hello");
    let backup = f._dir.path().join("backup.sqlite3");
    let before = f.store.backup(&backup).unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(&job, "uncertain", None, None)
        .unwrap();
    assert_ne!(before, Store::inspect_snapshot(f.store.path()).unwrap());
    let copy = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(copy.recover_provider_jobs().unwrap(), 0);
    assert!(copy.claim_provider_job().unwrap().is_some());
    let legacy = f._dir.path().join("v2.sqlite3");
    let c = rusqlite::Connection::open(&legacy).unwrap();
    c.execute_batch(include_str!("../migrations/001_identity.sql"))
        .unwrap();
    c.execute_batch(include_str!("../migrations/002_conversations.sql"))
        .unwrap();
    c.execute("INSERT INTO installation VALUES(1,'legacy','key')", [])
        .unwrap();
    c.execute_batch(
        "INSERT INTO identities VALUES('legacy-user','legacy-user','[\"messaging\"]');
         INSERT INTO conversations VALUES('legacy-conversation',0);
         INSERT INTO participants VALUES('legacy-conversation','legacy-user');
         INSERT INTO messages(id,conversation_id,sender,body)
           VALUES('legacy-message','legacy-conversation','legacy-user','preserve this message');
         INSERT INTO journal(id,recipient,conversation_id,kind,message_id)
           VALUES('legacy-event','legacy-user','legacy-conversation','message.accepted','legacy-message');",
    )
    .unwrap();
    drop(c);
    let b = f._dir.path().join("v2-backup.sqlite3");
    let expected = Store::backup_database(&legacy, &b).unwrap();
    assert_eq!(expected.schema, 2);
    assert_eq!(
        Store::migrate(&legacy, &b, &expected).unwrap().schema,
        connect_store::SCHEMA_VERSION
    );
    assert_eq!(Store::inspect_snapshot(&b).unwrap(), expected);
    let upgraded = Store::open(&legacy, Limits::default()).unwrap();
    assert_eq!(
        upgraded.identity("legacy-user").unwrap().alias,
        "legacy-user"
    );
    let credential = upgraded
        .issue(
            "legacy-user",
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let history = upgraded
        .execute(
            &credential.id,
            &Operation::History {
                conversation_id: "legacy-conversation".into(),
                after: None,
                limit: 100,
            },
        )
        .unwrap();
    match history {
        Outcome::History { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].id, "legacy-message");
            assert_eq!(messages[0].body, "preserve this message");
        }
        _ => panic!("expected migrated history"),
    }
    match upgraded
        .execute(
            &credential.id,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    {
        Outcome::Inbox { events, .. } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].id, "legacy-event");
            assert_eq!(events[0].message.as_ref().unwrap().id, "legacy-message");
        }
        _ => panic!("expected migrated inbox"),
    }
}
