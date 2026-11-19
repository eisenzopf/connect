//! Real current Store authority intervals; native Closed/OLD restore is NOT modeled as proved here.
use super::*;
use std::time::Duration;
struct Writer(
    Option<std::thread::JoinHandle<crate::Result<std::sync::Arc<crate::AuthorityGuardSet>>>>,
);
impl Writer {
    fn join(mut self) {
        drop(self.0.take().unwrap().join().unwrap().unwrap());
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(original) = self.0.take() {
            let _ = original.join();
        }
    }
}

#[test]
fn original_video_restore_read_admits_old_and_exact_publishers_once_and_releases_before_writer() {
    let (f, _key, _target, _command, sources) = fixture_with_sources();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let attempts = f.store.authority.test_admission_attempts();
    let read = f
        .store
        .try_native_video_restore_read(&sources[0], std::slice::from_ref(&sources[1]))
        .unwrap();
    assert_eq!(
        f.store.authority.test_admission_attempts(),
        attempts + 1,
        "one COMPLETE interval, no nested pair admission"
    );
    assert!(read.current());
    assert_eq!(
        read.old().target().runtime_connection_id,
        sources[0].runtime_connection_id()
    );
    let pair = read
        .received_from(sources[1].runtime_connection_id())
        .unwrap();
    assert_eq!(
        pair.source().unwrap().runtime_connection_id,
        sources[1].runtime_connection_id()
    );
    assert!(read.received_from("unselected-source").is_none());
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    let peer = Store::open_administration(f.store.path(), f.store.limits.clone()).unwrap();
    let queued = scope.clone();
    let writer = Writer(Some(std::thread::spawn(move || {
        peer.acquire_authority(
            &[queued],
            &crate::AuthorityWait::new(Duration::from_secs(3)),
        )
    })));
    crate::authority_fences::test_wait_for_writer(&f.store, &scope);
    assert!(
        read.current(),
        "unrelated waiting writer cannot invalidate an admitted interval"
    );
    let refused = f
        .store
        .try_native_video_restore_read(&sources[0], std::slice::from_ref(&sources[1]))
        .err()
        .unwrap();
    assert_eq!(refused.code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(read);
    writer.join();
    assert!(f
        .store
        .try_native_video_restore_read(&sources[0], std::slice::from_ref(&sources[1]))
        .unwrap()
        .current());
}
#[test]
fn original_video_restore_read_refuses_foreign_duplicate_and_newly_revoked_publishers_without_effects(
) {
    let (f, _key, _target, _command, sources) = fixture_with_sources();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let attempts = f.store.authority.test_admission_attempts();
    assert_eq!(
        f.store
            .try_native_video_restore_read(&sources[0], &[sources[1].clone(), sources[1].clone()])
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store.authority.test_admission_attempts(),
        attempts,
        "duplicate scope refuses before admission"
    );
    let (_foreign, _k, _t, _c, other) = fixture_with_sources();
    assert_eq!(
        f.store
            .try_native_video_restore_read(&sources[0], std::slice::from_ref(&other[1]))
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let original = f
        .store
        .try_native_video_restore_read(&sources[0], std::slice::from_ref(&sources[1]))
        .unwrap();
    assert!(original.current());
    drop(original);
    {
        let c = f.change_fixture();
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[1]])
            .unwrap();
    }
    let revoked = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.revalidate(&f.keys[1]).unwrap_err().code,
        "unauthorized",
        "the selected publisher's original credential was actually revoked"
    );
    assert!(
        f.store
            .try_native_video_restore_read(&sources[0], &[])
            .unwrap()
            .current(),
        "the unaffected OLD credential remains current; no target permission is renewed"
    );
    let until = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let refused = f
            .store
            .try_native_video_restore_read(&sources[0], std::slice::from_ref(&sources[1]))
            .err()
            .unwrap();
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), revoked);
        if refused.code == "authority-busy" && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        assert_eq!(refused.code, "unauthorized");
        break;
    }
}
