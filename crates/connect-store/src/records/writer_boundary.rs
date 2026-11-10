use super::*;
use crate::media_authorization::{Prepared, SnapshotFacts};
use connect_protocol::Limits;
use std::{sync::mpsc, time::Instant};

struct CapturedRecordCount(u64);
impl SnapshotFacts for CapturedRecordCount {
    fn current(&self) -> bool {
        true
    }
}

#[test]
fn inserting_a_record_revokes_the_original_conversation_epoch_and_drains_its_io() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap(),
    );
    let a = store.create_identity("record-a").unwrap().id;
    let b = store.create_identity("record-b").unwrap().id;
    let scope = RecordScope {
        tenant_id: "tnt_default".into(),
        conversation_id: "conv_record".into(),
        session_id: "sess_record".into(),
    };
    // Faithful fixture SQL has the same actual ordered writers as production.
    let writer = store
        .acquire_authority(
            &[
                AuthorityScope::identity(&a, AuthorityMode::Write).unwrap(),
                AuthorityScope::identity(&b, AuthorityMode::Write).unwrap(),
                AuthorityScope::conversation(&scope.conversation_id, AuthorityMode::Write).unwrap(),
            ],
            &AuthorityWait::new(Duration::from_secs(2)),
        )
        .unwrap();
    {
        let connection = store.connection_with_authority(&writer).unwrap();
        connection
            .execute(
                "INSERT INTO conversations(id) VALUES(?1)",
                [&scope.conversation_id],
            )
            .unwrap();
        for (identity, participant) in [(&a, "part_a"), (&b, "part_b")] {
            connection.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES(?1,?2,?3,'participant')", params![scope.conversation_id, identity, participant]).unwrap();
        }
        connection.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?4,'accepted','[\"audio\"]',100,100)", params![scope.session_id, scope.conversation_id, a, b]).unwrap();
    }
    drop(writer);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = Arc::downgrade(&store);
    let conversation = scope.conversation_id.clone();
    let until = Instant::now() + Duration::from_secs(10);
    let handle = store
        .media_authorization_registry()
        .register(move || {
            let store = weak.upgrade().ok_or_else(Error::forbidden)?;
            let scopes = {
                let c = store.connection()?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &conversation,
                    AuthorityMode::Read,
                )?
            };
            let read = store.try_acquire_authority(&scopes)?;
            let c = store.connection()?;
            let count = c
                .query_row(
                    "SELECT count(*) FROM session_records WHERE conversation_id=?1",
                    [&conversation],
                    |row| row.get(0),
                )
                .map_err(|_| Error::storage())?;
            Prepared::new(Arc::new(CapturedRecordCount(count)), read, until)
        })
        .unwrap();
    let mut first = None;
    let limit = Instant::now() + Duration::from_secs(1);
    while first.is_none() && Instant::now() < limit {
        first = handle.admit().ok();
        if first.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let first = first.expect("initial actual snapshot publication");
    assert_eq!(first.facts().0, 0);
    let stamp = first.stamp();
    let uuid = uuid::Uuid::new_v4().to_string();
    let bytes = RecordBytes {
        uuid: uuid.clone(),
        body: serde_json::to_vec(&serde_json::json!({"uuid":uuid})).unwrap(),
        manifest: b"{}".to_vec(),
    };
    let (began_tx, began_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let store = store.clone();
        let scope = scope.clone();
        std::thread::spawn(move || {
            began_tx.send(()).unwrap();
            let result = store.put_record(&scope, bytes);
            done_tx.send(()).unwrap();
            result
        })
    };
    began_rx.recv().unwrap();
    let limit = Instant::now() + Duration::from_secs(1);
    let mut completed = false;
    while first.current() && Instant::now() < limit {
        if done_rx.try_recv().is_ok() {
            completed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let revoked_before_join = !first.current();
    let old_queue_refused = handle.admit_original(stamp).is_err();
    let write_waited_for_original_io = !completed && done_rx.try_recv().is_err();
    // Always release/join original custody before asserting the old/final result.
    drop(first);
    let result = worker.join().unwrap();
    let mut final_count = None;
    let limit = Instant::now() + Duration::from_secs(1);
    while final_count != Some(1) && Instant::now() < limit {
        final_count = handle.admit().ok().map(|lease| lease.facts().0);
        if final_count != Some(1) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let old_queue_still_refused = handle.admit_original(stamp).is_err();
    drop(handle);
    watcher.shutdown().unwrap();
    assert!(result.is_ok());
    assert!(
        revoked_before_join,
        "record birth must invalidate its native ContextView epoch"
    );
    assert!(
        write_waited_for_original_io,
        "record SQL must wait for original Conversation IO custody"
    );
    assert!(
        old_queue_refused && old_queue_still_refused,
        "queued old frames cannot borrow the later snapshot"
    );
    assert_eq!(final_count, Some(1));
}
