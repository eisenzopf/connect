#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{
    record_retention::{Lifetime, Policy},
    Limits,
};
use serde_json::json;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    scope: RecordScope,
    uuid: String,
    handle: RecordHandle,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        store.connection().unwrap().execute_batch("INSERT INTO identities(id,alias,capabilities) VALUES('id_a','a','[]'),('id_b','b','[]');
        INSERT INTO conversations(id) VALUES('conv_one');
        INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES('conv_one','id_a','part_a','participant'),('conv_one','id_b','part_b','participant');
        INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('sess_one','conv_one','id_a','id_b','accepted','[\"audio\"]',100,100);").unwrap();
        let scope = RecordScope {
            tenant_id: "tnt_default".into(),
            conversation_id: "conv_one".into(),
            session_id: "sess_one".into(),
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let bytes=RecordBytes{uuid:uuid.clone(),body:serde_json::to_vec(&json!({"uuid":uuid,"subject":"original"})).unwrap(),manifest:serde_json::to_vec(&json!({"sources":[{"id":"document","target":{"kind":"document"},"state":"available","metadata_retention":{"kind":"until","until":"1970-01-01T00:03:20Z"},"content_retention":{"kind":"until-deleted"}}]})).unwrap()};
        let handle = store.put_record(&scope, bytes).unwrap();
        Self {
            _dir: dir,
            store,
            scope,
            uuid,
            handle,
        }
    }
    fn prepare(&self) -> String {
        self.store
            .prepare_record_retention(&self.uuid, 200, |_, _, _, _| Ok(Some(Decision::Delete)))
            .unwrap()
            .unwrap()
    }
    fn state(&self, op: &str) -> String {
        self.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM record_retention_jobs WHERE operation_id=?1",
                [op],
                |r| r.get(0),
            )
            .unwrap()
    }
}
#[test]
fn terminal_policy_freezes_before_materialization_and_survives_configuration_change() {
    let f = Fixture::new();
    let policy = Policy {
        name: "short-metadata".into(),
        metadata: Lifetime::After { seconds: 60 },
        transcript: Lifetime::After { seconds: 120 },
        ..Default::default()
    };
    f.store.configure_record_retention(&policy).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE sessions SET state='ended' WHERE id='sess_one'", [])
        .unwrap();
    f.store
        .configure_record_retention(&Policy::default())
        .unwrap();
    let uuid = f.store.pending_record_intents(1).unwrap().pop().unwrap();
    let original = super::super::outcomes::intent(&f.store.connection().unwrap(), &uuid)
        .unwrap()
        .unwrap();
    assert_eq!(original.retention, policy);
    let reopened = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        super::super::outcomes::intent(&reopened.connection().unwrap(), &uuid)
            .unwrap()
            .unwrap()
            .retention,
        policy
    );
    assert!(reopened
        .connection()
        .unwrap()
        .execute(
            "UPDATE session_record_retention SET policy_json='{}' WHERE uuid=?1",
            [uuid]
        )
        .is_err());
    let mut invalid = policy;
    invalid.metadata = Lifetime::After { seconds: 0 };
    assert!(f.store.configure_record_retention(&invalid).is_err());
}
#[test]
fn original_decision_cutoff_and_id_recover_after_restart_without_reselection_or_duplicate_journal()
{
    let f = Fixture::new();
    assert!(f.store.pending_record_retention(1, 199).unwrap().is_empty());
    assert_eq!(
        f.store.pending_record_retention(1, 200).unwrap(),
        vec![f.uuid.clone()]
    );
    let op = f.prepare();
    let reopened = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        reopened
            .prepare_record_retention(&f.uuid, 900, |_, _, _, _| panic!(
                "must not select at later cutoff"
            ))
            .unwrap(),
        Some(op.clone())
    );
    assert_eq!(
        reopened
            .connection()
            .unwrap()
            .query_row(
                "SELECT cutoff FROM record_retention_jobs WHERE operation_id=?1",
                [&op],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        200
    );
    assert!(reopened
        .apply_record_retention(&op, |_, expected, id, decision| {
            assert_eq!(expected, &f.handle);
            assert_eq!(id, op);
            assert!(matches!(decision, Decision::Delete));
            Ok(None)
        })
        .unwrap());
    let journal: u64 = reopened
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='record_unavailable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(journal, 2);
    assert!(!reopened
        .apply_record_retention(&op, |_, _, _, _| panic!())
        .unwrap());
    assert!(reopened
        .pending_record_retention(32, 1000)
        .unwrap()
        .is_empty());
    assert!(reopened
        .read_record(&f.scope, Some(&f.uuid), None, None)
        .unwrap()
        .is_none());
    assert_eq!(
        reopened
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM journal WHERE kind='record_unavailable'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        journal
    );
}
#[test]
fn quota_failure_rolls_back_expiry_and_receipt_but_retains_original_decision_for_retry() {
    let mut f = Fixture::new();
    let op = f.prepare();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let limit = f.store.limits.journal_events;
    f.store.limits.journal_events = 0;
    assert!(
        matches!(f.store.apply_record_retention(&op,|_,_,_,_|Ok(None)),Err(e) if e.code=="capacity-exceeded")
    );
    assert_eq!(f.state(&op), "pending");
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
    f.store.limits.journal_events = limit;
    assert!(f
        .store
        .apply_record_retention(&op, |_, _, _, _| Ok(None))
        .unwrap());
    assert_eq!(f.state(&op), "applied");
}
#[test]
fn concurrent_new_revision_supersedes_frozen_decision_without_overwriting_new_bytes() {
    let f = Fixture::new();
    let op = f.prepare();
    f.store
        .mutate_record(
            &f.scope,
            "competing",
            RecordMutationKind::Redact,
            "competitor-v1",
            &f.handle,
            |b| {
                let mut next = b.clone();
                next.body = serde_json::to_vec(&json!({"uuid":f.uuid,"subject":"new"})).unwrap();
                Ok(Some(next))
            },
        )
        .unwrap()
        .unwrap();
    assert!(!f
        .store
        .apply_record_retention(&op, |_, _, _, _| panic!("stale decision must not run"))
        .unwrap());
    assert_eq!(f.state(&op), "superseded");
    let read = f
        .store
        .read_record(&f.scope, Some(&f.uuid), None, None)
        .unwrap()
        .unwrap();
    assert_eq!(read.handle.revision, 2);
    drop(read);
    let next = f.prepare();
    assert_ne!(next, op);
    assert!(f
        .store
        .apply_record_retention(&next, |_, _, _, _| Ok(None))
        .unwrap());
}
