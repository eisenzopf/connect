//! Another enrollment service uses the same durable STOP, leases and replay rules.
#![allow(clippy::unwrap_used)]
use connect_protocol::{Operation, Outcome};
use connect_server::{synchronize_sms_policy, SmsPolicy};
use connect_store::{Route, Scope, SmsGrant, SmsGrantSnapshot, Store};
use std::sync::Arc;
struct Alternative {
    snapshot: SmsGrantSnapshot,
}
#[async_trait::async_trait]
impl SmsPolicy for Alternative {
    fn source(&self) -> &str {
        "alternative-fixture/1"
    }
    async fn snapshot(&self) -> anyhow::Result<SmsGrantSnapshot> {
        Ok(self.snapshot.clone())
    }
}
#[tokio::test]
async fn alternative_authority_keeps_stop_start_and_replay_at_one_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(&temp.path().join("state.sqlite3"), Default::default()).unwrap(),
    );
    let a = store.create_identity("a").unwrap();
    let b = store.create_identity("b").unwrap();
    let now = chrono::Utc::now().timestamp();
    let operator = store.issue(&a.id, Scope::operator(), now + 3600).unwrap();
    let recipient = store
        .issue(&b.id, Scope::messaging(vec![a.id.clone()]), now + 3600)
        .unwrap();
    let Outcome::Conversation(c) = store
        .execute(
            &operator.id,
            &Operation::Create {
                request_id: "conversation".into(),
                participants: vec![b.id.clone()],
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    store
        .configure_route(
            &operator.id,
            &Route {
                id: "fixture-sms".into(),
                identity_id: b.id.clone(),
                credential_id: recipient.id,
                conversation_id: c.id.clone(),
                connector: "telnyx".into(),
                target: "+15555550101".into(),
                source: Some("+15555550102".into()),
                enabled: true,
            },
        )
        .unwrap();
    let send = |id: &str| Operation::Send {
        request_id: id.into(),
        conversation_id: c.id.clone(),
        body: "fixture draft".into(),
    };
    store.execute(&operator.id, &send("unapproved")).unwrap();
    assert!(store.claim_provider_job().unwrap().is_none());
    let policy = Alternative {
        snapshot: SmsGrantSnapshot {
            revision: 1,
            grants: vec![SmsGrant {
                id: "grant".into(),
                phone: "+15555550101".into(),
                sender: "+15555550102".into(),
                conversation_id: c.id.clone(),
                purpose: "Review a requested draft".into(),
                consent_id: "explicit-fixture-form".into(),
                disclosure_version: "fixture-v1".into(),
                disclosure_text: "Fixture reviewed consent".into(),
                submitted_at: now - 10,
                approved_at: now - 5,
                expires_at: now + 3600,
            }],
        },
    };
    synchronize_sms_policy(&policy, store.clone())
        .await
        .unwrap();
    store.execute(&operator.id, &send("queued")).unwrap();
    store
        .receive_sms_control(
            "stop",
            "stop-digest",
            "+15555550101",
            "+15555550102",
            "STOP",
            now,
        )
        .unwrap();
    assert!(store.claim_provider_job().unwrap().is_none());
    synchronize_sms_policy(&policy, store.clone())
        .await
        .unwrap();
    store.execute(&operator.id, &send("while-stopped")).unwrap();
    assert!(store.claim_provider_job().unwrap().is_none());
    store
        .receive_sms_control(
            "start",
            "start-digest",
            "+15555550101",
            "+15555550102",
            "START",
            now + 1,
        )
        .unwrap();
    store.execute(&operator.id, &send("queued")).unwrap();
    assert!(
        store.claim_provider_job().unwrap().is_none(),
        "START never revives cancelled/replayed work"
    );
    store
        .execute(&operator.id, &send("new-authorized"))
        .unwrap();
    assert!(store.claim_provider_job().unwrap().is_some());
    store.expire_sms_authorities().unwrap();
    store.execute(&operator.id, &send("stale")).unwrap();
    assert!(store.claim_provider_job().unwrap().is_none());
}
