//! Controlled SQL races test capture invalidation, not physical media delivery.
use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    read: AuthorityScope,
    write: AuthorityScope,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(
                &dir.path().join("state.sqlite3"),
                connect_protocol::Limits::default(),
            )
            .unwrap(),
        );
        store
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TABLE packet_snapshot_test(value INTEGER NOT NULL);
                 INSERT INTO packet_snapshot_test VALUES(1);",
            )
            .unwrap();
        Self {
            _dir: dir,
            store,
            read: AuthorityScope::identity("packet-test", AuthorityMode::Read).unwrap(),
            write: AuthorityScope::identity("packet-test", AuthorityMode::Write).unwrap(),
        }
    }
    fn derive(&self, c: &Connection) -> Result<(i64, Vec<AuthorityScope>)> {
        let value = c
            .query_row("SELECT value FROM packet_snapshot_test", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        Ok((value, vec![self.read.clone()]))
    }
}

#[test]
fn packet_snapshot_external_commit_refuses_stale_facts_releases_reader_and_recovers() {
    let f = Fixture::new();
    let external = Connection::open(f.store.path()).unwrap();
    let c = f.store.try_connection().unwrap();
    let result = acquire(&f.store, &c, |c| {
        let facts = f.derive(c)?;
        // A different process/Store can commit despite this Store's SQL mutex.
        // Model a completed mutation between fact resolution and file admission.
        external
            .execute("UPDATE packet_snapshot_test SET value=2", [])
            .unwrap();
        Ok(facts)
    });
    assert_eq!(result.unwrap_err().code, "authority-busy");
    drop(c);
    // No stale reader survives the failed capture to obstruct a current writer.
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    let c = f.store.try_connection().unwrap();
    let (value, held) = acquire(&f.store, &c, |c| f.derive(c)).unwrap();
    assert_eq!(value, 2);
    drop(c);
    assert!(f.store.try_connection().is_ok(), "SQL ends before IO");
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.write.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(held);
    assert!(f.store.try_acquire_authority(&[f.write]).is_ok());
}

#[test]
fn packet_snapshot_own_connection_write_also_invalidates_capture() {
    let f = Fixture::new();
    let c = f.store.try_connection().unwrap();
    let result = acquire(&f.store, &c, |c| {
        let facts = f.derive(c)?;
        c.execute("UPDATE packet_snapshot_test SET value=3", [])
            .unwrap();
        Ok(facts)
    });
    assert_eq!(result.unwrap_err().code, "authority-busy");
    assert_eq!(acquire(&f.store, &c, |c| f.derive(c)).unwrap().0, 3);
}

#[test]
fn packet_snapshot_refuses_queued_writer_without_waiting_under_sql_and_recovers() {
    let f = Fixture::new();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let store = f.store.clone();
    let write = f.write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(3)))
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match f.store.try_acquire_authority(&[f.read.clone()]) {
            Err(error) => {
                assert_eq!(error.code, "authority-busy");
                break;
            }
            Ok(read) => drop(read),
        }
        assert!(Instant::now() < deadline, "writer never queued");
        std::thread::yield_now();
    }
    let c = f.store.try_connection().unwrap();
    let start = Instant::now();
    assert_eq!(
        acquire(&f.store, &c, |c| f.derive(c)).unwrap_err().code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(c);
    drop(held);
    let exclusive = writer.join().unwrap().unwrap();
    let c = f.store.try_connection().unwrap();
    assert_eq!(
        acquire(&f.store, &c, |c| f.derive(c)).unwrap_err().code,
        "authority-busy"
    );
    drop(exclusive);
    assert_eq!(acquire(&f.store, &c, |c| f.derive(c)).unwrap().0, 1);
}
