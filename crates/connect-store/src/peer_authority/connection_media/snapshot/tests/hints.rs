//! Observational metadata cannot become an IO reader or bypass its admission.
use super::*;

#[test]
fn packet_metadata_hint_peek_owns_no_reader_and_invalidates_sql_source_and_connection_changes() {
    let f = Fixture::new();
    let memo = Memo::default();
    assert_eq!(peek_map(&f.store, &memo, None, |v: &i64| *v).unwrap(), None);
    drop(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap());
    let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), Some(1));
    assert_eq!(
        cached(&f.store, &memo, None, |c| f.derive(c))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(
        peek_map(&f.store, &memo, Some(&new_metadata()), |v| *v).unwrap(),
        None
    );
    let other =
        Store::open_administration(f.store.path(), connect_protocol::Limits::default()).unwrap();
    assert_eq!(peek_map(&other, &memo, None, |v| *v).unwrap(), None);
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), None);
    drop(writer);
    let (value, read) = cached(&f.store, &memo, None, |c| f.derive(c)).unwrap();
    assert_eq!(value, 2);
    drop(read);
    let external = Connection::open(f.store.path()).unwrap();
    external
        .execute("UPDATE packet_snapshot_test SET value=3", [])
        .unwrap();
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), None);
    assert!(f.store.try_acquire_authority(&[f.write]).is_ok());
}

#[test]
fn packet_metadata_hint_peek_checks_clock_and_refuses_sql_without_waiting_on_metadata() {
    let f = Fixture::new();
    let memo = Memo::default();
    drop(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap());
    let locked = memo.0.lock().unwrap();
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), None);
    drop(locked);
    let sql = occupy_packet_sql(&f.store);
    assert_eq!(
        peek_map(&f.store, &memo, None, |v| *v).unwrap_err().code,
        "authority-busy"
    );
    drop(sql);
    let second = memo.0.lock().unwrap()[0].version.second;
    while crate::now() == second {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), None);
    drop(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap());
    assert_eq!(peek_map(&f.store, &memo, None, |v| *v).unwrap(), Some(1));
}
