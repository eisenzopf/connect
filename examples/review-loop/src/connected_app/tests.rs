//! Private-state admission and bounded exact-intent invariants.
#![allow(clippy::unwrap_used)]
use super::{domain::digest, ledger, AppStore, Draft, Policy};
use connect_client::{ClientConfig, Event, InboxPage, Message, Operation};
use std::path::PathBuf;

fn setup() -> (tempfile::TempDir, PathBuf, ClientConfig, Policy) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("app.sqlite3");
    let trust = temp.path().join("cert.der");
    std::fs::write(&trust, b"state-binding-test-cert").unwrap();
    let config = ClientConfig {
        endpoint: "127.0.0.1:4000".parse().unwrap(),
        server_name: "localhost".into(),
        trust_certificate: trust,
        credential: "private-test-credential".into(),
        identity_id: "app".into(),
        credential_id: "credential".into(),
        timeout_ms: 1000,
    };
    let policy = Policy {
        reviewer_address: "connect:peer@local".into(),
        reviewer_id: "peer".into(),
        allowed_contacts: vec!["peer".into()],
        draft: Draft {
            id: "draft".into(),
            title: "Title".into(),
            body: "Body".into(),
        },
    };
    (temp, path, config, policy)
}
#[test]
fn changed_policy_credential_and_trust_cannot_adopt_existing_state() {
    let (_temp, path, config, policy) = setup();
    drop(AppStore::open(&path, &config, &policy).unwrap());
    let before = std::fs::read(&path).unwrap();
    let mut changed = policy.clone();
    changed.allowed_contacts.push("another".into());
    assert!(AppStore::open(&path, &config, &changed).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut credential = config.clone();
    credential.credential = "replacement".into();
    assert!(AppStore::open(&path, &credential, &policy).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&config.trust_certificate, b"replacement-trust").unwrap();
    assert!(AppStore::open(&path, &config, &policy).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
#[test]
fn exclusive_app_writer_and_unrelated_database_are_preserved() {
    let (_temp, path, config, policy) = setup();
    let store = AppStore::open(&path, &config, &policy).unwrap();
    assert!(AppStore::open(&path, &config, &policy).is_err());
    drop(store);
    let foreign = path.with_extension("foreign.sqlite3");
    std::fs::write(&foreign, []).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let c = rusqlite::Connection::open(&foreign).unwrap();
    c.execute_batch("CREATE TABLE original(value TEXT); INSERT INTO original VALUES('keep');")
        .unwrap();
    drop(c);
    let bytes = std::fs::read(&foreign).unwrap();
    assert!(AppStore::open(&foreign, &config, &policy).is_err());
    assert_eq!(bytes, std::fs::read(&foreign).unwrap());
}
#[cfg(unix)]
#[test]
fn symlink_and_public_app_state_are_rejected_without_writes() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (_temp, path, config, policy) = setup();
    drop(AppStore::open(&path, &config, &policy).unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let alias = path.with_extension("alias");
    symlink(&path, &alias).unwrap();
    assert!(AppStore::open(&alias, &config, &policy).is_err());
    assert!(AppStore::inspect(&alias).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(AppStore::open(&path, &config, &policy).is_err());
    assert!(AppStore::inspect(&path).is_err());
    assert_eq!(bytes, std::fs::read(&path).unwrap());
}
#[test]
fn full_command_ledger_preserves_existing_exact_intent() {
    let (_temp, path, config, policy) = setup();
    let store = AppStore::open(&path, &config, &policy).unwrap();
    let original = Operation::Create {
        request_id: "original".into(),
        request: connect_client::conversations::CreateRequest::persistent(vec!["peer".into()]),
    };
    ledger::insert(&store.connection, "same-key", &original).unwrap();
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<9999) INSERT INTO app_commands(key,operation) SELECT 'filler-'||i,'{}' FROM n;").unwrap();
    ledger::insert(
        &store.connection,
        "same-key",
        &Operation::Create {
            request_id: "must-not-replace".into(),
            request: connect_client::conversations::CreateRequest::persistent(vec!["other".into()]),
        },
    )
    .unwrap();
    assert!(ledger::insert(&store.connection, "new-key", &original).is_err());
    let json: String = store
        .connection
        .query_row(
            "SELECT operation FROM app_commands WHERE key='same-key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Operation>(&json).unwrap(), original);
}
#[test]
fn changed_journal_identity_rolls_back_page_without_advancing_cursor() {
    let (_temp, path, config, policy) = setup();
    let mut store = AppStore::open(&path, &config, &policy).unwrap();
    let mut event = Event {
        id: "event".into(),
        kind: "message".into(),
        conversation_id: "cid".into(),
        cursor: "position-1".into(),
        message: Some(Message {
            id: "message".into(),
            conversation_id: "cid".into(),
            sender: "peer".into(),
            body: "Original".into(),
            sender_participant_id: Some("part_peer".into()),
            audience: vec![connect_client::messages::MessageRecipient {
                participant_id: "part_owner".into(),
                membership_epoch: 1,
            }],
            content_type: "text/plain".into(),
            body_encoding: connect_client::messages::BodyEncoding::Utf8,
            label: "connect-messages".into(),
            attachments: vec![],
            in_reply_to_msg: None,
            accepted_at: Some(1),
        }),
        call: None,
        session: None,
        artifact: None,
        receipt: None,
    };
    store
        .stage(InboxPage {
            events: vec![event.clone()],
            cursor: Some("position-1".into()),
        })
        .unwrap();
    store.finish_event("event", "ignored").unwrap();
    store.finish_page().unwrap();
    event.message.as_mut().unwrap().body = "Changed".into();
    assert!(store
        .stage(InboxPage {
            events: vec![event],
            cursor: Some("position-2".into())
        })
        .is_err());
    assert_eq!(store.cursor().unwrap().as_deref(), Some("position-1"));
    assert!(store.metadata("page").unwrap().is_none());
}
#[test]
fn business_wire_contract_rejects_unknown_fields_and_bounds_content() {
    let (_temp, _path, _config, mut policy) = setup();
    assert!(policy.validate().is_ok());
    let original = policy.draft.digest().unwrap();
    policy.draft.body.push('x');
    assert_ne!(policy.draft.digest().unwrap(), original);
    policy.draft.body = "x".repeat(4097);
    assert!(policy.validate().is_err());
    assert!(serde_json::from_str::<super::Request>(r#"{"action":"submit_draft","request_id":"one","draft":{"id":"d","title":"t","body":"b"},"sender":"forged"}"#).is_err());
    assert_eq!(digest(&"same").unwrap(), digest(&"same").unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_app_future_can_be_supervised_on_a_tokio_worker() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.toml");
    let result = tokio::spawn(super::run(
        super::RunOptions {
            config: missing.clone(),
            policy: missing,
            state: temp.path().join("not-created.sqlite3"),
        },
        tokio_util::sync::CancellationToken::new(),
    ))
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!temp.path().join("not-created.sqlite3").exists());
}
