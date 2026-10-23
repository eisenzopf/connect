#![allow(clippy::unwrap_used)]
use crate::{AuthorityMode, AuthorityScope, AuthorityWait, Scope, Store};
use connect_protocol::{Limits, Result};
use rusqlite::params;
use std::time::Duration;
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    identity: String,
}
impl Fixture {
    fn new(count: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let identity = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let key = store
            .issue(&identity, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        {
            let mut c = store.connection().unwrap();
            let tx = c.transaction().unwrap();
            tx.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('epoch-device',?1,'tnt_default','fixture','fixture')", [&identity]).unwrap();
            tx.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES('epoch-endpoint',?1,'tnt_default','epoch-device','webrtc','fixture-address','[\"audio\",\"video\"]','verified','fixture-proof',?2)", params![identity,crate::now()]).unwrap();
            for n in 0..count {
                let cid = format!("epoch-conversation-{n:04}");
                let sid = format!("epoch-session-{n:04}");
                let conn = format!("epoch-connection-{n:04}");
                let part = format!("epoch-participant-{n:04}");
                tx.execute("INSERT INTO conversations(id) VALUES(?1)", [&cid])
                    .unwrap();
                tx.execute("INSERT INTO participants(conversation_id,identity_id,participant_id) VALUES(?1,?2,?3),(?1,?4,?5)", params![cid,identity,part,peer,format!("epoch-peer-{n:04}")]).unwrap();
                tx.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?4,'ready','[\"audio\",\"video\"]',?5,?5)", params![sid,cid,identity,peer,crate::now()]).unwrap();
                tx.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state", params![sid,cid,part]).unwrap();
                tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default','epoch-endpoint',4,'active','old-runtime',100,'{\"ingress\":true,\"egress\":true}')", params![conn,sid,part,identity]).unwrap();
                tx.execute("INSERT INTO session_bindings(session_id,participant_id,credential_id,connection_id,stream_id,state,updated_at) VALUES(?1,?2,?3,?4,'fixture-stream','ready',100)", params![sid,identity,key,conn]).unwrap();
            }
            tx.commit().unwrap();
        }
        Self {
            dir,
            store,
            identity,
        }
    }
}
fn blocked<T: Send>(
    f: &Fixture,
    scope: AuthorityScope,
    write_scope: AuthorityScope,
    write: impl FnOnce() -> Result<T> + Send,
) -> T {
    let read = f
        .store
        .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(write);
        crate::authority_fences::test_wait_for_writer(&f.store, &write_scope);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        writer.join().unwrap().unwrap()
    })
}
#[test]
fn owned_connection_epoch_waits_for_delivery_and_never_revives_unknown_or_terminal_state() {
    let f = Fixture::new(4);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE connections SET observed_runtime_id=?1 WHERE id='epoch-connection-0001'",
            [f.store.runtime_epoch()],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired' WHERE id='epoch-connection-0002'",
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='unknown' WHERE id='epoch-connection-0003'",
            [],
        )
        .unwrap();
    }
    let count = blocked(
        &f,
        AuthorityScope::identity(&f.identity, AuthorityMode::Read).unwrap(),
        AuthorityScope::identity(&f.identity, AuthorityMode::Write).unwrap(),
        || f.store.fence_unobserved_connections(1000),
    );
    assert_eq!(count, 1);
    assert_eq!(f.store.fence_unobserved_connections(1000).unwrap(), 0);
    let c = f.store.connection().unwrap();
    let row:(String,u64,Option<String>,Option<String>) = c.query_row("SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id='epoch-connection-0000'", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row, ("interrupted".into(), 5, None, None));
    for (id, expected) in [("0001", "active"), ("0002", "retired"), ("0003", "unknown")] {
        let (state, generation): (String, u64) = c
            .query_row(
                "SELECT state,generation FROM connections WHERE id=?1",
                [format!("epoch-connection-{id}")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), generation), (expected, 4));
    }
}
#[test]
fn owned_video_reconciliation_waits_for_conversation_and_is_idempotent() {
    let f = Fixture::new(1);
    let count = blocked(
        &f,
        AuthorityScope::conversation("epoch-conversation-0000", AuthorityMode::Read).unwrap(),
        AuthorityScope::conversation("epoch-conversation-0000", AuthorityMode::Write).unwrap(),
        || f.store.reconcile_interrupted_video_connections(),
    );
    assert_eq!(count, 1);
    assert_eq!(
        f.store.reconcile_interrupted_video_connections().unwrap(),
        0
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT state FROM sessions", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "accepted"
    );
    assert_eq!(
        c.query_row("SELECT state FROM session_bindings", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "ended"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM journal WHERE kind='session_interrupted'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn bounded_startup_drains_1030_connection_and_video_scopes_under_owned_runtime() {
    let f = Fixture::new(1030);
    let (ids, guard) = f
        .store
        .startup_batch(crate::startup_authority::Kind::ConnectionEpoch, 1000)
        .unwrap();
    assert_eq!(ids.len(), 63);
    drop(guard);
    let store = Store::open(&f.dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM connections WHERE state='interrupted' AND generation=5",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        1030
    );
    let (ids, guard) = store
        .startup_batch(crate::startup_authority::Kind::VideoBindings, 1000)
        .unwrap();
    assert_eq!(ids.len(), 64);
    drop(guard);
    assert_eq!(
        store.reconcile_interrupted_video_connections().unwrap(),
        1030
    );
    let c = store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sessions WHERE state='accepted'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        1030
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM session_bindings WHERE state='ready'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM journal WHERE kind='session_interrupted'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        2060
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get::<_, usize>(0)
        })
        .unwrap(),
        0
    );
}
#[test]
fn startup_batch_never_substitutes_newly_eligible_ids_after_admission() {
    let f = Fixture::new(2);
    let kind = crate::startup_authority::Kind::ConnectionEpoch;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET observed_runtime_id=?1 WHERE id='epoch-connection-0000'",
            [f.store.runtime_epoch()],
        )
        .unwrap();
    let (ids, guard) = f.store.startup_batch(kind, 1000).unwrap();
    assert_eq!(ids, vec!["epoch-connection-0001"]);
    let mut c = f.store.connection().unwrap();
    c.execute(
        "UPDATE connections SET observed_runtime_id='old-runtime' WHERE id='epoch-connection-0000'",
        [],
    )
    .unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let selected = f.store.startup_rows(&tx, kind, &ids, &guard).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, ids[0]);
    assert!(!guard.covers(&[AuthorityScope::conversation(
        "epoch-conversation-0000",
        AuthorityMode::Write
    )
    .unwrap()]));
    tx.rollback().unwrap();
    drop(c);
    drop(guard);
    assert_eq!(f.store.fence_unobserved_connections(1000).unwrap(), 2);
}
#[test]
fn administration_open_cannot_run_epoch_video_or_global_worker_startup() {
    let f = Fixture::new(1);
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        admin.fence_unobserved_connections(1000).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        admin
            .reconcile_interrupted_video_connections()
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        admin.fence_transition_workers(1000).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn new_owner_in_same_process_and_restored_copy_require_fresh_connection_observations() {
    let Fixture { dir, store, .. } = Fixture::new(1);
    let epoch = store.runtime_epoch().to_owned();
    let backup = dir.path().join("copy.sqlite3");
    store.backup(&backup).unwrap();
    let copy = Store::open(&backup, Limits::default()).unwrap();
    assert_ne!(copy.runtime_epoch(), epoch);
    assert_eq!(
        copy.connection()
            .unwrap()
            .query_row("SELECT state FROM connections", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "interrupted"
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("SELECT state FROM connections", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "active"
    );
    let path = store.path().to_owned();
    drop(store);
    let next = Store::open(&path, Limits::default()).unwrap();
    assert_ne!(next.runtime_epoch(), epoch);
    let row: (String, u64, Option<String>) = next
        .connection()
        .unwrap()
        .query_row(
            "SELECT state,generation,observed_runtime_id FROM connections",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("interrupted".into(), 5, None));
}
