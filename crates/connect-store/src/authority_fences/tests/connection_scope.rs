//! Fence namespace/isolation only. These are not application or decoded IO grants.
use super::*;

#[test]
fn connection_namespace_is_distinct_and_fifty_peer_fences_survive_identity_cache_eviction() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    let id = "same_connection_record_identity";
    let cases = [
        (Kind::Identity, b"identity".as_slice()),
        (Kind::Conversation, b"conversation".as_slice()),
        (Kind::Record, b"record".as_slice()),
        (Kind::Connection, b"connection".as_slice()),
    ];
    let mut names = std::collections::BTreeSet::new();
    for (kind, domain) in cases {
        let name = manager.name(&(kind, id.into())).unwrap();
        assert_eq!(
            name,
            opaque(&manager.secret, domain, id.as_bytes()).unwrap()
        );
        assert!(names.insert(name));
    }
    let mut scopes =
        vec![AuthorityScope::conversation("fifty-native-session", AuthorityMode::Read).unwrap()];
    for index in 0..50 {
        scopes.push(
            AuthorityScope::identity(&format!("peer-{index:02}"), AuthorityMode::Read).unwrap(),
        );
        scopes.push(
            AuthorityScope::connection(&format!("connection-{index:02}"), AuthorityMode::Read)
                .unwrap(),
        );
    }
    assert_eq!(scopes.len(), 101);
    let held = f
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(2)))
        .unwrap();
    assert!(f.store.authority_covers(&held, &scopes));
    assert_eq!(
        manager.file_identities.read().unwrap().len(),
        MAX_FILE_IDENTITIES
    );
    // The bounded inode-name metadata can evict; the original real resource
    // file guards remain owned and still exclude a peer Store's exact Writer.
    let peer = f.peer();
    let own = AuthorityScope::connection("connection-00", AuthorityMode::Write).unwrap();
    let refused = peer
        .try_acquire_authority(std::slice::from_ref(&own))
        .err()
        .unwrap();
    let shared = peer.try_acquire_authority(&scopes).unwrap();
    let shared_covers = peer.authority_covers(&shared, &scopes);
    drop(shared);
    drop(held);
    let admitted = peer.try_acquire_authority(&[own]);
    assert_eq!(refused.code, "authority-busy");
    assert!(shared_covers);
    assert!(admitted.is_ok());
}

#[test]
fn connection_writer_excludes_affected_pair_fences_without_excluding_unrelated_pair() {
    let f = Fixture::new();
    let cid = AuthorityScope::conversation("scope-isolation-session", AuthorityMode::Read).unwrap();
    let own = AuthorityScope::connection("changed-connection-0", AuthorityMode::Read).unwrap();
    let target = AuthorityScope::connection("changed-target-0", AuthorityMode::Read).unwrap();
    let healthy = [
        cid.clone(),
        AuthorityScope::connection("healthy-source-1", AuthorityMode::Read).unwrap(),
        AuthorityScope::connection("healthy-receiver-2", AuthorityMode::Read).unwrap(),
    ];
    let old_pair = [cid.clone(), own.clone(), healthy[1].clone()];
    let target_pair = [cid.clone(), target.clone(), healthy[2].clone()];
    let io = f.store.try_acquire_authority(&healthy).unwrap();
    let writer = f
        .store
        .try_acquire_authority(&[cid, own.write_scope(), target.write_scope()])
        .unwrap();
    let old_refused = f.store.try_acquire_authority(&old_pair).err().unwrap();
    let target_refused = f.store.try_acquire_authority(&target_pair).err().unwrap();
    let new_healthy = f.store.try_acquire_authority(&healthy);
    let fresh_covered = new_healthy
        .as_ref()
        .is_ok_and(|proof| f.store.authority_covers(proof, &healthy));
    let original_covered = f.store.authority_covers(&io, &healthy);
    drop(new_healthy);
    drop(writer);
    drop(io);
    assert_eq!(old_refused.code, "authority-busy");
    assert_eq!(target_refused.code, "authority-busy");
    assert!(original_covered && fresh_covered);
    assert!(f.store.try_acquire_authority(&old_pair).is_ok());
    assert!(f.store.try_acquire_authority(&target_pair).is_ok());
}

#[test]
fn evicted_connection_resource_still_matches_same_manager_live_reader_descriptor() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    let mut scopes =
        vec![AuthorityScope::conversation("fifty-live-native-peers", AuthorityMode::Read).unwrap()];
    for index in 0..50 {
        scopes.push(
            AuthorityScope::identity(&format!("live-peer-{index:02}"), AuthorityMode::Read)
                .unwrap(),
        );
        scopes.push(
            AuthorityScope::connection(&format!("live-connection-{index:02}"), AuthorityMode::Read)
                .unwrap(),
        );
    }
    let held = f
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(2)))
        .unwrap();
    let own = AuthorityScope::connection("live-connection-00", AuthorityMode::Write).unwrap();
    let name = manager
        .name(&(Kind::Connection, "live-connection-00".into()))
        .unwrap();
    let resource_name = format!("{name}.resource");
    let churn = |round| {
        for index in 0..80 {
            let proof = f
                .store
                .try_acquire_authority(&[AuthorityScope::record(
                    &format!("eviction-{round}-{index:02}"),
                    AuthorityMode::Read,
                )
                .unwrap()])
                .unwrap();
            drop(proof);
        }
        assert!(!manager
            .file_identities
            .read()
            .unwrap()
            .contains_key(&resource_name));
        assert_eq!(
            manager.file_identities.read().unwrap().len(),
            MAX_FILE_IDENTITIES
        );
    };
    churn(0);
    let untampered = f
        .store
        .try_acquire_authority(std::slice::from_ref(&own))
        .err()
        .unwrap();
    assert_eq!(untampered.code, "authority-busy");
    drop(held);
    // No temporary lookup alias may outlive the failed Writer attempt.
    let released = f
        .store
        .try_acquire_authority(std::slice::from_ref(&own))
        .unwrap();
    drop(released);
    let held = f
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(2)))
        .unwrap();
    churn(1);
    let original_path = manager.directory.join(&resource_name);
    let original_identity = manager
        .lock_file_identity(&fs::symlink_metadata(&original_path).unwrap())
        .unwrap();
    // Real same-Manager live descriptor remains locked. Replacing only its
    // evicted pathname must not create a second independently writable fence.
    fs::rename(
        &original_path,
        manager.directory.join(format!("{name}.held-original")),
    )
    .unwrap();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(options.open(&original_path).unwrap());
    let replacement_identity = manager
        .lock_file_identity(&fs::symlink_metadata(&original_path).unwrap())
        .unwrap();
    assert!(original_identity != replacement_identity);
    let refused = f
        .store
        .try_acquire_authority(std::slice::from_ref(&own))
        .err()
        .unwrap();
    let original_still_owned = f.store.authority_covers(&held, &scopes);
    let cache_unchanged = !manager
        .file_identities
        .read()
        .unwrap()
        .contains_key(&resource_name);
    drop(held);
    assert_eq!(refused.code, "authority-unavailable");
    assert!(original_still_owned && cache_unchanged);
}
