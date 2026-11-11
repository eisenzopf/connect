//! Actual Start journal admission: unchanged current Read facts, exact CID cut.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use std::time::Duration;

#[test]
fn replacement_start_keeps_identity_readers_and_records_with_exact_conversation_writer() {
    let (f, _, command, allocated, request) = fixture();
    let identities = f
        .keys
        .iter()
        .map(|key| f.store.revalidate(key).unwrap().identity_id)
        .collect::<Vec<_>>();
    let readers = f
        .store
        .try_acquire_authority(
            &identities
                .iter()
                .map(|id| AuthorityScope::identity(id, AuthorityMode::Read).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let proof = f.admit(&request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap()]
    ));
    for id in &identities {
        assert!(!f.store.authority_covers(
            Store::peer_scopes(&proof),
            &[AuthorityScope::identity(id, AuthorityMode::Write).unwrap()]
        ));
    }
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let captured = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    let record = f
        .store
        .record_native_replacement_started_with_peer(&captured, &proof)
        .unwrap();
    assert_eq!(record.effects.len(), 3);
    assert!(record
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed));
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_started_with_peer(&captured, &proof)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    drop(proof);
    // Explicit peer mutation admission still excludes those same Identity readers.
    assert_eq!(
        f.store
            .admit_peer_authority_mode(
                &request,
                &AuthorityWait::new(Duration::from_millis(20)),
                true
            )
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(readers);
}

#[test]
fn replacement_start_same_conversation_reader_blocks_without_an_attempt_then_original_succeeds() {
    let (f, _, command, allocated, request) = fixture();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let reader = f
        .store
        .try_acquire_authority(&[AuthorityScope::conversation(
            &command.conversation_id,
            AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let refused = f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)));
    assert_eq!(refused.err().unwrap().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
    drop(reader);
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (_, read, _) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    drop(read);
    let repeat = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &proof,
                repeat
            )
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("native_replacement_steps"), 3);
}

#[test]
fn replacement_start_rechecks_current_credential_and_original_root_binding() {
    let (f, _, _, _, request) = fixture();
    let mut foreign = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut foreign.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["operation_record_id"] = json!("missing-original-root");
    assert!(f
        .store
        .admit_peer_authority(&foreign, &AuthorityWait::new(Duration::from_millis(20)))
        .is_err());
    let proof = f.admit(&request);
    drop(proof);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &request.credentials[0]).unwrap();
    assert!(f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)))
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
}

fn submitting_start() -> (
    Fixture,
    crate::NativeReplacementStartAttempt,
    Arc<crate::NativeReplacementStartOrigin>,
    Arc<crate::NativeMediaOrigin>,
) {
    let (f, _, _, allocated, request, sources) = fixture_scene(&[]);
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, origin) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    drop(read);
    drop(proof);
    (f, attempt, origin, sources[0].clone())
}

#[test]
fn dormant_start_activation_preserves_queued_old_source_epoch() {
    use super::scene_snapshot::{source_admitted, source_snapshot};
    let (f, _attempt, origin, source) = submitting_start();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let snapshot = source_snapshot(&store, &source);
    let io = source_admitted(&snapshot);
    let epoch = io.packet_epoch().unwrap();
    let stamp = io.snapshot_stamp().unwrap();
    drop(io);
    let proof = store.resume_native_replacement_start_activation_authority(
        &origin,
        &AuthorityWait::new(Duration::from_secs(1)),
    );
    let current = proof
        .as_ref()
        .is_ok_and(|proof| proof.current() && proof.expires_at() <= origin.expires_at());
    let preserved = epoch.current() && snapshot.admit_original(stamp).is_ok();
    drop(proof);
    drop(snapshot);
    watcher.shutdown().unwrap();
    assert!(
        current,
        "exact dormant activation must recapture its original current scope"
    );
    assert!(
        preserved,
        "dormant initial muted activation admission revoked the original queued old-source epoch"
    );
}

#[test]
fn dormant_start_activation_keeps_live_io_and_drains_real_revocation_before_uncertain_capture() {
    use super::scene_snapshot::{source_admitted, source_snapshot};
    use std::time::Instant;
    let (f, attempt, origin, source) = submitting_start();
    let source_credential = f.keys[0].clone();
    let admin = f.store.revalidate(&f.admin).unwrap();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let snapshot = source_snapshot(&store, &source);
    let io = source_admitted(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let proof = store
        .resume_native_replacement_start_activation_authority(
            &origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let read = store
        .native_replacement_start_read_under(&origin, &proof)
        .unwrap();
    assert!(read.current() && io.current() && snapshot.admit_original(stamp).is_ok());
    assert!(read.target().publication_suspended && read.expires_at() <= origin.expires_at());
    let (pending, waits_for_io, result) = std::thread::scope(|threads| {
        let writer = threads.spawn(|| store.revoke(&admin, &source_credential));
        let until = Instant::now() + Duration::from_secs(1);
        while epoch.current() && Instant::now() < until {
            std::thread::yield_now();
        }
        let pending = !epoch.current() && !writer.is_finished();
        drop(read);
        drop(proof);
        let waits_for_io = !writer.is_finished();
        drop(io);
        (pending, waits_for_io, writer.join().unwrap())
    });
    let original_refused = snapshot.admit_original(stamp).is_err() && !epoch.current();
    let resume_refused = store
        .resume_native_replacement_start_activation_authority(
            &origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .is_err();
    drop(snapshot);
    watcher.shutdown().unwrap();
    result.unwrap();
    assert!(
        pending && waits_for_io,
        "the real revocation must drain existing source IO and activation readers"
    );
    assert!(
        original_refused && resume_refused,
        "revoked source cannot resume initial activation or its old queued epoch"
    );
    let uncertain = attempt.uncertain();
    store
        .native_replacement_start_uncertain(&uncertain)
        .unwrap();
    let row = || {
        store.connection().unwrap().query_row(
        "SELECT o.state,o.revision,e.state,e.attempt,s.observation_json FROM operations o JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='start' JOIN operation_effects e ON e.id=s.effect_id WHERE o.record_id=?1",
        [origin.operation_record_id()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,r.get::<_,Option<String>>(4)?)),
    ).unwrap()
    };
    let captured = row();
    assert_eq!(
        (
            &*captured.0,
            &*captured.2,
            captured.3,
            captured.4.as_deref()
        ),
        ("unknown", "unknown", 1, None)
    );
    store
        .native_replacement_start_uncertain(&uncertain)
        .unwrap();
    assert_eq!(row(), captured);
}
