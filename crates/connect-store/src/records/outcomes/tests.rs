#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::Limits;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        {
            let c = store.connection().unwrap();
            c.execute_batch("INSERT INTO identities(id,alias,capabilities) VALUES('id_a','a','[]'),('id_b','b','[]');
    INSERT INTO conversations(id) VALUES('conv_one');
    INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES('conv_one','id_a','part_a','participant'),('conv_one','id_b','part_b','participant');
    INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('sess_one','conv_one','id_a','id_b','accepted','[\"audio\"]',100,100);
    INSERT INTO session_participants VALUES('sess_one','conv_one','tnt_default','part_a',1,'participant','present'),('sess_one','conv_one','tnt_default','part_b',1,'participant','present');").unwrap();
        }
        Self { _dir: dir, store }
    }
    fn end(&self, state: &str) {
        self.store.connection().unwrap().execute("UPDATE sessions SET state=?1,ended_reason='original-reason',updated_at=200 WHERE id='sess_one'",[state]).unwrap();
    }
    fn pending(&self) -> RecordIntent {
        let id = self.store.pending_record_intents(1).unwrap().pop().unwrap();
        intent(&self.store.connection().unwrap(), &id)
            .unwrap()
            .unwrap()
    }
}
fn encoded(i: &RecordIntent) -> (RecordBytes, Vec<RecordSourceEpoch>) {
    (RecordBytes{uuid:i.uuid.clone(),body:serde_json::to_vec(&serde_json::json!({"vcon":"0.4.0","uuid":i.uuid,"created_at":"2026-10-21T00:00:00Z"})).unwrap(),manifest:b"{}".to_vec()},vec![RecordSourceEpoch{source_group:"session".into(),participant_id:"part_a".into(),membership_epoch:1}])
}
#[test]
fn terminal_intent_is_atomic_stable_and_frozen_for_every_terminal_state() {
    for state in ["ended", "rejected", "cancelled", "failed"] {
        let f = Fixture::new();
        {
            let mut c = f.store.connection().unwrap();
            let tx = c.transaction().unwrap();
            tx.execute("UPDATE sessions SET state=?1 WHERE id='sess_one'", [state])
                .unwrap();
            assert_eq!(
                tx.query_row("SELECT count(*) FROM session_record_intents", [], |r| r
                    .get::<_, usize>(
                    0
                ))
                .unwrap(),
                1
            );
        }
        assert!(f.store.pending_record_intents(1).unwrap().is_empty());
        f.end(state);
        let original = f.pending();
        assert_eq!(
            uuid::Uuid::parse_str(&original.uuid)
                .unwrap()
                .get_version_num(),
            8
        );
        assert_eq!(original.snapshot["state"], state);
        assert_eq!(
            original.snapshot["appearances"].as_array().unwrap().len(),
            2
        );
        assert_eq!(original.snapshot["audience"].as_array().unwrap().len(), 2);
        let c = f.store.connection().unwrap();
        c.execute_batch(
            "UPDATE sessions SET ended_reason='later-reason',updated_at=300 WHERE id='sess_one';
   UPDATE session_participants SET state='left' WHERE session_id='sess_one';
   UPDATE participants SET active=0 WHERE participant_id='part_a';
   UPDATE participants SET active=1,membership_epoch=2 WHERE participant_id='part_a';",
        )
        .unwrap();
        drop(c);
        let after = f.pending();
        assert_eq!(original.uuid, after.uuid);
        assert_eq!(original.snapshot, after.snapshot);
    }
}
#[test]
fn finalization_commits_record_epochs_and_snapshot_release_or_rolls_back_all_of_them() {
    let f = Fixture::new();
    f.end("ended");
    let original = f.pending();
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| {
            let (bytes, mut epochs) = encoded(i);
            epochs[0].participant_id = "part_foreign".into();
            Ok((bytes, epochs))
        })
        .is_err());
    assert_eq!(f.pending().snapshot, original.snapshot);
    assert!(f
        .store
        .list_records("tnt_default", "conv_one", None)
        .unwrap()
        .is_empty());
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .unwrap());
    assert!(f.store.pending_record_intents(1).unwrap().is_empty());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT snapshot_json IS NULL FROM session_record_intents WHERE uuid=?1",
            [&original.uuid],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        true
    );
    assert_eq!(c.query_row("SELECT membership_epoch FROM session_record_audiences WHERE uuid=?1 AND source_group='session' AND participant_id='part_a'",[&original.uuid],|r|r.get::<_,u64>(0)).unwrap(),1);
    drop(c);
    assert!(!f
        .store
        .finalize_record_intent(&original.uuid, |_| panic!(
            "completed intent cannot run transform again"
        ))
        .unwrap());
    let h = f
        .store
        .list_records("tnt_default", "conv_one", None)
        .unwrap()
        .pop()
        .unwrap();
    f.store
        .mutate_record(
            &original.scope,
            "delete",
            RecordMutationKind::Delete,
            "delete-v1",
            &h,
            |_| Ok(None),
        )
        .unwrap();
    assert!(!f
        .store
        .finalize_record_intent(&original.uuid, |_| panic!(
            "cannot resurrect deleted record"
        ))
        .unwrap());
    assert!(f
        .store
        .list_records("tnt_default", "conv_one", None)
        .unwrap()
        .is_empty());
}
#[test]
fn initial_terminal_insert_freezes_only_original_prescribed_session_audience() {
    let f = Fixture::new();
    let c = f.store.connection().unwrap();
    for (id, mode) in [
        ("sess_default", "participants"),
        ("sess_explicit", "explicit"),
    ] {
        c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at,audience_mode) VALUES(?1,'conv_one','id_a','id_b','failed','[]',100,100,?2)",params![id,mode]).unwrap();
        let snapshot: String = c
            .query_row(
                "SELECT snapshot_json FROM session_record_intents WHERE session_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
        assert_eq!(
            snapshot["audience"].as_array().unwrap().len(),
            if mode == "participants" { 2 } else { 0 }
        );
        assert!(snapshot["appearances"].as_array().unwrap().is_empty());
    }
}
#[test]
fn outcome_slot_remains_available_at_the_ordinary_record_capacity() {
    let f = Fixture::new();
    {
        let mut c = f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        for index in 0..MAX_RECORDS - 1 {
            tx.execute("INSERT INTO session_records(uuid,tenant_id,conversation_id,session_id,location,revision,body,manifest,body_hash,manifest_hash) VALUES(?1,'tnt_default','conv_one','sess_one',?2,1,X'7B7D',X'7B7D','fixture','fixture')",params![format!("fixture_{index}"),format!("fixture:{index}")]).unwrap();
        }
        tx.commit().unwrap();
    }
    f.end("ended");
    let original = f.pending();
    let mut other = original.clone();
    other.uuid = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        f.store
            .put_record(&other.scope, encoded(&other).0)
            .err()
            .unwrap()
            .code,
        "capacity-exceeded"
    );
    assert!(f
        .store
        .put_record(&original.scope, encoded(&original).0)
        .is_err());
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .unwrap());
    assert_eq!(
        f.store
            .list_records("tnt_default", "conv_one", None)
            .unwrap()
            .len(),
        MAX_RECORDS
    );
}

#[test]
fn pending_discovery_and_atomic_ready_journal_survive_reopen_without_duplicate_finalization() {
    use connect_protocol::events::{EventQuery, JournalData, RecordAvailability, SnapshotQuery};
    let f = Fixture::new();
    let key = f
        .store
        .issue("id_a", crate::Scope::operator(), crate::now() + 300)
        .unwrap();
    f.end("ended");
    let original = f.pending();
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &key.id).unwrap();
    assert!(matches!(
        super::super::read::availability(&c, &actor, "conv_one", "sess_one").unwrap(),
        Some(RecordAvailability::Pending { .. })
    ));
    drop(c);
    let cursor = f
        .store
        .conversation_snapshot(&key.id, "conv_one", SnapshotQuery::default())
        .unwrap()
        .cursor;
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .unwrap());
    assert!(!f
        .store
        .finalize_record_intent(&original.uuid, |_| panic!("must not recreate"))
        .unwrap());
    let path = f.store.path().to_owned();
    drop(f.store);
    let store = Store::open(&path, Limits::default()).unwrap();
    let page = store
        .conversation_events(
            &key.id,
            "conv_one",
            EventQuery {
                after: Some(cursor),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        page.events
            .iter()
            .filter(|e| matches!(
                e.data,
                JournalData::Record(RecordAvailability::Ready { .. })
            ))
            .count(),
        1
    );
    assert!(store
        .conversation_snapshot(&key.id, "conv_one", SnapshotQuery::default())
        .unwrap()
        .items
        .iter()
        .any(|i| matches!(
            i,
            connect_protocol::events::SnapshotItem::Record(RecordAvailability::Ready { .. })
        )));
}
#[test]
fn readiness_journal_capacity_refusal_rolls_back_bytes_and_retries_original_intent() {
    let f = Fixture::new();
    f.end("ended");
    let original = f.pending();
    let c = f.store.connection().unwrap();
    let old: u64 = c
        .query_row("SELECT count(*) FROM journal", [], |r| r.get(0))
        .unwrap();
    drop(c);
    let path = f.store.path().to_owned();
    drop(f.store);
    let mut limits = Limits::default();
    limits.journal_events = old.saturating_add(1);
    let store = Store::open(&path, limits).unwrap();
    assert_eq!(
        store
            .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert!(store
        .list_records("tnt_default", "conv_one", None)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.pending_record_intents(1).unwrap(),
        vec![original.uuid.clone()]
    );
    drop(store);
    let store = Store::open(&path, Limits::default()).unwrap();
    assert!(store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .unwrap());
}

#[test]
fn readiness_failure_rolls_back_record_audience_journal_and_snapshot_release_then_recovers_once() {
    let f = Fixture::new();
    f.end("ended");
    let original = f.pending();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_ready BEFORE INSERT ON record_ready_facts BEGIN SELECT RAISE(ABORT,'controlled readiness failure'); END").unwrap();
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .is_err());
    assert_eq!(f.pending().snapshot, original.snapshot);
    let c = f.store.connection().unwrap();
    for table in [
        "record_ready_facts",
        "record_ready_audiences",
        "session_records",
        "session_record_audiences",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    c.execute_batch("DROP TRIGGER fail_ready").unwrap();
    drop(c);
    assert!(f
        .store
        .finalize_record_intent(&original.uuid, |i| Ok(encoded(i)))
        .unwrap());
    assert!(!f
        .store
        .finalize_record_intent(&original.uuid, |_| panic!("replay is read only"))
        .unwrap());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM record_ready_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM session_lifecycle_hints WHERE kind='record_ready'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
