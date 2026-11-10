//! Real provider facts on the existing Store Registry; no provider/network IO.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait, PrivateProviderMediaSnapshot};
use std::time::{Duration, Instant};
fn admitted(snapshot: &PrivateProviderMediaSnapshot) -> crate::PrivateProviderMediaLease {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match snapshot.admit() {
            Ok(lease) => return lease,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original provider publication failed: {}", error.code),
        }
    }
}
#[test]
fn provider_snapshot_idle_publication_releases_for_actual_writer_and_never_revives_queued_epoch() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = Arc::new(delivered(&f, &context));
    let scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap();
    // Counterfactual actual old idle reader excludes the same writer.
    let old = f
        .store
        .try_private_provider_media_read(&context, &applied)
        .unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(old);
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let snapshot = store
        .register_private_provider_media_snapshot(context, applied)
        .unwrap();
    let io = admitted(&snapshot);
    let queued = io.epoch();
    drop(io);
    let writer = store
        .acquire_authority(
            &[scope.clone()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert!(
        !queued.current(),
        "pending writer must revoke cached output metadata"
    );
    assert!(snapshot.admit().is_err());
    drop(writer);
    let current = admitted(&snapshot);
    let successor = current.epoch();
    assert!(!queued.same_generation(&successor));
    assert_eq!(
        snapshot.admit_original(&queued).err().unwrap().code,
        "stale-media-epoch"
    );
    assert!(snapshot.admit_original(&successor).unwrap().current());
    drop(current);
    drop(snapshot);
    watcher.shutdown().unwrap();
}
#[test]
fn provider_snapshot_actual_io_drains_before_writer_and_current_revocation_is_definitive() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = Arc::new(delivered(&f, &context));
    let admin = f.store.revalidate(&f.admin).unwrap();
    let key = f.keys[0].clone();
    let scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let snapshot = store
        .register_private_provider_media_snapshot(context, applied)
        .unwrap();
    let io = admitted(&snapshot);
    let original = io.epoch();
    assert_eq!(
        store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(
        !io.current() && !original.current(),
        "original IO cannot continue after invalidation"
    );
    drop(io);
    let writer = store
        .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    drop(writer);
    drop(admitted(&snapshot));
    store.revoke(&admin, &key).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match snapshot.admit() {
            Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            _ => panic!("revoked provider context remained refreshable"),
        }
    }
    assert!(!original.current());
    drop(snapshot);
    watcher.shutdown().unwrap();
}
#[test]
fn provider_snapshot_steady_admission_needs_no_sql_and_original_expiry_remains_binding() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = Arc::new(delivered(&f, &context));
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let snapshot = store
        .register_private_provider_media_snapshot(context, applied)
        .unwrap();
    let initial = admitted(&snapshot);
    let original = initial.epoch();
    drop(initial);
    // Hold the real primary SQL mutex. Every steady publication admission must
    // still succeed: no SQL query or try-acquisition may run on this path.
    let sql = store.connection().unwrap();
    for _ in 0..1000 {
        let io = snapshot.admit_original(&original).unwrap();
        assert!(io.current());
        drop(io);
    }
    let expiry = Instant::now() + Duration::from_secs(6);
    while original.current() && Instant::now() < expiry {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !original.current(),
        "snapshot extended the original bounded read expiry"
    );
    assert!(snapshot.admit_original(&original).is_err());
    // The existing watcher uses immediate SQL and can join even while this
    // owner prevents an expiry refresh. No per-provider worker exists.
    drop(snapshot);
    watcher.shutdown().unwrap();
    drop(sql);
}
