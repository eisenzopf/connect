//! Dedicated committed metadata readers for the two 100 ms Session pollers.
//! Reusing connections avoids reparsing the complete SQLite schema each tick.
//! A connection retains no transaction or grant between calls; each page owns
//! its original deferred snapshot and later delivery still checks authority.
use crate::*;
use std::sync::MutexGuard;

pub(crate) struct Readers {
    reconnect: Mutex<Connection>,
    lifecycle: Mutex<Connection>,
}
fn open(path: &Path) -> Result<Mutex<Connection>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| Error::storage())?;
    connection
        .busy_timeout(Duration::from_millis(50))
        .map_err(|_| Error::storage())?;
    connection
        .execute_batch("PRAGMA query_only=ON")
        .map_err(|_| Error::storage())?;
    Ok(Mutex::new(connection))
}
impl Readers {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            reconnect: open(path)?,
            lifecycle: open(path)?,
        })
    }
    pub(crate) fn reconnect(&self) -> Result<MutexGuard<'_, Connection>> {
        Self::take(&self.reconnect)
    }
    pub(crate) fn lifecycle(&self) -> Result<MutexGuard<'_, Connection>> {
        Self::take(&self.lifecycle)
    }
    fn take(connection: &Mutex<Connection>) -> Result<MutexGuard<'_, Connection>> {
        connection.try_lock().map_err(|_| {
            Error::new(
                "authority-busy",
                "Session metadata reader is already in use",
            )
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn session_readers_are_read_only_independent_and_refresh_committed_snapshots() {
        let private = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/gate-6/session-metadata-poll-tests");
        std::fs::create_dir_all(&private).unwrap();
        let directory = tempfile::tempdir_in(private).unwrap();
        let store =
            Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
        let writer = store.connection().unwrap();
        writer.execute_batch("CREATE TABLE fixture_revision(value INTEGER); INSERT INTO fixture_revision VALUES(1)").unwrap();
        let mut reconnect = store.session_readers.reconnect().unwrap();
        assert_eq!(
            store.session_readers.reconnect().unwrap_err().code,
            "authority-busy"
        );
        assert!(reconnect
            .execute("UPDATE fixture_revision SET value=100", [])
            .is_err());
        let transaction = reconnect.transaction().unwrap();
        let value = |c: &Connection| {
            c.query_row("SELECT value FROM fixture_revision", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        assert_eq!(value(&transaction), 1);
        writer
            .execute("UPDATE fixture_revision SET value=2", [])
            .unwrap();
        assert_eq!(
            value(&transaction),
            1,
            "one page keeps its committed snapshot"
        );
        assert_eq!(
            value(&store.session_readers.lifecycle().unwrap()),
            2,
            "the other poller owns an independent fresh snapshot"
        );
        drop(transaction);
        assert_eq!(
            value(&reconnect),
            2,
            "later pages cannot reuse the old snapshot"
        );
        writer.execute_batch("CREATE TABLE fixture_new_schema(value INTEGER); INSERT INTO fixture_new_schema VALUES(3)").unwrap();
        assert_eq!(
            reconnect
                .query_row("SELECT value FROM fixture_new_schema", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}
