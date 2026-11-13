//! Actual Start journal admission: unchanged current Read facts, exact CID cut.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use std::time::Duration;

#[test]
fn replacement_start_keeps_identity_readers_and_records_with_exact_current_read() {
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
    assert!(!f.store.authority_covers(
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
fn replacement_start_coexists_with_conversation_reader_and_still_excludes_real_writer() {
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
    let proof = f.admit(&request);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(
        f.store
            .try_acquire_authority(&[AuthorityScope::conversation(
                &command.conversation_id,
                AuthorityMode::Write
            )
            .unwrap()])
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
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
    drop(reader);
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
    let current = proof.as_ref().is_ok_and(|(proof, read)| {
        proof.current()
            && read.current()
            && read.target().publication_suspended
            && proof.expires_at() <= origin.expires_at()
            && read.expires_at() <= origin.expires_at()
    });
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
    let (proof, read) = store
        .resume_native_replacement_start_activation_authority(
            &origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
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

// Actual current Start proof, original journal attempt and real primary custody.
// No modeled authority or physical start callback is introduced here.
#[test]
fn dormant_start_projection_uses_query_only_observer_under_primary_sql_custody() {
    let (f, _attempt, origin, _) = submitting_start();
    let (proof, original) = f
        .store
        .resume_native_replacement_start_activation_authority(
            &origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let expected_connection = original.target().connection.connection_id.clone();
    let expected_generation = original.target().connection.generation;
    let expected_expiry = original.expires_at();
    drop(original);
    let journal_before = f.count("journal");
    let steps_before = f.count("native_replacement_steps");
    let primary = f.store.connection().unwrap();
    // Keep primary SQL held throughout this REAL projection. Old code refuses
    // its first try_connection; the fixed query-only observer must not need it.
    let projected = f.store.native_replacement_start_read_under(&origin, &proof);
    let current = projected.as_ref().is_ok_and(|read| {
        read.current()
            && read.target().connection.connection_id == expected_connection
            && read.target().connection.generation == expected_generation
            && read.target().publication_suspended
            && read.expires_at() <= expected_expiry
            && read.expires_at() <= origin.expires_at()
    });
    let error = projected
        .as_ref()
        .err()
        .map(|e| (e.code.clone(), e.message.clone()));
    drop(projected);
    drop(primary);
    drop(proof);
    let unchanged =
        f.count("journal") == journal_before && f.count("native_replacement_steps") == steps_before;
    let writer = f.controlled_conversation_writer(0);
    let writer_progress = f.store.connection_with_authority(&writer).is_ok();
    drop(writer);
    println!("dormant_start_projection current={current} unchanged={unchanged} writer_progress={writer_progress} error={error:?}");
    assert!(
        unchanged && writer_progress,
        "original projection cleanup/attempt identity failed"
    );
    assert!(
        current,
        "dormant Start projection must use its current proof independently of primary SQL custody"
    );
}

#[test]
fn dormant_start_projection_observer_refusal_retains_no_extra_reader_or_effect() {
    let (f, _attempt, origin, _) = submitting_start();
    let (proof, original) = f
        .store
        .resume_native_replacement_start_activation_authority(
            &origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    drop(original);
    let journal_before = f.count("journal");
    let steps_before = f.count("native_replacement_steps");
    let observation = f.store.packet_connections.observe().unwrap();
    let observers: Vec<_> = (0..crate::packet_observation::OBSERVERS)
        .map(|_| f.store.try_packet_connection(&observation).unwrap())
        .collect();
    let refused = f.store.native_replacement_start_read_under(&origin, &proof);
    let code = refused.as_ref().err().map(|e| e.code.clone());
    drop(refused);
    drop(observers);
    let recovered = f.store.native_replacement_start_read_under(&origin, &proof);
    let current = recovered.as_ref().is_ok_and(|read| read.current());
    drop(recovered);
    drop(proof);
    let writer = f.controlled_conversation_writer(0);
    let writer_progress = f.store.connection_with_authority(&writer).is_ok();
    drop(writer);
    let unchanged =
        f.count("journal") == journal_before && f.count("native_replacement_steps") == steps_before;
    assert_eq!(code.as_deref(), Some("authority-busy"));
    assert!(current && writer_progress && unchanged);
}

#[test]
fn dormant_start_projection_rejects_foreign_proof_without_fallback_or_new_effect() {
    let (f, _attempt, origin, _) = submitting_start();
    let (other, _other_attempt, other_origin, _) = submitting_start();
    let (foreign, other_read) = other
        .store
        .resume_native_replacement_start_activation_authority(
            &other_origin,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    drop(other_read);
    let journal_before = f.count("journal");
    let steps_before = f.count("native_replacement_steps");
    let result = f
        .store
        .native_replacement_start_read_under(&origin, &foreign);
    let code = result.as_ref().err().map(|e| e.code.clone());
    drop(result);
    drop(foreign);
    assert_eq!(code.as_deref(), Some("forbidden"));
    assert_eq!(f.count("journal"), journal_before);
    assert_eq!(f.count("native_replacement_steps"), steps_before);
}
