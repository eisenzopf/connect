//! Committed read snapshots must end with every metadata page, including empty
//! pages. A later writer/schema change cannot be hidden by an idle poller.
use connect_protocol::Limits;
use connect_store::Store;

#[test]
fn repeated_session_metadata_pages_release_wal_snapshots() {
    let private = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/gate-6/session-metadata-poll-tests");
    std::fs::create_dir_all(&private).unwrap();
    let directory = tempfile::tempdir_in(private).unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::initialize(&path, Limits::default()).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..30 {
        assert!(store.session_reconnect_page("").unwrap().1.is_empty());
        assert!(store.session_lifecycle_hint_page(0).unwrap().is_empty());
    }
    eprintln!(
        "SESSION_METADATA_EMPTY_30_PAIRS_MS={}",
        start.elapsed().as_millis()
    );
    // Warm read connections must see schema commits and release their WAL
    // readers immediately, rather than retaining a snapshot between polls.
    writer
        .execute_batch(
            "CREATE TABLE fixture_revision(value INTEGER); INSERT INTO fixture_revision VALUES(1);",
        )
        .unwrap();
    assert!(store.session_reconnect_page("").unwrap().1.is_empty());
    assert!(store.session_lifecycle_hint_page(0).unwrap().is_empty());
    let checkpoint: (i64, i64, i64) = writer
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(
        checkpoint,
        (0, 0, 0),
        "an idle metadata page must not pin the WAL"
    );
    writer.execute_batch("DROP TABLE fixture_revision").unwrap();
    assert!(store.session_reconnect_page("").unwrap().1.is_empty());
    assert!(store.session_lifecycle_hint_page(0).unwrap().is_empty());
}
