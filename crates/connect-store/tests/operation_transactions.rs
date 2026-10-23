#![allow(clippy::unwrap_used)]
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};

#[test]
fn infrastructure_failure_rolls_back_a_handler_write_and_does_not_cache_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Store::initialize(&path, Limits::default()).unwrap();
    let owner = store.create_identity("owner").unwrap().id;
    let key = store
        .issue(
            &owner,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let cid = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("conversation"),
    };
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("CREATE TRIGGER fail_artifact_event BEFORE INSERT ON journal WHEN NEW.kind='artifact_started' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let before = Store::inspect_snapshot(&path).unwrap();
    let request = Operation::ArtifactBegin {
        to: connect_protocol::messages::MessageAudience::all(),
        request_id: "begin".into(),
        conversation_id: cid,
        name: "fixture.txt".into(),
        media_type: "text/plain".into(),
        size: 1,
        sha256: "0".repeat(64),
        expires_at: None,
    };
    assert_eq!(
        store.execute(&key.id, &request).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
    c.execute_batch("DROP TRIGGER fail_artifact_event;")
        .unwrap();
    assert!(matches!(
        store.execute(&key.id, &request).unwrap(),
        Outcome::Artifact(_)
    ));
    let receipts: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='begin'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 1);
}
